//! What users are listening to right now, and the play queue each user saves so another
//! player can pick it up.
//!
//! The saved queue keeps every track it was given, also those the user cannot see now (a
//! missing file, a library whose access was revoked); [`load`] leaves them out and works
//! out the current track on the visible list. Stored positions only order the entries and
//! may have gaps after a purge, so nothing relies on them being contiguous.

use rusqlite::{Connection, OptionalExtension, params};

use crate::access::Access;
use crate::db::now_ms;
use crate::ids::{TrackId, UserId, resolve_track};
use crate::playlists::visible_views;
use crate::tracks::{self, Scope, TrackView};
use crate::users::require_user;
use crate::{Error, Result};

/// Entries older than this are not shown: the player stopped without saying so.
pub const NOW_PLAYING_TTL_MS: i64 = 10 * 60 * 1000;

/// One player of one user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NowPlayingEntry {
    /// Login name of the listener.
    pub user_name: String,
    /// Name of the player app, as it sent it.
    pub client: String,
    /// When the track started, Unix milliseconds.
    pub started_at: i64,
    /// The track, with the annotations of the viewer.
    pub track: TrackView,
}

/// Records that `user` is playing `track` on `client` now, replacing what that client
/// was playing.
pub fn set_now_playing(
    conn: &Connection,
    user: &UserId,
    client: &str,
    track: &TrackId,
) -> Result<()> {
    let user = require_user(conn, user)?;
    let key = resolve_track(conn, track)?.ok_or_else(|| Error::TrackNotFound(track.clone()))?;
    conn.prepare_cached(
        "INSERT INTO now_playing (user_id, client, track_id, started_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (user_id, client) DO UPDATE SET
           track_id = excluded.track_id, started_at = excluded.started_at",
    )?
    .execute(params![user, client, key, now_ms()])?;
    Ok(())
}

/// What is playing for `viewer`, newest first: their own players always, those of other
/// users only if they share what they play and the track is visible to the viewer.
/// Entries older than [`NOW_PLAYING_TTL_MS`] are left out.
pub fn now_playing(conn: &Connection, viewer: &Access) -> Result<Vec<NowPlayingEntry>> {
    now_playing_at(conn, viewer, now_ms())
}

/// [`now_playing`] as of `now` (Unix milliseconds).
pub fn now_playing_at(
    conn: &Connection,
    viewer: &Access,
    now: i64,
) -> Result<Vec<NowPlayingEntry>> {
    let viewer_key = require_user(conn, &viewer.user)?;
    let rows: Vec<(String, String, i64, TrackId)> = conn
        .prepare(
            "SELECT u.name, np.client, np.started_at, t.public_id
             FROM now_playing np
             JOIN users u ON u.id = np.user_id
             JOIN tracks t ON t.id = np.track_id
             WHERE np.started_at >= ?1 AND (np.user_id = ?2 OR u.share_now_playing = 1)
             ORDER BY np.started_at DESC, np.user_id, np.client",
        )?
        .query_map(params![now - NOW_PLAYING_TTL_MS, viewer_key], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let libs = viewer.visible(None);
    let scope = Scope {
        libs: &libs,
        user: &viewer.user,
        articles: &[],
    };
    let mut out = Vec::new();
    for (user_name, client, started_at, id) in rows {
        let Some(track) = tracks::tracks_by_ids(conn, &scope, &[id])?.pop() else {
            continue;
        };
        out.push(NowPlayingEntry {
            user_name,
            client,
            started_at,
            track,
        });
    }
    Ok(out)
}

/// The saved play queue of a user as its viewer sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayQueueView {
    /// The visible entries, in order.
    pub tracks: Vec<TrackView>,
    /// Index in `tracks` of the track that was playing, if it is still visible.
    pub current: Option<usize>,
    pub position_ms: i64,
    /// When the queue was saved, Unix milliseconds.
    pub changed_at: i64,
    /// Name of the player app that saved it.
    pub changed_by: String,
}

/// Saves the queue of `user`, replacing the previous one (the last save wins).
/// `current` is an index into `tracks`; an index past the end counts as none. Saving no
/// tracks clears the queue.
pub fn save(
    conn: &Connection,
    user: &UserId,
    tracks: &[TrackId],
    current: Option<usize>,
    position_ms: i64,
    client: &str,
) -> Result<()> {
    let user_key = require_user(conn, user)?;
    let keys: Vec<i64> = tracks
        .iter()
        .map(|t| resolve_track(conn, t)?.ok_or_else(|| Error::TrackNotFound(t.clone())))
        .collect::<Result<_>>()?;
    let tx = conn
        .is_autocommit()
        .then(|| conn.unchecked_transaction())
        .transpose()?;
    // The entries go by cascade.
    conn.execute("DELETE FROM play_queues WHERE user_id = ?1", [user_key])?;
    if !keys.is_empty() {
        let current = current
            .filter(|c| *c < keys.len())
            .and_then(|c| i64::try_from(c).ok());
        conn.execute(
            "INSERT INTO play_queues (user_id, current_index, position_ms, changed_at, changed_by)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![user_key, current, position_ms.max(0), now_ms(), client],
        )?;
        let mut insert = conn.prepare_cached(
            "INSERT INTO play_queue_entries (user_id, position, track_id) VALUES (?1, ?2, ?3)",
        )?;
        for (position, key) in keys.iter().enumerate() {
            let position = i64::try_from(position).unwrap_or(i64::MAX);
            insert.execute(params![user_key, position, key])?;
        }
    }
    if let Some(tx) = tx {
        tx.commit()?;
    }
    Ok(())
}

/// The saved queue of the viewer, without the entries they cannot see. `current` is
/// worked out on the visible list; it is `None` if the entry that was playing is hidden.
pub fn load(conn: &Connection, viewer: &Access) -> Result<Option<PlayQueueView>> {
    let user_key = require_user(conn, &viewer.user)?;
    let head: Option<(Option<i64>, i64, i64, String)> = conn
        .query_row(
            "SELECT current_index, position_ms, changed_at, changed_by FROM play_queues
             WHERE user_id = ?1",
            [user_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((current_position, position_ms, changed_at, changed_by)) = head else {
        return Ok(None);
    };
    let stored: Vec<(i64, TrackId)> = conn
        .prepare_cached(
            "SELECT e.position, t.public_id FROM play_queue_entries e
             JOIN tracks t ON t.id = e.track_id
             WHERE e.user_id = ?1 ORDER BY e.position",
        )?
        .query_map([user_key], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let ids: Vec<TrackId> = stored.iter().map(|(_, id)| id.clone()).collect();
    let by_id = visible_views(conn, viewer, &ids)?;
    let mut tracks = Vec::new();
    let mut current = None;
    for (position, id) in &stored {
        if let Some(view) = by_id.get(id) {
            if Some(*position) == current_position {
                current = Some(tracks.len());
            }
            tracks.push(view.clone());
        }
    }
    Ok(Some(PlayQueueView {
        tracks,
        current,
        position_ms,
        changed_at,
        changed_by,
    }))
}
