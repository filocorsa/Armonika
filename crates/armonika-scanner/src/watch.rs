//! Folder watching and periodic rescans: both only ask the controller for a scan.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use armonika_core::ids::LibraryId;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

use crate::controller::ScanController;

/// Events of a library are gathered for this long before they ask for a scan.
const DEBOUNCE: Duration = Duration::from_secs(2);

/// The folder of a library that holds files of the server itself: never watched.
const SERVER_DIR: &str = ".armonika";

/// Keeps the watchers and the timer alive; dropping it stops them.
pub struct WatchHandle {
    _watchers: Vec<Debouncer<notify::RecommendedWatcher, RecommendedCache>>,
    /// Dropped to stop the timer thread.
    _stop_timer: Option<mpsc::Sender<()>>,
}

/// Watches the roots of `libraries`: a change under a root (outside `.armonika`) asks
/// `controller` for a scan of that library, 2 s after the last event. A root that
/// cannot be watched (a network share, for example) is logged as a warning and left
/// to the periodic scan, which asks for a scan of every library each `interval`
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
) -> notify::Result<Debouncer<notify::RecommendedWatcher, RecommendedCache>> {
    let controller = controller.clone();
    let base = root.to_path_buf();
    let mut debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
        match result {
            Ok(events) => {
                let relevant = events.iter().any(|e| {
                    // Reading a file is not a change (inotify reports it).
                    !matches!(e.kind, EventKind::Access(_))
                        && e.paths.iter().any(|p| counts(&base, p))
                });
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
    debouncer.watch(root, RecursiveMode::Recursive)?;
    Ok(debouncer)
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
