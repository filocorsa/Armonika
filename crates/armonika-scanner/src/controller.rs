//! The scan controller: at most one scan per library, requests that arrive during a
//! scan merge into one more scan, and the content hasher and the fingerprinter run in
//! the background and wait while a library is scanned.

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use armonika_core::db::Db;
use armonika_core::ids::LibraryId;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::fingerprinter::{self, FingerprintOptions};
use crate::gate::WorkGate;
use crate::hasher::Hasher;
use crate::{LibraryChange, ScanError, ScanOptions, ScanProgress, ScanReport, scan_library_until};

/// Where the scans of one library are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryScanState {
    /// A scan is running now.
    pub running: bool,
    /// Another scan will start when the running one ends.
    pub queued: bool,
    /// Audio files found by the running scan so far, or by the last one.
    pub files_seen: u64,
    /// Scans finished since the controller started, failed ones included.
    pub scans_finished: u64,
    /// What the last scan that succeeded did.
    pub last_report: Option<ScanReport>,
    /// Why the last scan failed; cleared by a scan that succeeds.
    pub last_error: Option<String>,
}

/// What the controller is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanStatus {
    /// A scan is running.
    pub scanning: bool,
    /// Every library that was ever requested, by identifier.
    pub libraries: Vec<(LibraryId, LibraryScanState)>,
}

struct Inner {
    db: Db,
    opts: ScanOptions,
    changes: UnboundedSender<LibraryChange>,
    hasher: WorkGate,
    /// `None` when the fingerprinter is off.
    fingerprinter: Option<WorkGate>,
    stop: CancellationToken,
    states: Mutex<BTreeMap<LibraryId, LibraryScanState>>,
}

impl Inner {
    fn states(&self) -> MutexGuard<'_, BTreeMap<LibraryId, LibraryScanState>> {
        // Plain data: valid whatever a panicking holder was doing.
        self.states.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, library: LibraryId, f: impl FnOnce(&mut LibraryScanState)) {
        f(self.states().entry(library).or_default());
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // The background threads hold no reference to the controller: tell them to end.
        self.stop.cancel();
    }
}

/// Runs scans on request. Cloning gives another handle to the same controller; when
/// the last handle (and the last running scan) is gone the background threads stop.
#[derive(Clone)]
pub struct ScanController {
    inner: Arc<Inner>,
}

impl ScanController {
    /// A controller that writes to `db` and sends what every committed batch changed
    /// to `changes`. It starts the content hasher on a thread of its own, at low
    /// priority, and, unless `fingerprint` is `None`, the fingerprinter on another.
    pub fn new(
        db: Db,
        opts: ScanOptions,
        changes: UnboundedSender<LibraryChange>,
        fingerprint: Option<FingerprintOptions>,
    ) -> ScanController {
        let hasher = Hasher::new();
        let fingerprint_gate = fingerprint.as_ref().map(|_| WorkGate::new());
        let stop = CancellationToken::new();
        let inner = Arc::new(Inner {
            db: db.clone(),
            opts,
            changes: changes.clone(),
            hasher: hasher.handle(),
            fingerprinter: fingerprint_gate.clone(),
            stop: stop.clone(),
            states: Mutex::default(),
        });
        let (hasher_db, hasher_stop) = (db.clone(), stop.clone());
        let spawned = std::thread::Builder::new()
            .name("armonika-hasher".to_owned())
            .spawn(move || {
                lower_priority("hasher");
                let ran = panic::catch_unwind(AssertUnwindSafe(|| {
                    hasher.run(hasher_db, hasher_stop, move |change| {
                        let _ = changes.send(change);
                    });
                }));
                if ran.is_err() {
                    tracing::error!("the content hasher panicked and stopped");
                }
            });
        if let Err(err) = spawned {
            tracing::error!(error = %err, "cannot start the content hasher");
        }
        if let (Some(opts), Some(gate)) = (fingerprint, fingerprint_gate) {
            let spawned = std::thread::Builder::new()
                .name("armonika-fingerprint".to_owned())
                .spawn(move || {
                    lower_priority("fingerprinter");
                    let ran = panic::catch_unwind(AssertUnwindSafe(|| {
                        fingerprinter::run(&db, &opts, &gate, &stop);
                    }));
                    if ran.is_err() {
                        tracing::error!("the fingerprinter panicked and stopped");
                    }
                });
            if let Err(err) = spawned {
                tracing::error!(error = %err, "cannot start the fingerprinter");
            }
        }
        ScanController { inner }
    }

    /// Asks for a scan of `library`. If one is running, one more scan starts after it,
    /// however many requests arrive meanwhile.
    pub fn request(&self, library: LibraryId) {
        if self.inner.stop.is_cancelled() {
            return;
        }
        {
            let mut states = self.inner.states();
            let state = states.entry(library).or_default();
            if state.running {
                state.queued = true;
                return;
            }
            state.running = true;
        }
        let inner = Arc::clone(&self.inner);
        let spawned = std::thread::Builder::new()
            .name("armonika-scan".to_owned())
            .spawn(move || run_scans(&inner, library));
        if let Err(err) = spawned {
            tracing::error!(error = %err, "cannot start a scan");
            self.inner.update(library, |s| {
                s.running = false;
                s.last_error = Some(format!("cannot start a scan: {err}"));
            });
        }
    }

    /// Asks for a scan of every library the database has.
    pub fn request_all(&self) {
        match self.inner.db.read_blocking(armonika_core::libraries::list) {
            Ok(list) => list.into_iter().for_each(|l| self.request(l.id)),
            Err(err) => tracing::warn!(error = %err, "cannot list the libraries to scan"),
        }
    }

    pub fn status(&self) -> ScanStatus {
        let states = self.inner.states();
        ScanStatus {
            scanning: states.values().any(|s| s.running),
            libraries: states.iter().map(|(id, s)| (*id, s.clone())).collect(),
        }
    }

    /// Whether the content hasher is paused for `library`, which is so while a scan of
    /// it runs.
    pub fn is_hasher_paused(&self, library: LibraryId) -> bool {
        self.inner.hasher.is_paused(library)
    }

    /// Stops the content hasher and the scans: a running scan ends after the batch it
    /// is writing, and no scan starts any more.
    pub fn shutdown(&self) {
        self.inner.stop.cancel();
    }
}

/// Scans `library` and, while requests were made meanwhile, once more.
fn run_scans(inner: &Inner, library: LibraryId) {
    loop {
        inner.update(library, |s| s.files_seen = 0);
        // Waits for the hasher to finish the track it is on, so that the two never
        // work on the same library; resumed also if the scan panics.
        // Then for the fingerprinter, which needs the hash the hasher writes.
        inner.hasher.pause(library);
        if let Some(fingerprinter) = &inner.fingerprinter {
            fingerprinter.pause(library);
        }
        let outcome = {
            let _resume = Resume(&inner.hasher, inner.fingerprinter.as_ref(), library);
            panic::catch_unwind(AssertUnwindSafe(|| {
                scan_library_until(
                    &inner.db,
                    library,
                    inner.opts,
                    &|progress| {
                        if let ScanProgress::Walking { found } = progress {
                            inner.update(library, |s| s.files_seen = found);
                        }
                    },
                    &|change| {
                        let _ = inner.changes.send(change);
                    },
                    &|| inner.stop.is_cancelled(),
                )
            }))
        };
        let more = {
            let mut states = inner.states();
            let state = states.entry(library).or_default();
            state.scans_finished += 1;
            match outcome {
                Ok(Ok(report)) => {
                    state.files_seen = report.seen;
                    state.last_report = Some(report);
                    state.last_error = None;
                }
                Ok(Err(ScanError::Cancelled)) => {
                    tracing::info!(library = %library, "scan stopped by shutdown");
                }
                Ok(Err(err)) => {
                    tracing::warn!(library = %library, error = %err, "scan failed");
                    state.last_error = Some(err.to_string());
                }
                Err(_) => {
                    tracing::error!(library = %library, "scan panicked");
                    state.last_error = Some("the scan panicked".to_owned());
                }
            }
            // A request made before this point is served by the scan that follows.
            if state.queued && !inner.stop.is_cancelled() {
                state.queued = false;
                true
            } else {
                state.queued = false;
                state.running = false;
                false
            }
        };
        if !more {
            return;
        }
    }
}

/// Resumes the background workers, the fingerprinter first (the reverse of the order
/// they were paused in).
struct Resume<'a>(&'a WorkGate, Option<&'a WorkGate>, LibraryId);

impl Drop for Resume<'_> {
    fn drop(&mut self) {
        if let Some(fingerprinter) = self.1 {
            fingerprinter.resume(self.2);
        }
        self.0.resume(self.2);
    }
}

/// The hasher and the fingerprinter are background work: they must not slow the scans or
/// playback. A failure only costs speed, so it is logged at debug level. The crate is
/// safe code only (`unsafe` is forbidden here); the platform calls are inside
/// `thread-priority`.
fn lower_priority(worker: &str) {
    use thread_priority::{ThreadPriority, set_current_thread_priority};
    if let Err(err) = set_current_thread_priority(ThreadPriority::Min) {
        tracing::debug!(error = ?err, worker, "cannot lower the priority of a background thread");
    }
}
