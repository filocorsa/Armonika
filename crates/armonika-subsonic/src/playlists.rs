//! Playlists and the saved play queue: `getPlaylists`, `getPlaylist`, `createPlaylist`,
//! `updatePlaylist`, `deletePlaylist`, `savePlayQueue` and `getPlayQueue`.
//!
//! A track the caller cannot see answers 70, exactly like one that does not exist, and
//! nothing is written: the check runs in the writer slot before the first write. Changing
//! or deleting a public playlist of another user answers 50; a private one of another
//! user does not exist for the caller (70).

use armonika_core::Error;
use armonika_core::access::Access;
use armonika_core::db::Connection;
use armonika_core::ids::{PlaylistId, TrackId};
use armonika_core::play_queue;
use armonika_core::playlists::{self, PlaylistEdit, PlaylistView};
use armonika_core::tracks::{self, Scope, TrackView};
use serde_json::{Map, Value, json};

use crate::browse::{insert, insert_opt, iso8601, seconds, track_json};
use crate::error::ApiError;
use crate::response::Payload;
use crate::{AppState, Params};

fn playlist_json(p: &PlaylistView) -> Map<String, Value> {
    let mut m = Map::new();
    insert(&mut m, "id", p.id.as_str());
    insert(&mut m, "name", p.name.as_str());
    insert_opt(&mut m, "comment", p.comment.as_deref());
    insert(&mut m, "owner", p.owner_name.as_str());
    insert(&mut m, "public", p.public);
    insert(&mut m, "songCount", p.song_count);
    insert(&mut m, "duration", seconds(p.duration_ms));
    insert(&mut m, "created", iso8601(p.created_at));
    insert(&mut m, "changed", iso8601(p.updated_at));
    m
}

fn with_entries(p: &PlaylistView, entries: &[TrackView]) -> Value {
    let mut m = playlist_json(p);
    insert(
        &mut m,
        "entry",
        entries.iter().map(track_json).collect::<Vec<_>>(),
    );
    Value::Object(m)
}

fn track_ids(params: &Params, name: &str) -> Result<Vec<TrackId>, ApiError> {
    params
        .all(name)
        .iter()
        .map(|raw| raw.parse().map_err(|_| ApiError::not_found()))
        .collect()
}

fn playlist_id(raw: &str) -> Result<PlaylistId, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

/// Fails with the error behind 70 unless the caller can see every track.
fn require_visible(
    conn: &Connection,
    access: &Access,
    tracks: &[TrackId],
) -> armonika_core::Result<()> {
    let libs = access.visible(None);
    let scope = Scope {
        libs: &libs,
        user: &access.user,
        articles: &[],
    };
    for id in tracks {
        if tracks::track(conn, &scope, id)?.is_none() {
            return Err(Error::TrackNotFound(id.clone()));
        }
    }
    Ok(())
}

pub async fn get_playlists(state: &AppState, access: &Access) -> Result<Payload, ApiError> {
    let viewer = access.clone();
    let list = state.db.read(move |c| playlists::list(c, &viewer)).await?;
    Ok(Payload(
        "playlists",
        json!({ "playlist": list.iter().map(|p| Value::Object(playlist_json(p))).collect::<Vec<_>>() }),
    ))
}

pub async fn get_playlist(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let id = playlist_id(params.require("id")?)?;
    let viewer = access.clone();
    let found = state
        .db
        .read(move |c| playlists::get(c, &viewer, &id))
        .await?;
    let (view, entries) = found.ok_or_else(ApiError::not_found)?;
    Ok(Payload("playlist", with_entries(&view, &entries)))
}

pub async fn create_playlist(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let existing = params.get("playlistId").map(playlist_id).transpose()?;
    let name = match (&existing, params.get("name")) {
        (None, None) => return Err(ApiError::missing_parameter("name")),
        (_, name) => name.map(str::to_owned),
    };
    let songs = track_ids(params, "songId")?;
    let editor = access.clone();
    let (view, entries) = state
        .db
        .write(move |c| {
            require_visible(c, &editor, &songs)?;
            let id = match existing {
                Some(id) => {
                    // The whole list is replaced; the name changes only if one is given.
                    playlists::replace_visible(c, &editor, &id, &songs)?;
                    if name.is_some() {
                        let edit = PlaylistEdit {
                            name,
                            ..PlaylistEdit::default()
                        };
                        playlists::update(c, &editor, &id, edit)?;
                    }
                    id
                }
                None => playlists::create(c, &editor.user, name.as_deref().unwrap_or(""), &songs)?,
            };
            playlists::get(c, &editor, &id)?.ok_or(Error::PlaylistNotFound(id))
        })
        .await?;
    Ok(Payload("playlist", with_entries(&view, &entries)))
}

pub async fn update_playlist(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let id = playlist_id(params.require("playlistId")?)?;
    let add = track_ids(params, "songIdToAdd")?;
    let remove_visible_indexes = params
        .all("songIndexToRemove")
        .iter()
        .map(|raw| {
            raw.trim()
                .parse::<usize>()
                .map_err(|_| ApiError::generic("invalid songIndexToRemove"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let public = match params.get("public").map(|p| p.trim().to_ascii_lowercase()) {
        None => None,
        Some(p) if p == "true" => Some(true),
        Some(p) if p == "false" => Some(false),
        Some(_) => return Err(ApiError::generic("invalid public")),
    };
    let edit = PlaylistEdit {
        name: params.get("name").map(str::to_owned),
        comment: params.get("comment").map(str::to_owned),
        public,
        add,
        remove_visible_indexes,
    };
    let editor = access.clone();
    state
        .db
        .write(move |c| {
            require_visible(c, &editor, &edit.add)?;
            playlists::update(c, &editor, &id, edit)
        })
        .await?;
    Ok(Payload::empty())
}

pub async fn delete_playlist(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let id = playlist_id(params.require("id")?)?;
    let editor = access.clone();
    state
        .db
        .write(move |c| playlists::delete(c, &editor, &id))
        .await?;
    Ok(Payload::empty())
}

pub async fn save_play_queue(
    state: &AppState,
    access: &Access,
    params: &Params,
) -> Result<Payload, ApiError> {
    let songs = track_ids(params, "id")?;
    // `current` is a track id; with repeats the first occurrence stands for it.
    let current = match params.get("current") {
        Some(raw) => {
            let id: TrackId = raw.parse().map_err(|_| ApiError::not_found())?;
            songs.iter().position(|s| *s == id)
        }
        None => None,
    };
    let position_ms: i64 = params
        .get("position")
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(0);
    let client = params.get("c").unwrap_or("unknown").to_owned();
    let editor = access.clone();
    state
        .db
        .write(move |c| {
            require_visible(c, &editor, &songs)?;
            play_queue::save(c, &editor.user, &songs, current, position_ms, &client)
        })
        .await?;
    Ok(Payload::empty())
}

pub async fn get_play_queue(state: &AppState, access: &Access) -> Result<Payload, ApiError> {
    let viewer = access.clone();
    let queue = state.db.read(move |c| play_queue::load(c, &viewer)).await?;
    let Some(queue) = queue else {
        return Ok(Payload("playQueue", Value::Null));
    };
    let mut m = Map::new();
    let current = queue.current.and_then(|i| queue.tracks.get(i));
    insert_opt(&mut m, "current", current.map(|t| t.id.as_str()));
    insert(&mut m, "position", queue.position_ms);
    insert(&mut m, "username", access.name.as_str());
    insert(&mut m, "changed", iso8601(queue.changed_at));
    insert(&mut m, "changedBy", queue.changed_by.as_str());
    insert(
        &mut m,
        "entry",
        queue.tracks.iter().map(track_json).collect::<Vec<_>>(),
    );
    Ok(Payload("playQueue", Value::Object(m)))
}
