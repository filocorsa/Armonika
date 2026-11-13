//! Library scanner: turns the files of a library into tracks with a stable identity
//! (spec §8.1, §8.7 point 1, §11 points 5 and 6).
//!
//! A scan walks the library root ([`walk`]), matches every audio file with a track
//! and writes the result in batches ([`reconcile`]). Files are only read, never
//! written. A file keeps its track when it is unchanged, when its tags change, when
//! it is renamed or moved (pre-filter on size, modification time and duration), and
//! when it disappears and comes back; a track whose file disappears becomes missing
//! and keeps its personal data.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use armonika_core::db::Db;
use armonika_core::ids::LibraryId;
pub use armonika_core::model::LibraryChange;

mod reconcile;
mod walk;

/// How a scan runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
    /// Threads that read files in parallel (at least 1).
    pub threads: usize,
    /// Files written per database transaction; a [`LibraryChange`] follows each one
    /// (at least 1).
    pub batch: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            threads: std::thread::available_parallelism().map_or(4, |n| n.get().min(8)),
            batch: 500,
        }
    }
}

/// Where a scan is, for progress reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanProgress {
    /// Walking the directory tree: audio files found so far.
    Walking { found: u64 },
    /// Reading and writing the files that need it: done out of total.
    Writing { done: u64, total: u64 },
}

/// What a scan did. Every audio file seen is counted once in `new`, `changed`,
/// `unchanged`, `reappeared`, `relinked_prefilter` or `unreadable`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Audio files found by the walk.
    pub seen: u64,
    /// Files that became new tracks.
    pub new: u64,
    /// Files of a present track read again (size, date or tag reader changed) or
    /// whose name changed only in case.
    pub changed: u64,
    /// Files of a present track with the same size and date: not read.
    pub unchanged: u64,
    /// Present tracks whose file was not found: now missing.
    pub missing: u64,
    /// Missing tracks whose file came back at the same path.
    pub reappeared: u64,
    /// Tracks moved to a new path by the pre-filter (§11 point 6).
    pub relinked_prefilter: u64,
    /// Files that could not be read, or whose path cannot be stored exactly: skipped,
    /// and tried again by the next scan.
    pub unreadable: u64,
    /// Calls to `read_file`: 0 on a scan of an unchanged library.
    pub files_read: u64,
    /// How long the scan took.
    pub duration: Duration,
}

/// Why the library root cannot be scanned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RootProblem {
    #[error("does not exist")]
    NotFound,
    #[error("is not a directory")]
    NotADirectory,
    #[error("cannot be read: permission denied")]
    PermissionDenied,
    #[error("cannot be read: {0}")]
    Other(io::ErrorKind),
}

impl RootProblem {
    fn from_io(err: &io::Error) -> RootProblem {
        match err.kind() {
            io::ErrorKind::NotFound => RootProblem::NotFound,
            io::ErrorKind::NotADirectory => RootProblem::NotADirectory,
            io::ErrorKind::PermissionDenied => RootProblem::PermissionDenied,
            kind => RootProblem::Other(kind),
        }
    }
}

/// Why a scan failed.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The library root is not available (for example a disk that is not mounted).
    /// Nothing was written: no track became missing.
    #[error("library root {} {reason}", root.display())]
    RootUnavailable { root: PathBuf, reason: RootProblem },
    /// The database failed. The batches committed before stay.
    #[error(transparent)]
    Db(#[from] armonika_core::Error),
}

/// Scans the library `library`: blocking, to be called from a dedicated thread.
/// `progress` is called while walking and after each batch; `changes` after each
/// committed batch with what it wrote, so tracks appear during the scan.
pub fn scan_library(
    db: &Db,
    library: LibraryId,
    opts: ScanOptions,
    progress: &dyn Fn(ScanProgress),
    changes: &dyn Fn(LibraryChange),
) -> Result<ScanReport, ScanError> {
    reconcile::scan(db, library, opts, progress, changes)
}
