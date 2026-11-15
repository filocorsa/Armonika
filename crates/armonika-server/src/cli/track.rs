//! The `track` command: delete the tracks whose file is gone.
//!
//! A track whose file disappears becomes `missing` and keeps its personal data, in case
//! the file comes back (spec §8.1). This command is how an admin deletes such tracks
//! for good. It works on the database only: no file is touched. It can run while the
//! server is running (SQLite WAL).

use std::time::Duration;

use armonika_core::albums::reconcile_albums;
use armonika_core::artists::reconcile_artists;
use armonika_core::db::{Connection, Db, now_ms};
use armonika_core::ids::LibraryId;
use clap::Subcommand;
use rusqlite::params;

use super::library::find;

/// Manage tracks.
#[derive(Debug, Subcommand)]
pub enum TrackCommand {
    /// Permanently delete the missing tracks of a library, with the data linked to them
    /// (ratings, stars, play history, playlist entries). Without --yes, only print what
    /// would be deleted.
    PurgeMissing {
        /// Name of the library.
        library: String,
        /// Only the tracks missing for at least this long, for example 30d or 12h.
        #[arg(long, value_parser = parse_duration)]
        older_than: Option<Duration>,
        /// Really delete them.
        #[arg(long)]
        yes: bool,
    },
}

fn parse_duration(text: &str) -> Result<Duration, String> {
    humantime::parse_duration(text).map_err(|e| e.to_string())
}

/// What purging deletes, or would delete.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PurgeSummary {
    pub tracks: u64,
    pub annotations: u64,
    /// Playlists that lose at least one entry.
    pub playlists: u64,
    pub history: u64,
    /// Entries removed from playlists.
    pub playlist_entries: u64,
    /// Entries removed from saved play queues.
    pub queue_entries: u64,
}

pub fn run(db: &Db, command: TrackCommand) -> anyhow::Result<()> {
    match command {
        TrackCommand::PurgeMissing {
            library,
            older_than,
            yes,
        } => {
            let cutoff = older_than.map(cutoff_ms);
            if yes {
                let s = db.write_blocking(|c| {
                    let id = find(c, &library)?.id;
                    purge_missing(c, id, cutoff)
                })?;
                println!(
                    "purged library {library}: {} missing tracks deleted, with {} annotations, \
                     {} history entries, {} playlist entries (in {} playlists) and {} play queue entries; no file was touched",
                    s.tracks,
                    s.annotations,
                    s.history,
                    s.playlist_entries,
                    s.playlists,
                    s.queue_entries
                );
            } else {
                let s = db.read_blocking(|c| {
                    let id = find(c, &library)?.id;
                    summary(c, id, cutoff)
                })?;
                println!(
                    "purging the missing tracks of library {library} would delete {} tracks, \
                     {} annotations, {} history entries, {} playlist entries (in {} playlists) and {} play queue entries",
                    s.tracks,
                    s.annotations,
                    s.history,
                    s.playlist_entries,
                    s.playlists,
                    s.queue_entries
                );
                println!("nothing done: run again with --yes to delete them");
            }
        }
    }
    Ok(())
}

/// The latest `missing_since` a track may have to be purged.
fn cutoff_ms(older_than: Duration) -> i64 {
    let age = i64::try_from(older_than.as_millis()).unwrap_or(i64::MAX);
    now_ms().saturating_sub(age)
}

/// Tracks to purge: missing, and with `missing_since` up to `?2` when it is not NULL.
const SELECTED: &str = "SELECT id FROM tracks WHERE library_id = ?1 AND state = 'missing'
                        AND (?2 IS NULL OR missing_since <= ?2)";

fn summary(
    conn: &Connection,
    library: LibraryId,
    cutoff: Option<i64>,
) -> armonika_core::Result<PurgeSummary> {
    let count = |sql: String| -> armonika_core::Result<u64> {
        let n: i64 = conn.query_row(&sql, params![library.0, cutoff], |r| r.get(0))?;
        Ok(n.unsigned_abs())
    };
    Ok(PurgeSummary {
        tracks: count(format!("SELECT COUNT(*) FROM ({SELECTED})"))?,
        annotations: count(format!(
            "SELECT COUNT(*) FROM track_annotations WHERE track_id IN ({SELECTED})"
        ))?,
        playlists: count(format!(
            "SELECT COUNT(DISTINCT playlist_id) FROM playlist_entries
             WHERE track_id IN ({SELECTED})"
        ))?,
        history: count(format!(
            "SELECT COUNT(*) FROM play_history WHERE track_id IN ({SELECTED})"
        ))?,
        playlist_entries: count(format!(
            "SELECT COUNT(*) FROM playlist_entries WHERE track_id IN ({SELECTED})"
        ))?,
        queue_entries: count(format!(
            "SELECT COUNT(*) FROM play_queue_entries WHERE track_id IN ({SELECTED})"
        ))?,
    })
}

/// Deletes the selected tracks in one transaction; the foreign keys cascade to
/// everything linked to them. Albums and artists that no track needs any more go too.
fn purge_missing(
    conn: &mut Connection,
    library: LibraryId,
    cutoff: Option<i64>,
) -> armonika_core::Result<PurgeSummary> {
    let tx = conn.transaction()?;
    let done = summary(&tx, library, cutoff)?;
    tx.execute(
        &format!(
            "UPDATE playlists SET updated_at = ?3 WHERE id IN (
               SELECT playlist_id FROM playlist_entries WHERE track_id IN ({SELECTED}))"
        ),
        params![library.0, cutoff, now_ms()],
    )?;
    tx.execute(
        &format!("DELETE FROM tracks WHERE id IN ({SELECTED})"),
        params![library.0, cutoff],
    )?;
    reconcile_artists(&tx, library)?;
    reconcile_albums(&tx, library)?;
    tx.commit()?;
    Ok(done)
}
