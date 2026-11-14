//! Background content hash and relinking of moved files (spec §8.7 point 6, §11 point 6).
//!
//! The scan links most moved files to their track with a cheap pre-filter and leaves
//! the content hash to this module: it hashes the audio of the present tracks whose
//! `hash_state` is `pending`, 50 at a time, and for each one, in a single transaction:
//!
//! 1. A track the pre-filter relinked keeps the hash it had before. If the new hash is
//!    the same, the link is confirmed. If not, the link was wrong: the track goes back
//!    to its old path as missing, and the file gets a new track.
//! 2. Otherwise, if exactly one missing track of the library has the same hash (with
//!    several, the one with the same file name), that track takes the file and the
//!    track just created is merged into it: the older identifier, with the personal
//!    data, is the one kept, and the identifier of the absorbed track redirects to it.
//! 3. A file that cannot be read is `failed` and is not tried again until the scan
//!    sees it change (the scan makes it `pending` then).
//!
//! The work pauses for the libraries named by [`HasherHandle::pause`], which the scan
//! controller does while it scans them: `pause` returns when the hasher is no longer
//! working on a track of that library. Files are only read.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use armonika_core::albums::{reconcile_albums, track_album};
use armonika_core::artists::{reconcile_artists, track_artist_ids};
use armonika_core::db::{Connection, Db, Transaction, now_ms};
use armonika_core::ids::{LibraryId, TrackId, add_redirect};
use armonika_core::libraries;
use armonika_core::model::FileFacts;
use armonika_core::paths::rel_path_from_db;
use armonika_core::track_files::{ContentChange, insert_track};
use armonika_media::content_hash::content_hash;
use armonika_media::probe::{TAG_SCHEMA_VERSION, mtime_ms};
use rusqlite::{OptionalExtension, params};
use tokio_util::sync::CancellationToken;

use crate::LibraryChange;
use crate::reconcile::{Change, link_new, read_isolated, write_track};

/// Tracks hashed per database read.
const BATCH: usize = 50;

/// How long [`Hasher::run`] waits before looking for new work.
const IDLE: Duration = Duration::from_secs(2);

/// What one pass of the hasher did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HashReport {
    /// Tracks that got their content hash, the ones below included.
    pub hashed: u64,
    /// Pre-filter links confirmed by an equal hash.
    pub confirmed: u64,
    /// Pre-filter links undone: the file got a new track.
    pub reverted: u64,
    /// Tracks merged into a missing track with the same hash.
    pub merged: u64,
    /// Tracks whose file could not be read: `hash_state` is now `failed`.
    pub failed: u64,
    /// Tracks left `pending` because the file or the row changed meanwhile, or the
    /// database failed: they are looked at again by the next pass.
    pub skipped: u64,
}

/// What the hasher and the scan controller share.
#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    /// Signalled when the hasher finishes a track.
    idle: Condvar,
}

#[derive(Debug, Default)]
struct State {
    paused: HashSet<LibraryId>,
    /// The library of the track being hashed now.
    busy: Option<LibraryId>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A set of ids stays valid whatever a panicking holder was doing.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The background hasher. Create it once, give [`Hasher::handle`] to whoever scans, and
/// run [`Hasher::run`] on a thread of its own.
#[derive(Clone, Debug)]
pub struct Hasher {
    shared: Arc<Shared>,
    /// Reads the tags of a file when a relink needs them (replaced in tests).
    reader: Reader,
}

type Reader = fn(&Path) -> Result<FileFacts, String>;

impl Default for Hasher {
    fn default() -> Hasher {
        Hasher {
            shared: Arc::default(),
            reader: read_isolated,
        }
    }
}

/// Pauses and resumes the hashing of a library, from another thread.
#[derive(Clone, Debug)]
pub struct HasherHandle {
    shared: Arc<Shared>,
}

impl HasherHandle {
    /// No track of `library` is hashed until [`HasherHandle::resume`]. A track being
    /// hashed finishes first: this call waits for it, so when it returns the hasher is
    /// not touching the library. Do not call it from the hasher's own thread.
    pub fn pause(&self, library: LibraryId) {
        let mut state = self.shared.lock();
        state.paused.insert(library);
        while state.busy == Some(library) {
            state = self
                .shared
                .idle
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Hashing of `library` goes on.
    pub fn resume(&self, library: LibraryId) {
        self.shared.lock().paused.remove(&library);
    }

    pub fn is_paused(&self, library: LibraryId) -> bool {
        self.shared.lock().paused.contains(&library)
    }
}

/// Marks a library busy for as long as it lives, also when the hashing panics.
struct Busy<'a>(&'a Shared);

impl<'a> Busy<'a> {
    /// `None` if `library` is paused.
    fn enter(shared: &'a Shared, library: LibraryId) -> Option<Busy<'a>> {
        let mut state = shared.lock();
        if state.paused.contains(&library) {
            return None;
        }
        state.busy = Some(library);
        Some(Busy(shared))
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.lock().busy = None;
        self.0.idle.notify_all();
    }
}

/// A present track waiting for its hash.
struct Pending {
    key: i64,
    id: TrackId,
    library: LibraryId,
    rel: String,
    size: u64,
    mtime_ms: i64,
    /// Hash before the pre-filter relink, if the track had one.
    prev_hash: Option<Vec<u8>>,
    prev_rel: Option<String>,
}

/// What to do with a hashed track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// Store the hash: nothing to link.
    Keep,
    /// The pre-filter link is right.
    Confirm,
    /// The pre-filter link is wrong.
    Revert,
    /// Merge into the missing track with this key.
    Merge { target: i64 },
}

enum Outcome {
    Done(Plan),
    Skipped,
}

impl Hasher {
    pub fn new() -> Hasher {
        Hasher::default()
    }

    pub fn handle(&self) -> HasherHandle {
        HasherHandle {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Hashes the pending tracks until `stop` is cancelled, looking for new work every
    /// couple of seconds. `changes` is called after each relink that changes what the
    /// apps see. Blocking: run it on a thread of its own.
    pub fn run(&self, db: Db, stop: CancellationToken, changes: impl Fn(LibraryChange)) {
        while !stop.is_cancelled() {
            let report = self.hash_pending(&db, &stop, &changes);
            if report != HashReport::default() {
                tracing::info!(
                    hashed = report.hashed,
                    confirmed = report.confirmed,
                    reverted = report.reverted,
                    merged = report.merged,
                    failed = report.failed,
                    skipped = report.skipped,
                    "content hash pass finished"
                );
            }
            // Sleep in short slices so that a stop is heard at once.
            let mut waited = Duration::ZERO;
            while waited < IDLE && !stop.is_cancelled() {
                std::thread::sleep(Duration::from_millis(100));
                waited += Duration::from_millis(100);
            }
        }
    }

    /// One pass: hashes every track that is pending now (a track is looked at once per
    /// pass), skipping the paused libraries, and returns when there is no more work or
    /// `stop` is cancelled.
    pub fn hash_pending(
        &self,
        db: &Db,
        stop: &CancellationToken,
        changes: &dyn Fn(LibraryChange),
    ) -> HashReport {
        let mut report = HashReport::default();
        let mut last = 0;
        while !stop.is_cancelled() {
            let paused: Vec<LibraryId> = self.shared.lock().paused.iter().copied().collect();
            let batch = match db.read_blocking(|c| pending_batch(c, last, &paused)) {
                Ok(batch) => batch,
                Err(err) => {
                    tracing::warn!(error = %err, "cannot list the tracks to hash");
                    break;
                }
            };
            let Some(end) = batch.last().map(|p| p.key) else {
                break;
            };
            last = end;
            let mut roots: HashMap<LibraryId, Option<PathBuf>> = HashMap::new();
            for job in &batch {
                if stop.is_cancelled() {
                    break;
                }
                // Held until the track is done: a pause waits for it.
                let Some(_busy) = Busy::enter(&self.shared, job.library) else {
                    continue;
                };
                let root = roots.entry(job.library).or_insert_with(|| {
                    db.read_blocking(|c| libraries::get(c, job.library))
                        .ok()
                        .flatten()
                        .map(|l| l.root)
                });
                let Some(root) = root.as_deref() else {
                    report.skipped += 1;
                    continue;
                };
                match process(db, root, job, self.reader, changes) {
                    Ok(Step::Hashed(plan)) => {
                        report.hashed += 1;
                        match plan {
                            Plan::Keep => {}
                            Plan::Confirm => report.confirmed += 1,
                            Plan::Revert => report.reverted += 1,
                            Plan::Merge { .. } => report.merged += 1,
                        }
                    }
                    Ok(Step::Failed) => report.failed += 1,
                    Ok(Step::Skipped) => report.skipped += 1,
                    Err(err) => {
                        tracing::warn!(path = %job.rel, error = %err, "cannot store a content hash");
                        report.skipped += 1;
                    }
                }
            }
        }
        report
    }
}

enum Step {
    Hashed(Plan),
    Failed,
    Skipped,
}

/// The next tracks to hash after the internal key `after`, in key order, leaving out
/// the libraries in `paused`.
fn pending_batch(
    conn: &Connection,
    after: i64,
    paused: &[LibraryId],
) -> armonika_core::Result<Vec<Pending>> {
    let excluded: Vec<String> = paused.iter().map(|l| l.0.to_string()).collect();
    let filter = if excluded.is_empty() {
        String::new()
    } else {
        format!("AND library_id NOT IN ({})", excluded.join(","))
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT id, public_id, library_id, rel_path, size, mtime_ms,
                prev_content_hash, prev_rel_path
         FROM tracks
         WHERE state = 'present' AND hash_state = 'pending' AND id > ?1 {filter}
         ORDER BY id LIMIT {BATCH}"
    ))?;
    let rows = stmt.query_map([after], |r| {
        Ok(Pending {
            key: r.get(0)?,
            id: r.get(1)?,
            library: LibraryId(r.get(2)?),
            rel: r.get(3)?,
            size: r.get::<_, i64>(4)?.unsigned_abs(),
            mtime_ms: r.get(5)?,
            prev_hash: r.get(6)?,
            prev_rel: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Hashes one track and writes the outcome in one transaction.
fn process(
    db: &Db,
    root: &Path,
    job: &Pending,
    reader: Reader,
    changes: &dyn Fn(LibraryChange),
) -> armonika_core::Result<Step> {
    let abs = root.join(rel_path_from_db(&job.rel));
    // A file that is gone or changed since the scan is the scan's business.
    if !still_as_scanned(&abs, job) {
        return Ok(Step::Skipped);
    }
    let hash = match hash_isolated(&abs) {
        Ok(hash) => hash,
        Err(message) => {
            tracing::warn!(path = %abs.display(), error = %message, "cannot hash file");
            return mark_failed(db, job);
        }
    };
    if !still_as_scanned(&abs, job) {
        return Ok(Step::Skipped);
    }

    let planned = db.read_blocking(|c| plan(c, job, &hash))?;
    let facts = match planned {
        Plan::Revert | Plan::Merge { .. } => match reader(&abs) {
            Ok(facts) if facts.size == job.size && facts.mtime_ms == job.mtime_ms => Some(facts),
            Ok(_) => return Ok(Step::Skipped),
            Err(message) => {
                tracing::warn!(path = %abs.display(), error = %message, "cannot read file");
                if matches!(planned, Plan::Merge { .. }) {
                    return mark_failed(db, job);
                }
                // A revert does not need the tags: the proven-wrong link is undone
                // anyway, and the next scan reads the file as a new one.
                None
            }
        },
        Plan::Keep | Plan::Confirm => None,
    };

    let mut change = Change::default();
    let outcome = db.write_blocking(|conn| {
        let tx = conn.transaction()?;
        let outcome = apply(&tx, job, &hash, planned, facts.as_ref(), &mut change)?;
        if matches!(outcome, Outcome::Done(_)) {
            tx.commit()?;
        }
        Ok(outcome)
    })?;
    match outcome {
        Outcome::Done(plan) => {
            change.emit(job.library, changes);
            Ok(Step::Hashed(plan))
        }
        Outcome::Skipped => Ok(Step::Skipped),
    }
}

/// Whether the file at `abs` still has the size and modification time the scan saw.
fn still_as_scanned(abs: &Path, job: &Pending) -> bool {
    fs::metadata(abs)
        .and_then(|meta| Ok(meta.len() == job.size && mtime_ms(&meta)? == job.mtime_ms))
        .unwrap_or(false)
}

/// Hashes one file; a panic of the demuxer on a malformed file is an error of that
/// file only.
fn hash_isolated(path: &Path) -> Result<[u8; 32], String> {
    hash_isolated_with(path, content_hash)
}

fn hash_isolated_with<E: std::fmt::Display>(
    path: &Path,
    hasher: impl FnOnce(&Path) -> Result<[u8; 32], E>,
) -> Result<[u8; 32], String> {
    match panic::catch_unwind(AssertUnwindSafe(|| hasher(path))) {
        Ok(Ok(hash)) => Ok(hash),
        Ok(Err(err)) => Err(err.to_string()),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(format!(
                "the audio reader failed on {}: {message}",
                path.display()
            ))
        }
    }
}

/// Marks the track `failed`, if it is still the file that failed: no link is made or
/// undone (a pre-filter link stays as it is and its `prev_*` columns are cleared: with
/// no hash there is nothing to compare), and the scan makes the track pending again when the file changes.
fn mark_failed(db: &Db, job: &Pending) -> armonika_core::Result<Step> {
    let rows = db.write_blocking(|conn| {
        let tx = conn.transaction()?;
        let rows = tx
            .prepare_cached(
                "UPDATE tracks SET hash_state = 'failed', prev_content_hash = NULL,
                                   prev_rel_path = NULL
                 WHERE id = ?1 AND state = 'present' AND hash_state = 'pending'
                   AND rel_path = ?2 AND size = ?3 AND mtime_ms = ?4",
            )?
            .execute(params![
                job.key,
                job.rel,
                i64::try_from(job.size).unwrap_or(i64::MAX),
                job.mtime_ms
            ])?;
        tx.commit()?;
        Ok(rows)
    })?;
    Ok(if rows == 1 {
        Step::Failed
    } else {
        Step::Skipped
    })
}

/// What to do with `job`, whose file has `hash`.
fn plan(conn: &Connection, job: &Pending, hash: &[u8; 32]) -> armonika_core::Result<Plan> {
    if let Some(prev) = &job.prev_hash {
        return Ok(if prev.as_slice() == hash.as_slice() {
            Plan::Confirm
        } else {
            Plan::Revert
        });
    }
    if job.prev_rel.is_some() {
        // Relinked by the pre-filter with no hash to compare: the link stays, and the
        // track must not be merged into another one, it is the older identity itself.
        return Ok(Plan::Keep);
    }
    let candidates: Vec<(i64, String)> = conn
        .prepare_cached(
            "SELECT id, rel_path FROM tracks
             WHERE library_id = ?1 AND state = 'missing' AND content_hash = ?2 AND id != ?3
             ORDER BY id",
        )?
        .query_map(params![job.library.0, hash.as_slice(), job.key], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let target = match candidates.as_slice() {
        [] => None,
        [(key, _)] => Some(*key),
        several => {
            let name = file_name(&job.rel);
            let mut same = several.iter().filter(|(_, rel)| file_name(rel) == name);
            match (same.next(), same.next()) {
                (Some((key, _)), None) => Some(*key),
                _ => None,
            }
        }
    };
    Ok(target.map_or(Plan::Keep, |target| Plan::Merge { target }))
}

fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Writes the outcome for `job` in `tx`, if the row is still what was hashed and the
/// plan still holds; otherwise the transaction is left to roll back.
fn apply(
    tx: &Transaction<'_>,
    job: &Pending,
    hash: &[u8; 32],
    planned: Plan,
    facts: Option<&FileFacts>,
    change: &mut Change,
) -> armonika_core::Result<Outcome> {
    type Current = (String, String, String, i64, i64, Option<Vec<u8>>);
    let current: Option<Current> = tx
        .prepare_cached(
            "SELECT state, hash_state, rel_path, size, mtime_ms, prev_content_hash
             FROM tracks WHERE id = ?1",
        )?
        .query_row([job.key], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .optional()?;
    let unchanged = current.is_some_and(|(state, hash_state, rel, size, mtime, prev)| {
        state == "present"
            && hash_state == "pending"
            && rel == job.rel
            && size.unsigned_abs() == job.size
            && mtime == job.mtime_ms
            && prev == job.prev_hash
    });
    if !unchanged || plan(tx, job, hash)? != planned {
        return Ok(Outcome::Skipped);
    }
    let library = job.library;
    match (planned, facts) {
        (Plan::Keep | Plan::Confirm, _) => store_hash(tx, job.key, hash)?,
        (Plan::Revert, facts) => {
            change.artists.extend(track_artist_ids(tx, &job.id)?);
            change.albums.extend(track_album(tx, &job.id)?);
            // The track goes back to the file it had, which is not there now. Its tag
            // columns are those of the other file: tag_schema 0 makes the next scan read
            // the original again when it comes back.
            tx.prepare_cached(
                "UPDATE tracks SET state = 'missing', missing_since = ?2, updated_at = ?2,
                                   rel_path = COALESCE(prev_rel_path, rel_path),
                                   tag_schema = 0,
                                   content_hash = prev_content_hash, hash_state = 'done',
                                   prev_content_hash = NULL, prev_rel_path = NULL
                 WHERE id = ?1",
            )?
            .execute(params![job.key, now_ms()])?;
            if facts.is_none() {
                // No track takes the file now. The old track must not match it again
                // in the pre-filter, which would repeat the wrong link: its size and
                // date are those of the other file, so they are cleared.
                tx.prepare_cached("UPDATE tracks SET size = 0, mtime_ms = 0 WHERE id = ?1")?
                    .execute([job.key])?;
            }
            if let Some(facts) = facts {
                let fresh = insert_track(tx, library, &job.rel, facts, TAG_SCHEMA_VERSION)?;
                let key = armonika_core::ids::resolve_track(tx, &fresh)?.unwrap_or_default();
                store_hash(tx, key, hash)?;
                link_new(tx, library, &fresh, &facts.tags, change)?;
            }
            change.hidden.insert(job.id.clone());
            reconcile(tx, library, change)?;
        }
        (Plan::Merge { target }, Some(facts)) => {
            let kept: TrackId = tx
                .prepare_cached("SELECT public_id FROM tracks WHERE id = ?1")?
                .query_row([target], |r| r.get(0))?;
            change.artists.extend(track_artist_ids(tx, &job.id)?);
            change.albums.extend(track_album(tx, &job.id)?);
            merge_personal_data(tx, job.key, target)?;
            add_redirect(tx, job.id.as_str(), kept.as_str())?;
            tx.prepare_cached("DELETE FROM tracks WHERE id = ?1")?
                .execute([job.key])?;
            change.hidden.insert(job.id.clone());
            write_track(
                tx,
                library,
                &kept,
                &job.rel,
                facts,
                ContentChange::Changed,
                change,
            )?;
            store_hash(tx, target, hash)?;
            reconcile(tx, library, change)?;
        }
        (Plan::Merge { .. }, None) => return Ok(Outcome::Skipped),
    }
    Ok(Outcome::Done(planned))
}

/// The hash of the track with internal key `key` is `hash`; no link to confirm any more.
fn store_hash(tx: &Transaction<'_>, key: i64, hash: &[u8; 32]) -> armonika_core::Result<()> {
    tx.prepare_cached(
        "UPDATE tracks SET content_hash = ?2, hash_state = 'done',
                           prev_content_hash = NULL, prev_rel_path = NULL
         WHERE id = ?1",
    )?
    .execute(params![key, hash.as_slice()])?;
    Ok(())
}

/// Brings artists and albums in line after the tracks moved, as the end of a scan does.
fn reconcile(
    tx: &Transaction<'_>,
    library: LibraryId,
    change: &mut Change,
) -> armonika_core::Result<()> {
    change.artists.extend(reconcile_artists(tx, library)?);
    change.albums.extend(reconcile_albums(tx, library)?);
    change.add_album_artists(tx)
}

/// Moves what users keep about the track `from` onto the track `to`, which is kept
/// (§8.7 point 6): starred if either was (the oldest date), the highest rating, play
/// counts added, the latest `last_played_at`; history, playlist entries, play queue,
/// now-playing and scrobble outbox point to `to`.
fn merge_personal_data(tx: &Transaction<'_>, from: i64, to: i64) -> armonika_core::Result<()> {
    type Annotation = (i64, Option<i64>, Option<i64>, i64, Option<i64>);
    let rows: Vec<Annotation> = tx
        .prepare_cached(
            "SELECT user_id, starred_at, rating, play_count, last_played_at
             FROM track_annotations WHERE track_id = ?1",
        )?
        .query_map([from], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (user, starred, rating, plays, last) in rows {
        let kept: Option<Annotation> = tx
            .prepare_cached(
                "SELECT user_id, starred_at, rating, play_count, last_played_at
                 FROM track_annotations WHERE user_id = ?1 AND track_id = ?2",
            )?
            .query_row(params![user, to], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .optional()?;
        match kept {
            None => {
                tx.prepare_cached(
                    "UPDATE track_annotations SET track_id = ?2
                     WHERE user_id = ?1 AND track_id = ?3",
                )?
                .execute(params![user, to, from])?;
            }
            Some((_, k_starred, k_rating, k_plays, k_last)) => {
                let starred = [starred, k_starred].into_iter().flatten().min();
                let rating = [rating, k_rating].into_iter().flatten().max();
                let last = [last, k_last].into_iter().flatten().max();
                tx.prepare_cached(
                    "UPDATE track_annotations
                     SET starred_at = ?3, rating = ?4, play_count = ?5, last_played_at = ?6
                     WHERE user_id = ?1 AND track_id = ?2",
                )?
                .execute(params![
                    user,
                    to,
                    starred,
                    rating,
                    plays + k_plays,
                    last
                ])?;
            }
        }
    }
    for table in [
        "play_history",
        "playlist_entries",
        "play_queue_entries",
        "scrobble_outbox",
    ] {
        tx.execute(
            &format!("UPDATE {table} SET track_id = ?1 WHERE track_id = ?2"),
            params![to, from],
        )?;
    }
    // One row per user and client: the kept track's own row wins on a clash.
    tx.execute(
        "UPDATE OR IGNORE now_playing SET track_id = ?1 WHERE track_id = ?2",
        params![to, from],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_hasher_is_an_error_of_that_file() {
        let result = hash_isolated_with(Path::new("bad.flac"), |_| -> Result<[u8; 32], String> {
            panic!("malformed block")
        });
        let err = result.unwrap_err();
        assert!(
            err.contains("bad.flac") && err.contains("malformed block"),
            "{err}"
        );
        assert!(hash_isolated(Path::new("does/not/exist.mp3")).is_err());
    }

    #[test]
    fn revert_without_readable_tags_still_undoes_the_link() {
        use armonika_core::libraries;
        use armonika_testdata::AudioFormat;
        use armonika_testdata::fixture::{FixtureTags, tagged_file};

        let data = tempfile::tempdir().unwrap();
        let music = tempfile::tempdir().unwrap();
        let db = Db::open(&data.path().join("armonika.db")).unwrap();
        let lib = db
            .write_blocking(|c| libraries::create(c, "Music", music.path(), data.path()))
            .unwrap();
        let scan = || {
            crate::scan_library(&db, lib, crate::ScanOptions::default(), &|_| {}, &|_| {}).unwrap()
        };
        let tags = FixtureTags {
            title: Some("Song".to_owned()),
            ..FixtureTags::default()
        };
        let a = tagged_file(music.path(), "a.mp3", AudioFormat::Mp3, &tags);
        let mtime = fs::metadata(&a).unwrap().modified().unwrap();
        scan();
        let hasher = Hasher::new();
        let stop = CancellationToken::new();
        hasher.hash_pending(&db, &stop, &|_| {});
        let old_hash: Vec<u8> = db
            .read_blocking(
                |c| Ok(c.query_row("SELECT content_hash FROM tracks", [], |r| r.get(0))?),
            )
            .unwrap();

        // Different audio, same size and date, at another path: the pre-filter relinks.
        let mut bytes = fs::read(&a).unwrap();
        let at = bytes.len() - 200;
        bytes[at] ^= 0xFF;
        fs::remove_file(&a).unwrap();
        let b = music.path().join("b.mp3");
        fs::write(&b, bytes).unwrap();
        fs::File::options()
            .write(true)
            .open(&b)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        assert_eq!(scan().relinked_prefilter, 1);

        let failing = Hasher {
            reader: |_| Err("cannot read tags".to_owned()),
            ..hasher
        };
        let report = failing.hash_pending(&db, &stop, &|_| {});
        assert_eq!(report.reverted, 1, "{report:?}");
        assert_eq!(report.failed, 0);
        let row: (String, String, Option<Vec<u8>>, Option<String>, String) = db
            .read_blocking(|c| {
                Ok(c.query_row(
                    "SELECT state, rel_path, content_hash, prev_rel_path, hash_state
                     FROM tracks",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )?)
            })
            .unwrap();
        assert_eq!(row.0, "missing");
        assert_eq!(row.1, "a.mp3");
        assert_eq!(row.2, Some(old_hash));
        assert_eq!(row.3, None);
        assert_eq!(row.4, "done");

        // The next scan reads the file as a new one.
        assert_eq!(scan().new, 1);
    }

    /// The reader of `pause_waits_for_the_track_in_flight`: says it was entered and
    /// waits for the test to let it go.
    static GATE: (Mutex<(bool, bool)>, Condvar) = (Mutex::new((false, false)), Condvar::new());

    fn gated_reader(path: &Path) -> Result<FileFacts, String> {
        let (lock, cv) = &GATE;
        let mut gate = lock.lock().unwrap();
        gate.0 = true;
        cv.notify_all();
        while !gate.1 {
            gate = cv.wait(gate).unwrap();
        }
        drop(gate);
        read_isolated(path)
    }

    #[test]
    fn pause_waits_for_the_track_in_flight() {
        use armonika_core::libraries;
        use armonika_testdata::AudioFormat;
        use armonika_testdata::fixture::{FixtureTags, tagged_file};
        use std::sync::atomic::{AtomicBool, Ordering};

        let data = tempfile::tempdir().unwrap();
        let music = tempfile::tempdir().unwrap();
        let db = Db::open(&data.path().join("armonika.db")).unwrap();
        let lib = db
            .write_blocking(|c| libraries::create(c, "Music", music.path(), data.path()))
            .unwrap();
        let scan = || {
            crate::scan_library(&db, lib, crate::ScanOptions::default(), &|_| {}, &|_| {}).unwrap()
        };
        let tags = |title: &str| FixtureTags {
            title: Some(title.to_owned()),
            ..FixtureTags::default()
        };
        let set_mtime = |path: &Path, secs: u64| {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
                .unwrap();
        };
        // The same audio at a new path with other tags and another date: the hash
        // merges the new track into the missing one, which needs the gated reader.
        let a = tagged_file(music.path(), "a.mp3", AudioFormat::Mp3, &tags("Song"));
        set_mtime(&a, 1_600_000_000);
        scan();
        let hasher = Hasher::new();
        let stop = CancellationToken::new();
        hasher.hash_pending(&db, &stop, &|_| {});
        fs::remove_file(&a).unwrap();
        let b = tagged_file(
            music.path(),
            "b.mp3",
            AudioFormat::Mp3,
            &tags("Song, retagged"),
        );
        set_mtime(&b, 1_700_000_000);
        scan();

        let hasher = Hasher {
            reader: gated_reader,
            ..hasher
        };
        let handle = hasher.handle();
        let worker = {
            let (db, stop) = (db.clone(), stop.clone());
            std::thread::spawn(move || hasher.hash_pending(&db, &stop, &|_| {}))
        };
        let (lock, cv) = &GATE;
        {
            let mut gate = lock.lock().unwrap();
            while !gate.0 {
                gate = cv.wait(gate).unwrap();
            }
        }
        // The hasher is inside the track: pause must not return before it is done.
        let paused = Arc::new(AtomicBool::new(false));
        let pauser = {
            let (handle, paused) = (handle.clone(), Arc::clone(&paused));
            std::thread::spawn(move || {
                handle.pause(lib);
                paused.store(true, Ordering::SeqCst);
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !paused.load(Ordering::SeqCst),
            "pause returned during a track"
        );
        assert!(handle.is_paused(lib));
        {
            lock.lock().unwrap().1 = true;
            cv.notify_all();
        }
        pauser.join().unwrap();
        let report = worker.join().unwrap();
        assert_eq!(report.merged, 1, "{report:?}");
        assert!(paused.load(Ordering::SeqCst));
    }

    #[test]
    fn file_name_is_the_last_component() {
        assert_eq!(file_name("a/b/song.mp3"), "song.mp3");
        assert_eq!(file_name("song.mp3"), "song.mp3");
    }
}
