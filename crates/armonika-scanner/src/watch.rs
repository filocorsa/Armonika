//! Folder watching and periodic rescans: both only ask the controller for a scan.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak, mpsc};
use std::time::Duration;

use armonika_core::ids::LibraryId;
use notify::event::ModifyKind;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use walkdir::WalkDir;

use crate::controller::ScanController;

/// Events of a library are gathered for this long before they ask for a scan.
const DEBOUNCE: Duration = Duration::from_secs(2);

/// The folder of a library that holds files of the server itself: never watched.
const SERVER_DIR: &str = ".armonika";

type Watcher = Debouncer<notify::RecommendedWatcher, RecommendedCache>;

/// Keeps the watchers and the timer alive; dropping it stops them.
pub struct WatchHandle {
    _watchers: Vec<Arc<Mutex<Watcher>>>,
    /// Dropped to stop the timer thread.
    _stop_timer: Option<mpsc::Sender<()>>,
}

/// Watches the roots of `libraries`: a change under a root (outside `.armonika`) asks
/// `controller` for a scan of that library, 2 s after the last event. When a folder
/// under the root cannot be watched (no read permission, such as `lost+found` at the
/// top of a volume), the root is watched folder by folder and only the unreadable
/// folders are left out, as the scan leaves them out. A root that cannot be watched at
/// all (a network share, for example) is logged as a warning and left to the periodic
/// scan, which asks for a scan of every library each `interval`
/// (never if it is zero).
pub fn spawn(
    controller: ScanController,
    libraries: Vec<(LibraryId, PathBuf)>,
    interval: Duration,
) -> WatchHandle {
    let mut watchers = Vec::new();
    for (library, root) in libraries {
        match watch_root(&controller, library, &root) {
            Ok(watcher) => watchers.push(watcher),
            Err(err) => tracing::warn!(
                library = %library,
                root = %root.display(),
                error = %err,
                "cannot watch the library folder; only periodic scans will find changes"
            ),
        }
    }
    WatchHandle {
        _watchers: watchers,
        _stop_timer: start_timer(controller, interval),
    }
}

fn watch_root(
    controller: &ScanController,
    library: LibraryId,
    root: &Path,
) -> notify::Result<Arc<Mutex<Watcher>>> {
    let controller = controller.clone();
    let base = root.to_path_buf();
    // New folders go to the thread that watches them, which only exists when the
    // root is watched folder by folder; otherwise the sends fail and are ignored.
    let (new_dirs, added) = mpsc::channel::<PathBuf>();
    let debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
        match result {
            Ok(events) => {
                let mut relevant = false;
                for event in &events {
                    // Reading a file is not a change (inotify reports it).
                    if matches!(event.kind, EventKind::Access(_)) {
                        continue;
                    }
                    let created = matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
                    );
                    for path in event.paths.iter().filter(|p| counts(&base, p)) {
                        relevant = true;
                        if created && path.is_dir() {
                            let _ = new_dirs.send(path.clone());
                        }
                    }
                }
                if relevant {
                    controller.request(library);
                }
            }
            Err(errors) => {
                for err in errors {
                    tracing::warn!(library = %library, error = %err, "folder watcher error");
                }
                // Events may have been lost: look at everything.
                controller.request(library);
            }
        }
    })?;
    let watcher = Arc::new(Mutex::new(debouncer));
    let mut guard = watcher.lock().unwrap_or_else(PoisonError::into_inner);
    let Err(err) = guard.watch(root, RecursiveMode::Recursive) else {
        drop(guard);
        return Ok(watcher);
    };
    // The recursive watch stops at the first folder it cannot read and keeps the
    // folders it reached; start again from a clean state.
    let _ = guard.unwatch(root);
    if watch_tree(&mut guard, library, root) == 0 {
        return Err(err);
    }
    drop(guard);
    tracing::info!(
        library = %library,
        root = %root.display(),
        reason = %err,
        "watching the library folder by folder"
    );
    let weak = Arc::downgrade(&watcher);
    let spawned = std::thread::Builder::new()
        .name("armonika-watch".to_owned())
        .spawn(move || watch_new_dirs(&weak, library, &added));
    if let Err(err) = spawned {
        tracing::warn!(
            library = %library,
            error = %err,
            "cannot start the thread that watches new folders; only periodic scans will find changes inside them"
        );
    }
    Ok(watcher)
}

/// Watches each folder created under a root watched folder by folder. Ends when the
/// watcher is dropped (its event handler holds the sender of `added`).
fn watch_new_dirs(
    watcher: &Weak<Mutex<Watcher>>,
    library: LibraryId,
    added: &mpsc::Receiver<PathBuf>,
) {
    while let Ok(dir) = added.recv() {
        let Some(watcher) = watcher.upgrade() else {
            return;
        };
        let mut guard = watcher.lock().unwrap_or_else(PoisonError::into_inner);
        watch_tree(&mut guard, library, &dir);
    }
}

/// Watches `dir` and every folder under it one by one, skipping the server folder and
/// the folders that cannot be read or watched (each one logged). Returns how many
/// folders are watched.
fn watch_tree(watcher: &mut Watcher, library: LibraryId, dir: &Path) -> usize {
    let mut watched = 0;
    let mut entries = WalkDir::new(dir).follow_links(true).into_iter();
    while let Some(entry) = entries.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!(library = %library, error = %err, "cannot watch a folder of the library");
                continue;
            }
        };
        if !entry.file_type().is_dir() {
            continue;
        }
        if entry.file_name() == SERVER_DIR && entry.depth() > 0 {
            entries.skip_current_dir();
            continue;
        }
        match watcher.watch(entry.path(), RecursiveMode::NonRecursive) {
            Ok(()) => watched += 1,
            Err(err) => {
                tracing::warn!(
                    library = %library,
                    folder = %entry.path().display(),
                    error = %err,
                    "cannot watch a folder of the library"
                );
                entries.skip_current_dir();
            }
        }
    }
    watched
}

/// Whether a change at `path` is one the scan could care about: not inside the
/// server's own folder at the top of the library. Events only come from the watched
/// root; if a platform spells the path differently from `root` (for example with or
/// without the `\?\` prefix on Windows) the change counts.
fn counts(root: &Path, path: &Path) -> bool {
    match path.strip_prefix(root) {
        Ok(rel) => rel.components().next() != Some(Component::Normal(SERVER_DIR.as_ref())),
        Err(_) => !path
            .components()
            .any(|c| c == Component::Normal(SERVER_DIR.as_ref())),
    }
}

fn start_timer(controller: ScanController, interval: Duration) -> Option<mpsc::Sender<()>> {
    if interval.is_zero() {
        return None;
    }
    let (stop, stopped) = mpsc::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name("armonika-rescan".to_owned())
        .spawn(move || {
            // Only a timeout is a tick; the sender being dropped ends the thread.
            while let Err(mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(interval) {
                controller.request_all();
            }
        });
    match spawned {
        Ok(_) => Some(stop),
        Err(err) => {
            tracing::error!(error = %err, "cannot start the periodic scan timer");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_folder_is_ignored() {
        let root = Path::new("/music");
        assert!(counts(root, Path::new("/music/a.flac")));
        assert!(counts(root, Path::new("/music/x/.armonika/y")));
        assert!(!counts(root, Path::new("/music/.armonika/cache/y")));
        // A different spelling of the root still counts, unless it is the server folder.
        assert!(counts(root, Path::new("/elsewhere/a.flac")));
        assert!(!counts(root, Path::new("/elsewhere/.armonika/a")));
    }
}
