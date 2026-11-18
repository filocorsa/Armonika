//! Background audio fingerprints.
//!
//! The fingerprinter computes the Chromaprint fingerprint of the audio of every present
//! track that has its content hash, 20 tracks at a time. Fingerprints are keyed by the
//! content hash, so copies of the same audio are decoded once. For each track it checks
//! that the file still has the size and modification time the scan saw, before and
//! after the computation, and writes the outcome (a fingerprint or a failure, with its
//! reason) in one short transaction. A track that cannot be fingerprinted is recorded
//! as failed and is not tried again. It emits no `LibraryChange`: fingerprints are not
//! shown to the apps.
//!
//! The work pauses for the libraries paused in its [`WorkGate`], which the scan
//! controller does while it scans them. Files are only read.

use std::collections::HashMap;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::Duration;

use armonika_core::db::Db;
use armonika_core::fingerprints::{self, FingerprintJob, FingerprintState};
use armonika_core::ids::LibraryId;
use armonika_core::libraries;
use armonika_core::paths::rel_path_from_db;
use armonika_media::fingerprint::{AudioFingerprint, FingerprintError, fingerprint};
use armonika_media::probe::mtime_ms;
use tokio_util::sync::CancellationToken;

use crate::gate::WorkGate;

/// Tracks fingerprinted per database read.
const BATCH: usize = 20;

/// How long [`run`] waits before looking for new work.
const IDLE: Duration = Duration::from_secs(2);

/// How the fingerprinter runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FingerprintOptions {
    /// The ffmpeg program that decodes the codecs the built-in decoders do not support.
    pub ffmpeg: Option<PathBuf>,
}

/// What one pass of the fingerprinter did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FingerprintReport {
    /// Tracks whose fingerprint was computed now.
    pub done: u64,
    /// Tracks recorded as failed: the file could not be fingerprinted.
    pub failed: u64,
    /// Tracks left pending because the file changed or is gone, or the database failed:
    /// they are looked at again by the next pass.
    pub skipped: u64,
}

type Compute = fn(&Path, Option<&Path>) -> Result<AudioFingerprint, FingerprintError>;

/// Fingerprints the pending tracks until `stop` is cancelled, looking for new work every
/// couple of seconds. Blocking: run it on a thread of its own.
pub fn run(db: &Db, opts: &FingerprintOptions, gate: &WorkGate, stop: &CancellationToken) {
    while !stop.is_cancelled() {
        let report = fingerprint_pending(db, opts, gate, stop);
        if report != FingerprintReport::default() {
            tracing::info!(
                done = report.done,
                failed = report.failed,
                skipped = report.skipped,
                "fingerprint pass finished"
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

/// One pass: fingerprints every track that is pending now (a track is looked at once
/// per pass), skipping the paused libraries, and returns when there is no more work or
/// `stop` is cancelled.
pub fn fingerprint_pending(
    db: &Db,
    opts: &FingerprintOptions,
    gate: &WorkGate,
    stop: &CancellationToken,
) -> FingerprintReport {
    fingerprint_pending_with(db, opts, gate, stop, fingerprint)
}

fn fingerprint_pending_with(
    db: &Db,
    opts: &FingerprintOptions,
    gate: &WorkGate,
    stop: &CancellationToken,
    compute: Compute,
) -> FingerprintReport {
    let mut report = FingerprintReport::default();
    let mut last = 0;
    let mut roots: HashMap<LibraryId, Option<PathBuf>> = HashMap::new();
    while !stop.is_cancelled() {
        let paused = gate.paused_libraries();
        let batch = match db.read_blocking(|c| fingerprints::pending(c, last, &paused, BATCH)) {
            Ok(batch) => batch,
            Err(err) => {
                tracing::warn!(error = %err, "cannot list the tracks to fingerprint");
                break;
            }
        };
        let Some(end) = batch.last().map(|j| j.key) else {
            break;
        };
        last = end;
        for job in &batch {
            if stop.is_cancelled() {
                break;
            }
            // Held until the track is done: a pause waits for it.
            let Some(_busy) = gate.enter(job.library) else {
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
            match process(db, root, job, opts, compute) {
                Ok(Step::Done) => report.done += 1,
                Ok(Step::Failed) => report.failed += 1,
                Ok(Step::Skipped) => report.skipped += 1,
                // Another track with the same audio was settled meanwhile.
                Ok(Step::Shared) => {}
                Err(err) => {
                    tracing::warn!(path = %job.rel_path, error = %err, "cannot store a fingerprint");
                    report.skipped += 1;
                }
            }
        }
    }
    report
}

enum Step {
    Done,
    Failed,
    Skipped,
    Shared,
}

/// Fingerprints one track and writes the outcome in one transaction.
fn process(
    db: &Db,
    root: &Path,
    job: &FingerprintJob,
    opts: &FingerprintOptions,
    compute: Compute,
) -> armonika_core::Result<Step> {
    // A copy of the same audio earlier in the pass may have settled it.
    if db.read_blocking(|c| fingerprints::state(c, &job.content_hash))? != FingerprintState::Pending
    {
        return Ok(Step::Shared);
    }
    let abs = root.join(rel_path_from_db(&job.rel_path));
    // A file that is gone or changed since the scan is the scan's business.
    if !still_as_scanned(&abs, job) {
        return Ok(Step::Skipped);
    }
    let computed = compute_isolated(&abs, opts.ffmpeg.as_deref(), compute);
    if !still_as_scanned(&abs, job) {
        return Ok(Step::Skipped);
    }
    db.write_blocking(|conn| {
        let tx = conn.transaction()?;
        let step = match &computed {
            Ok(fp) => {
                fingerprints::store(&tx, &job.content_hash, &fp.fingerprint, fp.decoder.as_str())?;
                Step::Done
            }
            Err(reason) => {
                tracing::warn!(path = %abs.display(), error = %reason, "cannot fingerprint file");
                fingerprints::store_failure(&tx, &job.content_hash, reason)?;
                Step::Failed
            }
        };
        tx.commit()?;
        Ok(step)
    })
}

/// Whether the file at `abs` still has the size and modification time the scan saw.
fn still_as_scanned(abs: &Path, job: &FingerprintJob) -> bool {
    fs::metadata(abs)
        .and_then(|meta| Ok(meta.len() == job.size && mtime_ms(&meta)? == job.mtime_ms))
        .unwrap_or(false)
}

/// Fingerprints one file; a panic of a decoder on a malformed file is an error of that
/// file only.
fn compute_isolated(
    path: &Path,
    ffmpeg: Option<&Path>,
    compute: Compute,
) -> Result<AudioFingerprint, String> {
    match panic::catch_unwind(AssertUnwindSafe(|| compute(path, ffmpeg))) {
        Ok(Ok(fp)) => Ok(fp),
        Ok(Err(err)) => Err(err.to_string()),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(format!(
                "the audio decoder failed on {}: {message}",
                path.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};

    use armonika_core::fingerprints::FingerprintState;
    use armonika_media::fingerprint::Decoder;

    use super::*;

    /// (the compute function is inside, the test lets it go on)
    static HOLD: (Mutex<(bool, bool)>, Condvar) = (Mutex::new((false, false)), Condvar::new());

    fn held_compute(_: &Path, _: Option<&Path>) -> Result<AudioFingerprint, FingerprintError> {
        let (lock, cv) = &HOLD;
        let mut state = lock.lock().unwrap();
        state.0 = true;
        cv.notify_all();
        while !state.1 {
            state = cv.wait(state).unwrap();
        }
        Ok(AudioFingerprint {
            fingerprint: "AQAA".to_owned(),
            decoder: Decoder::Symphonia,
        })
    }

    #[test]
    fn a_panicking_decoder_is_an_error_of_that_file() {
        fn boom(_: &Path, _: Option<&Path>) -> Result<AudioFingerprint, FingerprintError> {
            panic!("bad frame");
        }
        let err = compute_isolated(Path::new("x.mp3"), None, boom).unwrap_err();
        assert!(err.contains("bad frame"), "{err}");
    }

    #[test]
    fn scan_pauses_the_fingerprinter() {
        let data = tempfile::tempdir().unwrap();
        let music = tempfile::tempdir().unwrap();
        let db = Db::open(&data.path().join("armonika.db")).unwrap();
        let lib = db
            .write_blocking(|c| libraries::create(c, "Music", music.path(), data.path()))
            .unwrap();
        let file = music.path().join("a.mp3");
        fs::write(&file, b"audio").unwrap();
        let meta = fs::metadata(&file).unwrap();
        db.write_blocking(|c| {
            c.execute(
                "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms,
                                     tag_schema, format, added_at, updated_at, content_hash,
                                     hash_state)
                 VALUES ('t', ?1, 'present', 'a.mp3', ?2, ?3, 1, 'mp3', 0, 0, x'01', 'done')",
                rusqlite::params![
                    lib.0,
                    i64::try_from(meta.len()).unwrap(),
                    mtime_ms(&meta).unwrap()
                ],
            )?;
            Ok(())
        })
        .unwrap();

        let gate = WorkGate::new();
        let stop = CancellationToken::new();
        let worker = {
            let (db, gate, stop) = (db.clone(), gate.clone(), stop.clone());
            std::thread::spawn(move || {
                fingerprint_pending_with(
                    &db,
                    &FingerprintOptions::default(),
                    &gate,
                    &stop,
                    held_compute,
                )
            })
        };
        let (lock, cv) = &HOLD;
        {
            let mut hold = lock.lock().unwrap();
            while !hold.0 {
                hold = cv.wait(hold).unwrap();
            }
        }
        // The fingerprinter is inside the track: pause must not return before it is done.
        let paused = Arc::new(AtomicBool::new(false));
        let pauser = {
            let (gate, paused) = (gate.clone(), Arc::clone(&paused));
            std::thread::spawn(move || {
                gate.pause(lib);
                paused.store(true, Ordering::SeqCst);
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !paused.load(Ordering::SeqCst),
            "pause returned during a track"
        );
        assert!(gate.is_paused(lib));
        {
            lock.lock().unwrap().1 = true;
            cv.notify_all();
        }
        pauser.join().unwrap();
        let report = worker.join().unwrap();
        assert_eq!(report.done, 1, "{report:?}");
        assert!(paused.load(Ordering::SeqCst));
        let state = db.read_blocking(|c| fingerprints::state(c, &[1])).unwrap();
        assert!(matches!(state, FingerprintState::Done(_)));
    }
}
