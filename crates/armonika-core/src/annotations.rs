//! What a user does with items: favorites, ratings and plays.
//!
//! Annotations belong to the user and to the item, not to the library access: when a
//! user loses a library the rows stay, the queries of [`crate::tracks`] stop showing
//! the items, and they come back with the access. Artist annotations hang on the
//! artist, so a track relink or merge never touches them.

use rusqlite::{Connection, params};

use crate::db::now_ms;
use crate::ids::{ItemRef, TrackId, UserId, resolve_album, resolve_artist, resolve_track};
use crate::tracks::{
    AlbumView, ArtistView, Scope, TrackView, load_tracks, query_albums, query_artists,
};
use crate::users::require_user;
use crate::{Error, Result};

/// An item resolved to its internal key, merged identifiers followed.
#[derive(Clone, Copy)]
enum Key {
    Track(i64),
    Album(i64),
    Artist(i64),
}

impl Key {
    /// The annotation table, its item column and the internal key.
    fn target(self) -> (&'static str, &'static str, i64) {
        match self {
            Key::Track(id) => ("track_annotations", "track_id", id),
            Key::Album(id) => ("album_annotations", "album_id", id),
            Key::Artist(id) => ("artist_annotations", "artist_id", id),
        }
    }
}

fn resolve_item(conn: &Connection, item: &ItemRef) -> Result<Key> {
    match item {
        ItemRef::Track(id) => resolve_track(conn, id)?
            .map(Key::Track)
            .ok_or_else(|| Error::TrackNotFound(id.clone())),
        ItemRef::Album(id) => resolve_album(conn, id)?
            .map(Key::Album)
            .ok_or_else(|| Error::AlbumNotFound(id.clone())),
        ItemRef::Artist(id) => resolve_artist(conn, id)?
            .map(Key::Artist)
            .ok_or_else(|| Error::ArtistNotFound(id.clone())),
    }
}

fn resolve_all(conn: &Connection, items: &[ItemRef]) -> Result<Vec<Key>> {
    items.iter().map(|i| resolve_item(conn, i)).collect()
}

/// Sets `column` of the annotation row of `user` and `key` to `value`, creating the row.
fn put(conn: &Connection, user: i64, key: Key, column: &str, value: Option<i64>) -> Result<()> {
    let (table, id_column, id) = key.target();
    conn.prepare_cached(&format!(
        "INSERT INTO {table} (user_id, {id_column}, {column}) VALUES (?1, ?2, ?3)
         ON CONFLICT (user_id, {id_column}) DO UPDATE SET {column} = excluded.{column}"
    ))?
    .execute(params![user, id, value])?;
    Ok(())
}

/// Stars `items` for `user`. An item already starred keeps its date. Nothing is written
/// if one of the items does not exist.
pub fn star(conn: &Connection, user: &UserId, items: &[ItemRef]) -> Result<()> {
    let user = require_user(conn, user)?;
    let keys = resolve_all(conn, items)?;
    let at = now_ms();
    let tx = conn.unchecked_transaction()?;
    for key in keys {
        let (table, id_column, id) = key.target();
        tx.prepare_cached(&format!(
            "INSERT INTO {table} (user_id, {id_column}, starred_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (user_id, {id_column}) DO UPDATE SET
               starred_at = COALESCE({table}.starred_at, excluded.starred_at)"
        ))?
        .execute(params![user, id, at])?;
    }
    tx.commit()?;
    Ok(())
}

/// Removes the star of `user` from `items`. Nothing is written if one of the items does
/// not exist.
pub fn unstar(conn: &Connection, user: &UserId, items: &[ItemRef]) -> Result<()> {
    let user = require_user(conn, user)?;
    let keys = resolve_all(conn, items)?;
    let tx = conn.unchecked_transaction()?;
    for key in keys {
        put(&tx, user, key, "starred_at", None)?;
    }
    tx.commit()?;
    Ok(())
}

/// Rates `item` from 1 to 5 for `user`; 0 removes the rating. Anything above 5 counts
/// as 5.
pub fn set_rating(conn: &Connection, user: &UserId, item: &ItemRef, rating: u8) -> Result<()> {
    let user = require_user(conn, user)?;
    let key = resolve_item(conn, item)?;
    let value = (rating > 0).then_some(i64::from(rating.min(5)));
    put(conn, user, key, "rating", value)
}

/// Counts a play of `track` by `user` at `at_ms` (Unix milliseconds): the play count
/// goes up, the last play moves forward (never back) and the play goes in the history.
pub fn record_play(conn: &Connection, user: &UserId, track: &TrackId, at_ms: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    record_play_in(&tx, user, track, at_ms)?;
    tx.commit()?;
    Ok(())
}

/// [`record_play`] without a transaction of its own, for a caller that already has one
/// open and writes more rows (the scrobble outbox) that must commit with the play.
/// Called outside a transaction, the two statements are not atomic.
pub fn record_play_in(conn: &Connection, user: &UserId, track: &TrackId, at_ms: i64) -> Result<()> {
    let user = require_user(conn, user)?;
    let key = resolve_track(conn, track)?.ok_or_else(|| Error::TrackNotFound(track.clone()))?;
    conn.prepare_cached(
        "INSERT INTO track_annotations (user_id, track_id, play_count, last_played_at)
         VALUES (?1, ?2, 1, ?3)
         ON CONFLICT (user_id, track_id) DO UPDATE SET
           play_count = play_count + 1,
           last_played_at = MAX(COALESCE(last_played_at, excluded.last_played_at),
                                excluded.last_played_at)",
    )?
    .execute(params![user, key, at_ms])?;
    conn.prepare_cached(
        "INSERT INTO play_history (user_id, track_id, played_at) VALUES (?1, ?2, ?3)",
    )?
    .execute(params![user, key, at_ms])?;
    Ok(())
}

/// What the user of the scope starred, most recently starred first, limited to what the
/// scope can see.
pub fn starred(
    conn: &Connection,
    scope: &Scope<'_>,
) -> Result<(Vec<ArtistView>, Vec<AlbumView>, Vec<TrackView>)> {
    let mut artists = query_artists(conn, scope, "AND an.starred_at IS NOT NULL")?;
    artists.sort_by(|a, b| {
        b.annotations
            .starred_at
            .cmp(&a.annotations.starred_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let albums = query_albums(
        conn,
        scope,
        "AND aa.starred_at IS NOT NULL",
        "",
        "ORDER BY MAX(aa.starred_at) DESC, al.id",
        &[],
    )?;
    let tracks = load_tracks(
        conn,
        scope,
        "AND ta.starred_at IS NOT NULL ORDER BY ta.starred_at DESC, t.id",
        &[],
    )?;
    Ok((artists, albums, tracks))
}
