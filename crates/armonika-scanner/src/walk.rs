//! Walking the library root: which audio files exist, with their size and
//! modification time. Nothing is read here beyond directory listings and `stat`.

use std::fs;
use std::path::{Component, Path, PathBuf};

use armonika_core::model::AudioFormat;
use armonika_core::paths::{rel_path_from_db, rel_path_to_db};
use armonika_media::probe::mtime_ms;
use walkdir::WalkDir;

use crate::{RootProblem, ScanProgress};

/// The directory at the library root that Armonika reserves for itself; never scanned.
const RESERVED_DIR: &str = ".armonika";

/// How often the walk reports progress, in audio files found.
const PROGRESS_EVERY: u64 = 1000;

/// An audio file found by the walk.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    /// Path relative to the root, in database form.
    pub rel: String,
    /// Full path, to read the file.
    pub abs: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
}

/// What the walk found.
#[derive(Debug, Default)]
pub(crate) struct Walk {
    /// Audio files, in walk order (sorted by name inside each directory).
    pub files: Vec<Seen>,
    /// Audio files skipped because their path cannot be stored exactly or their
    /// metadata cannot be read.
    pub skipped: u64,
    /// Paths (database form) the walk could not look into or stat: a track at or below
    /// one of them may still exist, so it must not become missing.
    pub blocked: Vec<String>,
}

impl Walk {
    /// Whether `rel` is at or below a path the walk could not visit.
    pub fn is_blocked(&self, rel: &str) -> bool {
        self.blocked.iter().any(|b| {
            rel.strip_prefix(b.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    }
}

/// Checks that `root` is a directory that can be listed.
pub(crate) fn check_root(root: &Path) -> Result<(), RootProblem> {
    let meta = fs::metadata(root).map_err(|e| RootProblem::from_io(&e))?;
    if !meta.is_dir() {
        return Err(RootProblem::NotADirectory);
    }
    fs::read_dir(root).map_err(|e| RootProblem::from_io(&e))?;
    Ok(())
}

/// Walks `root`, following links (a loop is an error of that entry only) and skipping
/// the reserved `.armonika` directory at the root. Fails only if the root itself cannot
/// be listed.
pub(crate) fn walk(root: &Path, progress: &dyn Fn(ScanProgress)) -> Result<Walk, RootProblem> {
    let mut out = Walk::default();
    let entries = WalkDir::new(root)
        .follow_links(true)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !(e.depth() == 1 && e.file_type().is_dir() && is_reserved(e.path())));
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                if err.depth() == 0 {
                    return Err(err
                        .io_error()
                        .map_or(RootProblem::Other(std::io::ErrorKind::Other), |e| {
                            RootProblem::from_io(e)
                        }));
                }
                let path = err.path().map(Path::to_path_buf);
                if let Some(ancestor) = err.loop_ancestor() {
                    tracing::warn!(
                        path = ?path,
                        ancestor = %ancestor.display(),
                        "link loop in the library, not followed"
                    );
                } else {
                    tracing::warn!(error = %err, "cannot visit an entry of the library, skipped");
                }
                if let Some(path) = path {
                    if is_audio(&path) {
                        out.skipped += 1;
                    }
                    if let Some(rel) = path.strip_prefix(root).ok().and_then(db_rel_path) {
                        out.blocked.push(rel);
                    }
                }
                continue;
            }
        };
        if !entry.file_type().is_file() || !is_audio(entry.path()) {
            continue;
        }
        let Some(rel) = entry.path().strip_prefix(root).ok().and_then(db_rel_path) else {
            tracing::warn!(
                path = %entry.path().display(),
                "file name cannot be stored exactly (not valid Unicode), skipped"
            );
            out.skipped += 1;
            continue;
        };
        let stat = entry
            .metadata()
            .map_err(std::io::Error::from)
            .and_then(|meta| Ok((meta.len(), mtime_ms(&meta)?)));
        match stat {
            Ok((size, mtime_ms)) => {
                out.files.push(Seen {
                    rel,
                    abs: entry.into_path(),
                    size,
                    mtime_ms,
                });
                let found = out.files.len() as u64;
                if found.is_multiple_of(PROGRESS_EVERY) {
                    progress(ScanProgress::Walking { found });
                }
            }
            Err(err) => {
                tracing::warn!(path = %entry.path().display(), error = %err, "cannot stat file, skipped");
                out.skipped += 1;
                out.blocked.push(rel);
            }
        }
    }
    progress(ScanProgress::Walking {
        found: out.files.len() as u64,
    });
    Ok(out)
}

fn is_reserved(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(RESERVED_DIR))
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(AudioFormat::from_extension)
        .is_some()
}

/// The database form of the relative path `rel`, or `None` if it would not convert
/// back to the same path: a name that is not valid Unicode, or a component that is not
/// a plain name (`..`, a root, a prefix). Such files are skipped, never stored lossily.
pub(crate) fn db_rel_path(rel: &Path) -> Option<String> {
    let plain = rel
        .components()
        .all(|c| matches!(c, Component::Normal(name) if name.to_str().is_some()));
    if !plain || rel.as_os_str().is_empty() {
        return None;
    }
    let db = rel_path_to_db(rel);
    (rel_path_from_db(&db) == rel).then_some(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_relative_paths_convert() {
        let rel: PathBuf = ["Artist", "Album", "01 Song.flac"].iter().collect();
        assert_eq!(
            db_rel_path(&rel).as_deref(),
            Some("Artist/Album/01 Song.flac")
        );
    }

    #[test]
    fn paths_that_do_not_round_trip_are_refused() {
        assert_eq!(db_rel_path(Path::new("")), None);
        assert_eq!(db_rel_path(Path::new("../a.mp3")), None);
        assert_eq!(db_rel_path(Path::new("a/./b.mp3")), Some("a/b.mp3".into()));
        let absolute: PathBuf = std::env::temp_dir().join("a.mp3");
        assert_eq!(db_rel_path(&absolute), None);
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_names_are_refused() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let rel = Path::new(OsStr::from_bytes(b"caf\xe9.mp3"));
        assert_eq!(db_rel_path(rel), None);
    }

    #[test]
    fn blocked_paths_cover_their_children_only() {
        let walk = Walk {
            blocked: vec!["a/b".into()],
            ..Walk::default()
        };
        assert!(walk.is_blocked("a/b"));
        assert!(walk.is_blocked("a/b/c.mp3"));
        assert!(!walk.is_blocked("a/bc.mp3"));
        assert!(!walk.is_blocked("a/c.mp3"));
    }
}
