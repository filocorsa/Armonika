//! Matching files with tracks and writing the result (spec §8.1, §8.7 point 1, §11
//! points 5 and 6).
//!
//! The walk runs to the end before anything is written, so every decision knows the
//! whole picture: which present tracks lost their file in this walk (they will become
//! missing, step 7) is known before the first new file is matched. Those tracks are
//! pre-filter candidates from the start, together with the tracks already missing, so
//! a file moved between two scans is relinked in the same scan without holding new
//! files back until the end.
//!
//! Each file goes through, in order: same path as a present track (unchanged, tags read
//! again, or file read again); same path ignoring case as a present track that lost its
//! file, with the same size and date (renamed in case only); same path as a missing
//! track (reappeared); read, then pre-filter on size, date and duration against exactly
//! one candidate (relinked); otherwise a new track. Files are read in parallel and
//! written in transactions of `batch` files, each followed by a [`LibraryChange`].

use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::time::Instant;

use armonika_core::Error;
use armonika_core::albums::{
    album_artist_ids, album_credit, assign_album, folder_has_mixed_artists, group_key,
    linked_artist_ids, reconcile_albums, track_album,
};
use armonika_core::artists::{
    link_track_artists, reconcile_artists, split_credit, track_artist_ids,
};
use armonika_core::db::Db;
use armonika_core::db::Transaction;
use armonika_core::ids::{ArtistId, LibraryAlbumId, LibraryId, TrackId};
use armonika_core::libraries;
use armonika_core::model::{FileFacts, TagFacts};
use armonika_core::track_files::{
    ContentChange, TrackFile, insert_track, mark_missing, missing_files, present_files,
    set_present_at, write_file,
};
use armonika_media::probe::{ProbeError, TAG_SCHEMA_VERSION, read_file};
use rayon::prelude::*;

use crate::walk::{self, Seen, Walk};
use crate::{LibraryChange, ScanError, ScanOptions, ScanProgress, ScanReport};

/// Largest difference in duration, in milliseconds, for the pre-filter.
const DURATION_TOLERANCE_MS: u64 = 1000;

pub(crate) fn scan(
    db: &Db,
    library: LibraryId,
    opts: ScanOptions,
    progress: &dyn Fn(ScanProgress),
    changes: &dyn Fn(LibraryChange),
) -> Result<ScanReport, ScanError> {
    let started = Instant::now();
    let root = db
        .read_blocking(|c| libraries::get(c, library))?
        .ok_or(Error::LibraryNotFound(library))?
        .root;

    // Step 1: no write at all if the root is not there.
    let unavailable = |reason| {
        tracing::warn!(
            library = %library,
            root = %root.display(),
            %reason,
            "library root unavailable, scan aborted"
        );
        ScanError::RootUnavailable {
            root: root.clone(),
            reason,
        }
    };
    walk::check_root(&root).map_err(unavailable)?;
    let (present, missing) =
        db.read_blocking(|c| Ok((present_files(c, library)?, missing_files(c, library)?)))?;
    // Step 2.
    let walked = walk::walk(&root, progress).map_err(unavailable)?;

    let mut report = ScanReport {
        seen: walked.files.len() as u64 + walked.skipped,
        unreadable: walked.skipped,
        ..ScanReport::default()
    };
    // Steps 3 and 4a-4b: everything that does not need the file's content.
    let mut matcher = Matcher::new(present, missing, &walked);
    let jobs: Vec<Job> = walked
        .files
        .into_iter()
        .filter_map(|file| {
            let action = matcher.classify(&file);
            if matches!(action, Action::Unchanged) {
                report.unchanged += 1;
                None
            } else {
                Some(Job { file, action })
            }
        })
        .collect();

    // Steps 4c-4d, 5 and 6.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads.max(1))
        .thread_name(|i| format!("armonika-scan-{i}"))
        .build()
        .inspect_err(|e| tracing::warn!(error = %e, "no reader threads, reading files one by one"))
        .ok();
    let total = jobs.len() as u64;
    let mut done = 0;
    for chunk in jobs.chunks(opts.batch.max(1)) {
        let reads = read_chunk(pool.as_ref(), chunk);
        report.files_read += reads.iter().filter(|r| r.is_some()).count() as u64;
        let mut change = Change::default();
        db.write_blocking(|conn| {
            let tx = conn.transaction()?;
            for (job, read) in chunk.iter().zip(reads) {
                apply(
                    &tx,
                    library,
                    job,
                    read,
                    &mut matcher,
                    &mut report,
                    &mut change,
                )?;
            }
            change.add_album_artists(&tx)?;
            tx.commit()?;
            Ok(())
        })?;
        change.emit(library, changes);
        done += chunk.len() as u64;
        progress(ScanProgress::Writing { done, total });
    }

    // Steps 7 and 8, in one transaction.
    let hidden = matcher.lost();
    let mut change = Change::default();
    db.write_blocking(|conn| {
        let tx = conn.transaction()?;
        for id in &hidden {
            change.artists.extend(track_artist_ids(&tx, id)?);
            change.albums.extend(track_album(&tx, id)?);
        }
        mark_missing(&tx, &hidden)?;
        change.hidden.extend(hidden.iter().cloned());
        change.reconcile(&tx, library)?;
        tx.commit()?;
        Ok(())
    })?;
    report.missing = hidden.len() as u64;
    change.emit(library, changes);

    report.duration = started.elapsed();
    tracing::info!(
        library = %library,
        seen = report.seen,
        new = report.new,
        changed = report.changed,
        unchanged = report.unchanged,
        missing = report.missing,
        reappeared = report.reappeared,
        relinked_prefilter = report.relinked_prefilter,
        unreadable = report.unreadable,
        files_read = report.files_read,
        duration_ms = report.duration.as_millis() as u64,
        "scan finished"
    );
    Ok(report)
}

/// What to do with a file, decided before reading it.
#[derive(Debug)]
enum Action {
    /// Same path, size, date and tag reader as a present track: nothing to do.
    Unchanged,
    /// Same path as a present track; the file is read again. `Same` when only the tag
    /// reader changed (the content hash stays valid), `Changed` otherwise.
    Reread { id: TrackId, change: ContentChange },
    /// The path of a present track that lost its file, ignoring case, with the same size
    /// and date: the name changed only in case.
    CaseRename { id: TrackId },
    /// Same path as a missing track. The file is read only if `changed` (size or date
    /// differ).
    Reappear { id: TrackId, changed: bool },
    /// No track has this path: read, then pre-filter or new track.
    Unknown,
}

impl Action {
    fn needs_read(&self) -> bool {
        match self {
            Action::Reread { .. } | Action::Unknown => true,
            Action::Reappear { changed, .. } => *changed,
            Action::Unchanged | Action::CaseRename { .. } => false,
        }
    }
}

struct Job {
    file: Seen,
    action: Action,
}

/// The tracks a file can be matched with, and which of them were taken.
struct Matcher {
    present: HashMap<String, TrackFile>,
    /// Tracks without a file: missing ones, and present ones whose path the walk did
    /// not see (they become missing at the end unless a file claims them).
    orphans: Vec<Orphan>,
    /// Present orphans by lower-case path, for case-only renames.
    by_folded_path: HashMap<String, Vec<usize>>,
    /// Missing orphans by path, for reappeared files.
    by_path: HashMap<String, Vec<usize>>,
    /// Orphans by size and date, for the pre-filter.
    by_stat: HashMap<(u64, i64), Vec<usize>>,
}

struct Orphan {
    track: TrackFile,
    /// Present at the start of the scan (its file was not seen by this walk).
    was_present: bool,
    claimed: bool,
}

impl Matcher {
    fn new(present: Vec<TrackFile>, missing: Vec<TrackFile>, walked: &Walk) -> Matcher {
        let seen: HashSet<&str> = walked.files.iter().map(|f| f.rel.as_str()).collect();
        let mut matcher = Matcher {
            present: HashMap::new(),
            orphans: Vec::new(),
            by_folded_path: HashMap::new(),
            by_path: HashMap::new(),
            by_stat: HashMap::new(),
        };
        for track in present {
            if seen.contains(track.rel_path.as_str()) {
                matcher.present.insert(track.rel_path.clone(), track);
            } else if walked.is_blocked(&track.rel_path) {
                // Its directory could not be visited: the file may still be there, so
                // the track is left alone.
                tracing::warn!(path = %track.rel_path, "not visited, track left as it was");
            } else {
                let i = matcher.push(track, true);
                let folded = matcher.orphans[i].track.rel_path.to_lowercase();
                matcher.by_folded_path.entry(folded).or_default().push(i);
            }
        }
        for track in missing {
            let i = matcher.push(track, false);
            let path = matcher.orphans[i].track.rel_path.clone();
            matcher.by_path.entry(path).or_default().push(i);
        }
        matcher
    }

    fn push(&mut self, track: TrackFile, was_present: bool) -> usize {
        let i = self.orphans.len();
        self.by_stat
            .entry((track.size, track.mtime_ms))
            .or_default()
            .push(i);
        self.orphans.push(Orphan {
            track,
            was_present,
            claimed: false,
        });
        i
    }

    fn claim(&mut self, i: usize) -> TrackId {
        self.orphans[i].claimed = true;
        self.orphans[i].track.id.clone()
    }

    /// Steps 3, 4a and 4b.
    fn classify(&mut self, file: &Seen) -> Action {
        if let Some(track) = self.present.get(&file.rel) {
            let same_file = track.size == file.size && track.mtime_ms == file.mtime_ms;
            return match (same_file, track.tag_schema == TAG_SCHEMA_VERSION) {
                (true, true) => Action::Unchanged,
                (true, false) => Action::Reread {
                    id: track.id.clone(),
                    change: ContentChange::Same,
                },
                (false, _) => Action::Reread {
                    id: track.id.clone(),
                    change: ContentChange::Changed,
                },
            };
        }
        let case_match = self
            .by_folded_path
            .get(&file.rel.to_lowercase())
            .into_iter()
            .flatten()
            .copied()
            .find(|&i| {
                let o = &self.orphans[i];
                !o.claimed && o.track.size == file.size && o.track.mtime_ms == file.mtime_ms
            });
        if let Some(i) = case_match {
            return Action::CaseRename { id: self.claim(i) };
        }
        let candidates: Vec<usize> = self
            .by_path
            .get(&file.rel)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| !self.orphans[i].claimed)
            .collect();
        // Several missing tracks can share a path; prefer the one whose file is
        // unchanged, then the one that went missing last.
        let reappeared = candidates.iter().copied().max_by_key(|&i| {
            let t = &self.orphans[i].track;
            (
                t.size == file.size && t.mtime_ms == file.mtime_ms,
                t.missing_since,
            )
        });
        if let Some(i) = reappeared {
            let t = &self.orphans[i].track;
            let changed = t.size != file.size || t.mtime_ms != file.mtime_ms;
            return Action::Reappear {
                id: self.claim(i),
                changed,
            };
        }
        Action::Unknown
    }

    /// Step 4c: the only unclaimed orphan with the same size, date and duration (within
    /// one second), claimed. `None` if there are none or several.
    fn prefilter(&mut self, facts: &FileFacts) -> Option<TrackId> {
        let mut found = self
            .by_stat
            .get(&(facts.size, facts.mtime_ms))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| {
                let o = &self.orphans[i];
                !o.claimed && same_duration(o.track.duration_ms, facts.duration_ms)
            });
        let first = found.next()?;
        if found.next().is_some() {
            return None;
        }
        Some(self.claim(first))
    }

    /// Step 7: the tracks present at the start whose file no file claimed.
    fn lost(&self) -> Vec<TrackId> {
        self.orphans
            .iter()
            .filter(|o| o.was_present && !o.claimed)
            .map(|o| o.track.id.clone())
            .collect()
    }
}

fn same_duration(a: Option<u64>, b: Option<u64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) <= DURATION_TOLERANCE_MS,
        (None, None) => true,
        _ => false,
    }
}

/// The outcome of reading one file: `None` if the job needs no read.
type Read = Option<Result<FileFacts, String>>;

/// Reads the files of `chunk` that need it, in parallel on `pool` when there is one.
fn read_chunk(pool: Option<&rayon::ThreadPool>, chunk: &[Job]) -> Vec<Read> {
    let read = |job: &Job| {
        job.action
            .needs_read()
            .then(|| read_isolated(&job.file.abs))
    };
    match pool {
        Some(pool) => pool.install(|| chunk.par_iter().map(read).collect()),
        None => chunk.iter().map(read).collect(),
    }
}

/// Reads one file; a panic of the tag reader on a malformed file is an error of that
/// file only.
pub(crate) fn read_isolated(path: &Path) -> Result<FileFacts, String> {
    read_isolated_with(path, read_file)
}

fn read_isolated_with(
    path: &Path,
    reader: impl FnOnce(&Path) -> Result<FileFacts, ProbeError>,
) -> Result<FileFacts, String> {
    match panic::catch_unwind(AssertUnwindSafe(|| reader(path))) {
        Ok(Ok(facts)) => Ok(facts),
        Ok(Err(err)) => Err(err.to_string()),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(format!(
                "the tag reader failed on {}: {message}",
                path.display()
            ))
        }
    }
}

/// What one transaction changed, for its [`LibraryChange`].
#[derive(Default)]
pub(crate) struct Change {
    pub(crate) upserted: BTreeSet<TrackId>,
    pub(crate) hidden: BTreeSet<TrackId>,
    pub(crate) albums: BTreeSet<LibraryAlbumId>,
    pub(crate) artists: BTreeSet<ArtistId>,
}

impl Change {
    /// Adds the artists of the albums touched so far: their album lists changed.
    pub(crate) fn add_album_artists(&mut self, tx: &Transaction<'_>) -> armonika_core::Result<()> {
        for album in &self.albums {
            self.artists.extend(album_artist_ids(tx, album)?);
        }
        Ok(())
    }

    /// Brings artists and albums in line after the tracks moved, as the end of a scan
    /// does. Artists that left an album credit are reported too: their album lists
    /// changed and `add_album_artists` only sees the credits that remain.
    pub(crate) fn reconcile(
        &mut self,
        tx: &Transaction<'_>,
        library: LibraryId,
    ) -> armonika_core::Result<()> {
        self.artists.extend(reconcile_artists(tx, library)?);
        let before = linked_artist_ids(tx, library)?;
        self.albums.extend(reconcile_albums(tx, library)?);
        let after = linked_artist_ids(tx, library)?;
        self.artists.extend(before.difference(&after).cloned());
        self.add_album_artists(tx)
    }

    pub(crate) fn emit(self, library: LibraryId, changes: &dyn Fn(LibraryChange)) {
        let event = LibraryChange {
            library,
            tracks_upserted: self.upserted.into_iter().collect(),
            tracks_hidden: self.hidden.into_iter().collect(),
            albums_touched: self.albums.into_iter().collect(),
            artists_touched: self.artists.into_iter().collect(),
        };
        if !event.is_empty() {
            changes(event);
        }
    }
}

/// Writes the outcome of one job. A file that could not be read changes nothing: a
/// track at its path keeps its old size and date, so the next scan reads it again.
fn apply(
    tx: &Transaction<'_>,
    library: LibraryId,
    job: &Job,
    read: Read,
    matcher: &mut Matcher,
    report: &mut ScanReport,
    change: &mut Change,
) -> armonika_core::Result<()> {
    let rel = job.file.rel.as_str();
    let facts = match read {
        Some(Ok(facts)) => Some(facts),
        Some(Err(err)) => {
            tracing::warn!(path = %job.file.abs.display(), error = %err, "cannot read file, skipped");
            report.unreadable += 1;
            return Ok(());
        }
        None => None,
    };
    match (&job.action, facts) {
        (Action::Unchanged, _) => report.unchanged += 1,
        (
            Action::Reread {
                id,
                change: content,
            },
            Some(facts),
        ) => {
            // The file may have changed between the walk and the read: then the hash
            // is no longer valid either.
            let content = if facts.size == job.file.size && facts.mtime_ms == job.file.mtime_ms {
                *content
            } else {
                ContentChange::Changed
            };
            write_track(tx, library, id, rel, &facts, content, change)?;
            report.changed += 1;
        }
        (Action::CaseRename { id }, _) => {
            set_present_at(tx, id, rel)?;
            touch_existing(tx, id, change)?;
            report.changed += 1;
        }
        (Action::Reappear { id, .. }, None) => {
            set_present_at(tx, id, rel)?;
            touch_existing(tx, id, change)?;
            report.reappeared += 1;
        }
        (Action::Reappear { id, .. }, Some(facts)) => {
            write_track(tx, library, id, rel, &facts, ContentChange::Changed, change)?;
            report.reappeared += 1;
        }
        (Action::Unknown, Some(facts)) => match matcher.prefilter(&facts) {
            Some(id) => {
                write_track(
                    tx,
                    library,
                    &id,
                    rel,
                    &facts,
                    ContentChange::Relinked,
                    change,
                )?;
                report.relinked_prefilter += 1;
            }
            None => {
                let id = insert_track(tx, library, rel, &facts, TAG_SCHEMA_VERSION)?;
                link_new(tx, library, &id, &facts.tags, change)?;
                report.new += 1;
            }
        },
        (Action::Reread { .. } | Action::Unknown, None) => {
            unreachable!("these actions always read the file")
        }
    }
    Ok(())
}

/// Writes `facts` into the existing track `id` at `rel` and relinks its artists.
pub(crate) fn write_track(
    tx: &Transaction<'_>,
    library: LibraryId,
    id: &TrackId,
    rel: &str,
    facts: &FileFacts,
    content: ContentChange,
    change: &mut Change,
) -> armonika_core::Result<()> {
    // The artists it had lose a track if the credit changed.
    change.artists.extend(track_artist_ids(tx, id)?);
    write_file(tx, id, rel, facts, TAG_SCHEMA_VERSION, content)?;
    link_new(tx, library, id, &facts.tags, change)
}

/// Links the artists and the album of a track whose tags were just written.
pub(crate) fn link_new(
    tx: &Transaction<'_>,
    library: LibraryId,
    id: &TrackId,
    tags: &TagFacts,
    change: &mut Change,
) -> armonika_core::Result<()> {
    let credit = split_credit(
        tags.artist_credit.as_deref().unwrap_or(""),
        &tags.artists,
        &tags.mb_artist_ids,
    );
    let artists = link_track_artists(tx, id, &credit, tags.artist_sort.as_deref())?;
    let mixed = folder_has_mixed_artists(tx, library, id)?;
    let key = group_key(tags, &album_credit(tags, mixed), artists.first());
    change.artists.extend(artists);
    change
        .albums
        .insert(assign_album(tx, library, id, &key, tags)?);
    change.upserted.insert(id.clone());
    Ok(())
}

/// A track that became present again without reading its file: its artists and its
/// album show it again.
fn touch_existing(
    tx: &Transaction<'_>,
    id: &TrackId,
    change: &mut Change,
) -> armonika_core::Result<()> {
    change.artists.extend(track_artist_ids(tx, id)?);
    change.albums.extend(track_album(tx, id)?);
    change.upserted.insert(id.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_match_within_one_second() {
        assert!(same_duration(Some(180_000), Some(180_999)));
        assert!(same_duration(Some(180_999), Some(180_000)));
        assert!(!same_duration(Some(180_000), Some(181_001)));
        assert!(same_duration(None, None));
        assert!(!same_duration(Some(1), None));
    }

    #[test]
    fn a_panicking_reader_is_an_error_of_that_file() {
        let result = read_isolated_with(Path::new("bad.mp3"), |_| panic!("malformed frame"));
        let err = result.unwrap_err();
        assert!(
            err.contains("bad.mp3") && err.contains("malformed frame"),
            "{err}"
        );
        assert!(read_isolated(Path::new("does/not/exist.mp3")).is_err());
    }
}
