//! `search3` and `search2`: the same data, under `searchResult3` and `searchResult2`.
//!
//! A query with words goes to the search index, with the popularity of the user as a
//! boost on the candidates. An empty query (nothing, only spaces, or `""` as clients
//! such as Symfonium send to ask for everything) is not a search: it pages through the
//! whole visible library in the order of the identifiers, so a client can mirror it.

use std::sync::Arc;

use armonika_core::access::Access;
use armonika_core::ids::{ItemRef, resolve_user};
use armonika_core::tracks::{self, Scope};
use armonika_search::{GroupLimits, SearchHits, SearchQuery};
use serde_json::{Value, json};

use crate::browse::{album_json, artist_json, request_libraries, track_json};
use crate::error::ApiError;
use crate::response::Payload;
use crate::{AppState, Params};

/// Items of one kind a page holds when the count is not given.
const DEFAULT_COUNT: usize = 20;

/// Most items of one kind a page holds.
const MAX_COUNT: usize = 500;

/// Largest offset taken as given: more is clamped, so it cannot overflow a sum or a
/// SQL `OFFSET`.
const MAX_OFFSET: usize = 1_000_000_000;

/// A count or offset parameter: `default` if absent, at most `max`. A number too large
/// for the type counts as `max`; anything else that is not a number is an error.
fn bounded(params: &Params, name: &str, default: usize, max: usize) -> Result<usize, ApiError> {
    let Some(raw) = params.get(name) else {
        return Ok(default);
    };
    let raw = raw.trim();
    match raw.parse::<usize>() {
        Ok(n) => Ok(n.min(max)),
        Err(_) if !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()) => Ok(max),
        Err(_) => Err(ApiError::generic(format!("invalid {name}"))),
    }
}

fn limits(
    params: &Params,
    suffix: &str,
    default: usize,
    max: usize,
) -> Result<GroupLimits, ApiError> {
    Ok(GroupLimits {
        artists: bounded(params, &format!("artist{suffix}"), default, max)?,
        albums: bounded(params, &format!("album{suffix}"), default, max)?,
        tracks: bounded(params, &format!("song{suffix}"), default, max)?,
    })
}

/// The text to search for: `""` and only spaces are empty.
fn query_text(params: &Params) -> &str {
    let text = params.get("query").unwrap_or_default().trim();
    if text == "\"\"" { "" } else { text }
}

pub async fn search(
    state: &AppState,
    access: &Access,
    params: &Params,
    node: &'static str,
) -> Result<Payload, ApiError> {
    let counts = limits(params, "Count", DEFAULT_COUNT, MAX_COUNT)?;
    let offsets = limits(params, "Offset", 0, MAX_OFFSET)?;
    let libs = request_libraries(access, params)?;
    let user = access.user.clone();
    let articles = state.ignored_articles.clone();
    let text = query_text(params).to_owned();

    let found = if text.is_empty() {
        state
            .db
            .read(move |c| {
                let scope = Scope {
                    libs: &libs,
                    user: &user,
                    articles: &articles,
                };
                Ok((
                    tracks::all_artists_page(c, &scope, offsets.artists, counts.artists)?,
                    tracks::all_albums_page(c, &scope, offsets.albums, counts.albums)?,
                    tracks::all_tracks_page(c, &scope, offsets.tracks, counts.tracks)?,
                ))
            })
            .await?
    } else {
        let key = {
            let user = user.clone();
            state.db.read(move |c| resolve_user(c, &user)).await?
        };
        let hits = match key {
            Some(key) if !libs.is_empty() => {
                let db = state.db.clone();
                let index = state.search.clone();
                let query = SearchQuery {
                    text,
                    libraries: libs.clone(),
                    limits: counts,
                    offsets,
                    // Looked up for each candidate the index weighs, not for the library.
                    boost: Some(Arc::new(move |id: &ItemRef| {
                        db.read_blocking(|c| tracks::popularity(c, key, id))
                            .unwrap_or(0.0)
                    })),
                };
                tokio::task::spawn_blocking(move || index.search(&query))
                    .await
                    .map_err(armonika_core::Error::from)??
            }
            _ => SearchHits::default(),
        };
        state
            .db
            .read(move |c| {
                let scope = Scope {
                    libs: &libs,
                    user: &user,
                    articles: &articles,
                };
                Ok((
                    tracks::artists_by_ids(c, &scope, &hits.artists)?,
                    tracks::albums_by_ids(c, &scope, &hits.albums)?,
                    tracks::tracks_by_ids(c, &scope, &hits.tracks)?,
                ))
            })
            .await?
    };

    let (artists, albums, songs) = found;
    let list = |items: Vec<Value>| Value::Array(items);
    Ok(Payload(
        node,
        json!({
            "artist": list(artists.iter().map(artist_json).collect()),
            "album": list(albums.iter().map(album_json).collect()),
            "song": list(songs.iter().map(track_json).collect()),
        }),
    ))
}
