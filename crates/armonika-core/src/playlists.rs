//! Playlists of users.
//!
//! A playlist belongs to one user and is private unless made public; others see a public
//! one read-only. It may hold the same track more than once.
//!
//! Entries a caller cannot see are hidden, not deleted: a track that is not `present`, or
//! that belongs to a library the caller cannot reach, stays in the list and shows up again
//! when it comes back. Every index in this API counts the visible entries only, and a
//! change made through the visible list keeps the hidden entries where they were.
//!
//! The stored `position` of an entry only orders the list. A purge can leave gaps in it
//! (the entry of a deleted track goes by cascade), so nothing here assumes the positions
//! are contiguous: lists are read in position order and every write numbers them again
//! from 0.

use std::collections::{HashMap, VecDeque};

use rusqlite::{Connection, params};

use crate::access::Access;
use crate::db::now_ms;
use crate::ids::{PlaylistId, TrackId, UserId, resolve_playlist, resolve_track};
use crate::tracks::{self, Scope, TrackView};
use crate::users::require_user;
use crate::{Error, Result};

/// Tracks looked up in one statement.
const LOOKUP_CHUNK: usize = 500;

/// A playlist as its viewer sees it: the counts cover the visible entries only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistView {
    pub id: PlaylistId,
    pub owner: UserId,
    /// Login name of the owner.
    pub owner_name: String,
    pub name: String,
    pub comment: Option<String>,
    pub public: bool,
    /// Visible entries.
    pub song_count: u32,
    /// Total duration of the visible entries, in milliseconds.
    pub duration_ms: u64,
    /// Unix milliseconds.
    pub created_at: i64,
    /// Unix milliseconds.
    pub updated_at: i64,
}

/// A change to a playlist; whatever is not set stays as it is.
#[derive(Clone, Debug, Default)]
pub struct PlaylistEdit {
    pub name: Option<String>,
    /// An empty comment clears it.
    pub comment: Option<String>,
    pub public: Option<bool>,
    /// Tracks added at the end of the full list, after the removals.
    pub add: Vec<TrackId>,
    /// Indexes in the visible list (as it was before this edit) to remove. Indexes past
    /// the end are ignored.
    pub remove_visible_indexes: Vec<usize>,
}

/// Runs `f` in a transaction of its own unless the caller already opened one.
fn in_transaction<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let tx = conn
        .is_autocommit()
        .then(|| conn.unchecked_transaction())
        .transpose()?;
    let out = f()?;
    if let Some(tx) = tx {
        tx.commit()?;
    }
    Ok(out)
}

fn track_keys(conn: &Connection, tracks: &[TrackId]) -> Result<Vec<i64>> {
    tracks
        .iter()
        .map(|t| resolve_track(conn, t)?.ok_or_else(|| Error::TrackNotFound(t.clone())))
        .collect()
}

/// The views of the distinct tracks of `ids` that `viewer` can see.
pub(crate) fn visible_views(
    conn: &Connection,
    viewer: &Access,
    ids: &[TrackId],
) -> Result<HashMap<TrackId, TrackView>> {
    let libs = viewer.visible(None);
    let scope = Scope {
        libs: &libs,
        user: &viewer.user,
        articles: &[],
    };
    let mut distinct: Vec<TrackId> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if seen.insert(id) {
            distinct.push(id.clone());
        }
    }
    let mut out = HashMap::new();
    for chunk in distinct.chunks(LOOKUP_CHUNK) {
        for view in tracks::tracks_by_ids(conn, &scope, chunk)? {
            out.insert(view.id.clone(), view);
        }
    }
    Ok(out)
}

/// A stored entry: the track key and the identifier it is visible under.
struct Stored {
    key: i64,
    id: TrackId,
}

fn stored_entries(conn: &Connection, playlist: i64) -> Result<Vec<Stored>> {
    Ok(conn
        .prepare_cached(
            "SELECT t.id, t.public_id FROM playlist_entries e
             JOIN tracks t ON t.id = e.track_id
             WHERE e.playlist_id = ?1 ORDER BY e.position",
        )?
        .query_map([playlist], |r| {
            Ok(Stored {
                key: r.get(0)?,
                id: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

/// The stored entries with whether `editor` sees each.
fn flagged_entries(conn: &Connection, editor: &Access, playlist: i64) -> Result<Vec<(i64, bool)>> {
    let stored = stored_entries(conn, playlist)?;
    let ids: Vec<TrackId> = stored.iter().map(|s| s.id.clone()).collect();
    let visible = visible_views(conn, editor, &ids)?;
    Ok(stored
        .into_iter()
        .map(|s| (s.key, visible.contains_key(&s.id)))
        .collect())
}

/// Replaces the entries of `playlist` by `keys`, numbered from 0, and stamps the change.
fn store_entries(conn: &Connection, playlist: i64, keys: &[i64]) -> Result<()> {
    conn.execute(
        "DELETE FROM playlist_entries WHERE playlist_id = ?1",
        [playlist],
    )?;
    let mut insert = conn.prepare_cached(
        "INSERT INTO playlist_entries (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
    )?;
    for (position, key) in keys.iter().enumerate() {
        insert.execute(params![
            playlist,
            i64::try_from(position).unwrap_or(i64::MAX),
            key
        ])?;
    }
    touch(conn, playlist)
}

fn touch(conn: &Connection, playlist: i64) -> Result<()> {
    conn.execute(
        "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
        params![playlist, now_ms()],
    )?;
    Ok(())
}

/// The key of a playlist `editor` may change. A playlist the editor cannot see (private
/// and someone else's) does not exist for them; a public one of another user is
/// [`Error::NotPlaylistOwner`].
fn owned(conn: &Connection, editor: &Access, id: &PlaylistId) -> Result<i64> {
    let editor_key = require_user(conn, &editor.user)?;
    let key = resolve_playlist(conn, id)?.ok_or_else(|| Error::PlaylistNotFound(id.clone()))?;
    let (owner, public): (i64, bool) = conn.query_row(
        "SELECT owner_id, public FROM playlists WHERE id = ?1",
        [key],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if owner == editor_key {
        Ok(key)
    } else if public {
        Err(Error::NotPlaylistOwner(id.clone()))
    } else {
        Err(Error::PlaylistNotFound(id.clone()))
    }
}

/// Creates a playlist of `owner` holding `tracks` in that order.
pub fn create(
    conn: &Connection,
    owner: &UserId,
    name: &str,
    tracks: &[TrackId],
) -> Result<PlaylistId> {
    let owner_key = require_user(conn, owner)?;
    let keys = track_keys(conn, tracks)?;
    in_transaction(conn, || {
        let id = PlaylistId::generate();
        let now = now_ms();
        conn.execute(
            "INSERT INTO playlists (public_id, owner_id, name, public, created_at, updated_at)
             VALUES (?1, ?2, ?3, 0, ?4, ?4)",
            params![id, owner_key, name, now],
        )?;
        store_entries(conn, conn.last_insert_rowid(), &keys)?;
        Ok(id)
    })
}

const VIEW_COLUMNS: &str = "SELECT p.id, p.public_id, u.public_id, u.name, p.name, p.comment,
       p.public, p.created_at, p.updated_at, {visible_count}, {visible_duration}
     FROM playlists p JOIN users u ON u.id = p.owner_id";

/// Views of the playlists `viewer` can see (own and public ones), filtered by `extra`
/// (a clause starting with `AND`, which may use `?2`).
fn views(
    conn: &Connection,
    viewer: &Access,
    extra: &str,
    bind: Option<i64>,
) -> Result<Vec<PlaylistView>> {
    let viewer_key = require_user(conn, &viewer.user)?;
    let libs = viewer
        .visible(None)
        .iter()
        .map(|l| l.0.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let over_visible = |select: &str| {
        format!(
            "(SELECT {select} FROM playlist_entries e JOIN tracks t ON t.id = e.track_id
              WHERE e.playlist_id = p.id AND t.state = 'present'
                AND t.library_id IN ({libs}))"
        )
    };
    let sql = format!(
        "{} WHERE (p.owner_id = ?1 OR p.public = 1) {extra}
         ORDER BY (p.owner_id = ?1) DESC, p.name COLLATE NOCASE, p.id",
        VIEW_COLUMNS
            .replace("{visible_count}", &over_visible("COUNT(*)"))
            .replace(
                "{visible_duration}",
                &over_visible("COALESCE(SUM(t.duration_ms), 0)")
            ),
    );
    let map = |r: &rusqlite::Row<'_>| -> rusqlite::Result<PlaylistView> {
        let count: i64 = r.get(9)?;
        let duration: i64 = r.get(10)?;
        Ok(PlaylistView {
            id: r.get(1)?,
            owner: r.get(2)?,
            owner_name: r.get(3)?,
            name: r.get(4)?,
            comment: r.get(5)?,
            public: r.get(6)?,
            created_at: r.get(7)?,
            updated_at: r.get(8)?,
            song_count: u32::try_from(count).unwrap_or(u32::MAX),
            duration_ms: duration.unsigned_abs(),
        })
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = match bind {
        Some(key) => stmt.query_map(params![viewer_key, key], map)?,
        None => stmt.query_map(params![viewer_key], map)?,
    };
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The playlists `viewer` owns, then the public ones of others, each group by name.
pub fn list(conn: &Connection, viewer: &Access) -> Result<Vec<PlaylistView>> {
    views(conn, viewer, "", None)
}

/// The playlist `id` with its entries `viewer` can see, in order. `None` if there is no
/// such playlist or it is private and not the viewer's.
pub fn get(
    conn: &Connection,
    viewer: &Access,
    id: &PlaylistId,
) -> Result<Option<(PlaylistView, Vec<TrackView>)>> {
    let Some(key) = resolve_playlist(conn, id)? else {
        return Ok(None);
    };
    let Some(view) = views(conn, viewer, "AND p.id = ?2", Some(key))?.pop() else {
        return Ok(None);
    };
    let stored = stored_entries(conn, key)?;
    let ids: Vec<TrackId> = stored.into_iter().map(|s| s.id).collect();
    let by_id = visible_views(conn, viewer, &ids)?;
    let entries = ids.iter().filter_map(|id| by_id.get(id).cloned()).collect();
    Ok(Some((view, entries)))
}

/// Applies `edit` to the playlist `id`. Only the owner may.
pub fn update(
    conn: &Connection,
    editor: &Access,
    id: &PlaylistId,
    edit: PlaylistEdit,
) -> Result<()> {
    let key = owned(conn, editor, id)?;
    let added = track_keys(conn, &edit.add)?;
    in_transaction(conn, || {
        if let Some(name) = &edit.name {
            conn.execute(
                "UPDATE playlists SET name = ?2 WHERE id = ?1",
                params![key, name],
            )?;
        }
        if let Some(comment) = &edit.comment {
            let comment = (!comment.is_empty()).then_some(comment);
            conn.execute(
                "UPDATE playlists SET comment = ?2 WHERE id = ?1",
                params![key, comment],
            )?;
        }
        if let Some(public) = edit.public {
            conn.execute(
                "UPDATE playlists SET public = ?2 WHERE id = ?1",
                params![key, public],
            )?;
        }
        if !edit.remove_visible_indexes.is_empty() || !added.is_empty() {
            let mut kept = Vec::new();
            let mut visible_index = 0;
            for (track, visible) in flagged_entries(conn, editor, key)? {
                if visible {
                    let removed = edit.remove_visible_indexes.contains(&visible_index);
                    visible_index += 1;
                    if removed {
                        continue;
                    }
                }
                kept.push(track);
            }
            kept.extend(added);
            store_entries(conn, key, &kept)?;
        }
        touch(conn, key)
    })
}

/// Makes the visible part of the playlist `id` equal to `new_visible`, keeping the
/// hidden entries (see [`merge_visible`] for where they go). Only the owner may.
pub fn replace_visible(
    conn: &Connection,
    editor: &Access,
    id: &PlaylistId,
    new_visible: &[TrackId],
) -> Result<()> {
    let key = owned(conn, editor, id)?;
    let new_keys = track_keys(conn, new_visible)?;
    in_transaction(conn, || {
        let old = flagged_entries(conn, editor, key)?;
        store_entries(conn, key, &merge_visible(&old, &new_keys))
    })
}

/// Deletes the playlist `id` with its entries. Only the owner may.
pub fn delete(conn: &Connection, editor: &Access, id: &PlaylistId) -> Result<()> {
    let key = owned(conn, editor, id)?;
    conn.execute("DELETE FROM playlists WHERE id = ?1", [key])?;
    Ok(())
}

/// The list after replacing the visible entries of `old` (track key, visible) by `new`.
///
/// 1. The old list splits into runs of consecutive hidden entries; each run is anchored
///    to the visible entry before it, or to the start.
/// 2. Old visible entries pair with new ones by (track, occurrence number of that track):
///    the second old occurrence of a track pairs with the second new one.
/// 3. The result is the run anchored to the start, then each new entry followed by the
///    run of the old entry it pairs with, if any.
/// 4. The run of an old entry that is not paired (it was removed) goes to the nearest
///    paired old entry before it in the old list, after that entry's own run, or to the
///    start after the leading run.
fn merge_visible(old: &[(i64, bool)], new: &[i64]) -> Vec<i64> {
    // runs[0] is the leading run, runs[i + 1] follows the i-th old visible entry.
    let mut visible_keys = Vec::new();
    let mut runs: Vec<Vec<i64>> = vec![Vec::new()];
    for &(key, visible) in old {
        if visible {
            visible_keys.push(key);
            runs.push(Vec::new());
        } else if let Some(run) = runs.last_mut() {
            run.push(key);
        }
    }
    let mut waiting: HashMap<i64, VecDeque<usize>> = HashMap::new();
    for (i, key) in visible_keys.iter().enumerate() {
        waiting.entry(*key).or_default().push_back(i);
    }
    let mut paired_with: Vec<Option<usize>> = Vec::with_capacity(new.len());
    let mut is_paired = vec![false; visible_keys.len()];
    for key in new {
        let old_index = waiting.get_mut(key).and_then(VecDeque::pop_front);
        if let Some(i) = old_index {
            is_paired[i] = true;
        }
        paired_with.push(old_index);
    }
    // carried[0] is the leading run, carried[i + 1] the runs that follow old entry i.
    let mut carried: Vec<Vec<i64>> = vec![Vec::new(); runs.len()];
    carried[0] = std::mem::take(&mut runs[0]);
    let mut anchor = 0;
    for (i, paired) in is_paired.iter().enumerate() {
        let run = std::mem::take(&mut runs[i + 1]);
        if *paired {
            anchor = i + 1;
            carried[anchor] = run;
        } else {
            carried[anchor].extend(run);
        }
    }
    let mut out = std::mem::take(&mut carried[0]);
    for (key, old_index) in new.iter().zip(&paired_with) {
        out.push(*key);
        if let Some(i) = old_index {
            out.append(&mut carried[i + 1]);
        }
    }
    out
}
