//! What users are listening to right now. (The saved play queue itself arrives with the
//! playlist endpoints.)

use rusqlite::{Connection, params};

use crate::access::Access;
use crate::db::now_ms;
use crate::ids::{TrackId, UserId, resolve_track};
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
