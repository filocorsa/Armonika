//! The per-library pause that the background workers (content hasher, fingerprinter)
//! share with the scan controller.
//!
//! A worker calls [`WorkGate::enter`] before it touches a track and keeps the returned
//! guard until it is done with it. The controller calls [`WorkGate::pause`] before it
//! scans a library: the call returns when the worker is no longer on a track of that
//! library, and the worker takes no new one until [`WorkGate::resume`].

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use armonika_core::ids::LibraryId;

/// What a worker and the scan controller share.
#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    /// Signalled when the worker finishes a track.
    idle: Condvar,
}

#[derive(Debug, Default)]
struct State {
    paused: HashSet<LibraryId>,
    /// The library of the track being worked on now.
    busy: Option<LibraryId>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A set of ids stays valid whatever a panicking holder was doing.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Pauses and resumes the work of one background worker on a library, from another
/// thread. Cloning gives another handle to the same gate.
#[derive(Clone, Debug, Default)]
pub struct WorkGate {
    shared: Arc<Shared>,
}

impl WorkGate {
    pub fn new() -> WorkGate {
        WorkGate::default()
    }

    /// No track of `library` is worked on until [`WorkGate::resume`]. A track being
    /// worked on finishes first: this call waits for it, so when it returns the worker
    /// is not touching the library. Do not call it from the worker's own thread.
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

    /// Work on `library` goes on.
    pub fn resume(&self, library: LibraryId) {
        self.shared.lock().paused.remove(&library);
    }

    pub fn is_paused(&self, library: LibraryId) -> bool {
        self.shared.lock().paused.contains(&library)
    }

    /// The libraries that are paused now.
    pub fn paused_libraries(&self) -> Vec<LibraryId> {
        self.shared.lock().paused.iter().copied().collect()
    }

    /// Marks `library` busy until the guard is dropped. `None` if it is paused.
    pub fn enter(&self, library: LibraryId) -> Option<Busy<'_>> {
        let mut state = self.shared.lock();
        if state.paused.contains(&library) {
            return None;
        }
        state.busy = Some(library);
        Some(Busy(&self.shared))
    }
}

/// Marks a library busy for as long as it lives, also when the work panics.
#[derive(Debug)]
pub struct Busy<'a>(&'a Shared);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.lock().busy = None;
        self.0.idle.notify_all();
    }
}
