//! Favorites, ratings, plays and what is playing: `star`, `unstar`, `setRating`,
//! `scrobble`, `getStarred2`, `getStarred` and `getNowPlaying`.
//!
//! An id the caller cannot see answers 70, exactly like one that does not exist, and
//! nothing is written: the check and the write run in one writer transaction slot, so a
//! list with one bad id changes nothing.

use armonika_core::access::Access;
use armonika_core::annotations;
use armonika_core::db::{Connection, now_ms};
use armonika_core::ids::{ItemRef, TrackId};
use armonika_core::play_queue;
use armonika_core::tracks::{self, Scope};
use serde_json::{Value, json};

use crate::browse::{album_json, artist_json, track_json};
use crate::error::ApiError;
use crate::response::Payload;
use crate::{AppState, Params};

/// Everything named by `id`, `albumId` and `artistId` (each may repeat).
fn items(params: &Params) -> Result<Vec<ItemRef>, ApiError> {
    let mut out = Vec::new();
    for name in ["id", "albumId", "artistId"] {
        for raw in params.all(name) {
            out.push(raw.parse().map_err(|_| ApiError::not_found())?);
        }
    }
    if out.is_empty() {
        return Err(ApiError::missing_parameter("id"));
    }
    Ok(out)
}

/// Whether the scope can see `item`.
fn visible(conn: &Connection, scope: &Scope<'_>, item: &ItemRef) -> armonika_core::Result<bool> {
    Ok(match item {
        ItemRef::Track(id) => tracks::track(conn, scope, id)?.is_some(),
        ItemRef::Album(id) => tracks::album(conn, scope, id)?.is_some(),
        ItemRef::Artist(id) => tracks::artist(conn, scope, id)?.is_some(),
    })
}

/// Runs `write` on the writer connection if the caller can see every item, and answers
/// 70 without running it otherwise.
async fn write_visible(
    state: &AppState,
    access: &Access,
    items: Vec<ItemRef>,
    write: impl FnOnce(&Connection) -> armonika_core::Result<()> + Send + 'static,
) -> Result<Payload, ApiError> {
    let libs = access.visible(None);
    let user = access.user.clone();
    let articles = state.ignored_articles.clone();
    let done = state
        .db
        .write(move |c| {
            let scope = Scope {
                libs: &libs,
                user: &user,
                articles: &articles,
            };
            for item in &items {
                if !visible(c, &scope, item)? {
                    return Ok(false);
                }
            }
            write(c)?;
            Ok(true)
        })
        .await?;
    if done {
        Ok(Payload::empty())
    } else {
        Err(ApiError::not_found())
    }
}

pub async fn star(state: &AppState, access: &Access, params: &Params) -> Result<Payload, ApiError> {
    let items = items(params)?;
    let user = access.user.clone();
    let list = items.clone();
    write_visible(state, access, items, move |c| {
        annotations::star(c, &user, &list)
    })
    .await
}

pub async fn unstar(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let items = items(params)?;
    let user = access.user.clone();
    let list = items.clone();
    write_visible(state, access, items, move |c| {
        annotations::unstar(c, &user, &list)
    })
    .await
}

pub async fn set_rating(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let item: ItemRef = params
        .require("id")?
        .parse()
        .map_err(|_| ApiError::not_found())?;
    let rating: u8 = params
        .require("rating")?
        .trim()
        .parse()
        .ok()
        .filter(|r| *r <= 5)
        .ok_or_else(|| ApiError::generic("rating must be from 0 to 5"))?;
    let user = access.user.clone();
    let target = item.clone();
    write_visible(state, access, vec![item], move |c| {
        annotations::set_rating(c, &user, &target, rating)
    })
    .await
}

pub async fn scrobble(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let ids = params.all("id");
    if ids.is_empty() {
        return Err(ApiError::missing_parameter("id"));
    }
    let tracks: Vec<TrackId> = ids
        .iter()
        .map(|raw| raw.parse().map_err(|_| ApiError::not_found()))
        .collect::<Result<_, _>>()?;
    // The time of each play, by position; a play without one happened now, and none
    // happens after now.
    let now = now_ms();
    let times: Vec<i64> = (0..tracks.len())
        .map(|i| {
            params
                .all("time")
                .get(i)
                .and_then(|t| t.trim().parse::<i64>().ok())
                .map_or(now, |t| t.min(now))
        })
        .collect();
    let submission = !params
        .get("submission")
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("false"));
    let client = params.get("c").unwrap_or("unknown").to_owned();
    let user = access.user.clone();
    let items = tracks.iter().cloned().map(ItemRef::Track).collect();
    // All the plays of the request go in one transaction, together with the outbox
    // rows of the linked scrobbling services.
    let sink = state.scrobbles.clone();
    let (play_user, plays) = (user.clone(), tracks.clone());
    let response = write_visible(state, access, items, move |c| {
        if !submission {
            for track in &plays {
                play_queue::set_now_playing(c, &play_user, &client, track)?;
            }
            return Ok(());
        }
        let tx = c.unchecked_transaction()?;
        for (track, time) in plays.iter().zip(&times) {
            annotations::record_play_in(&tx, &play_user, track, *time)?;
            sink.submitted(&tx, &play_user, track, *time)?;
        }
        tx.commit()?;
        Ok(())
    })
    .await?;
    if !submission {
        for track in tracks {
            // Best effort and off the request: a slow service never slows the player.
            let sink = state.scrobbles.clone();
            let user = user.clone();
            tokio::spawn(async move { sink.now_playing(&user, &track).await });
        }
    }
    Ok(response)
}

pub async fn get_starred(
    state: &AppState,
    access: &Access,
    node: &'static str,
) -> Result<Payload, ApiError> {
    let libs = access.visible(None);
    let user = access.user.clone();
    let articles = state.ignored_articles.clone();
    let (artists, albums, songs) = state
        .db
        .read(move |c| {
            annotations::starred(
                c,
                &Scope {
                    libs: &libs,
                    user: &user,
                    articles: &articles,
                },
            )
        })
        .await?;
    Ok(Payload(
        node,
        json!({
            "artist": artists.iter().map(artist_json).collect::<Vec<_>>(),
            "album": albums.iter().map(album_json).collect::<Vec<_>>(),
            "song": songs.iter().map(track_json).collect::<Vec<_>>(),
        }),
    ))
}

pub async fn get_now_playing(state: &AppState, access: &Access) -> Result<Payload, ApiError> {
    let viewer = access.clone();
    let now = now_ms();
    let entries = state
        .db
        .read(move |c| play_queue::now_playing_at(c, &viewer, now))
        .await?;
    let list: Vec<Value> = entries
        .iter()
        .map(|e| {
            let mut v = track_json(&e.track);
            if let Value::Object(m) = &mut v {
                m.insert("username".to_owned(), e.user_name.as_str().into());
                m.insert(
                    "minutesAgo".to_owned(),
                    (now.saturating_sub(e.started_at).max(0) / 60_000).into(),
                );
                m.insert("playerId".to_owned(), 0.into());
                m.insert("playerName".to_owned(), e.client.as_str().into());
            }
            v
        })
        .collect();
    Ok(Payload("nowPlaying", json!({ "entry": list })))
}
