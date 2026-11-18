//! Background audio fingerprints.
//!
//! The fingerprinter computes the Chromaprint fingerprint of the audio of every present
//! track that has its content hash, 20 tracks at a time. Fingerprints are keyed by the
//! content hash, so copies of the same audio are decoded once. For each track it checks
//! that the file still has the size and modification time the scan saw, before and
//! after the computation, and writes the outcome (a fingerprint or a failure, with its
//! reason) in one short transaction. A track that cannot be fingerprinted because of
//! its audio is recorded as failed and is not tried again; one that could not be read,
//! or for which ffmpeg could not be started, stays pending and is tried again after a
//! growing delay. It emits no `LibraryChange`: fingerprints are not shown to the apps.
//!
//! When a pass finds nothing to do the wait before the next one grows, up to five
//! minutes, so an idle server costs nothing; the hasher and the scan controller nudge
//! the fingerprinter when new work can have appeared.
//!
//! The work pauses for the libraries paused in its [`WorkGate`], which the scan
//! controller does while it scans them. Files are only read.

use std::collections::HashMap;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use armonika_core::db::Db;
use armonika_core::fingerprints::{self, FingerprintJob, FingerprintState};
use armonika_core::ids::LibraryId;
use armonika_core::libraries;
use armonika_core::paths::rel_path_from_db;
use armonika_media::fingerprint::{AudioFingerprint, FingerprintError, fingerprint};
use armonika_media::probe::mtime_ms;
use tokio_util::sync::CancellationToken;

use crate::gate::WorkGate;

/// The ffmpeg program to use: the configured one, or `ffmpeg` from `PATH`.
pub use armonika_media::fingerprint::ffmpeg_or_default;

/// Tracks fingerprinted per database read.
const BATCH: usize = 20;

/// The wait between passes while there is work, and where the idle wait starts.
const IDLE: Duration = Duration::from_secs(2);

/// The longest idle wait.
const IDLE_MAX: Duration = Duration::from_secs(300);

/// The first delay before a track that could not be read is tried again, and the
/// longest.
const RETRY_FIRST: Duration = Duration::from_secs(60);
const RETRY_MAX: Duration = Duration::from_secs(3600);

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
    /// Tracks recorded as failed: the audio could not be fingerprinted.
    pub failed: u64,
    /// Tracks left pending because the file changed or is gone, could not be read, or
    /// the database failed: they are looked at again by a later pass.
    pub skipped: u64,
    /// Of the skipped, the tracks that need ffmpeg and could not start it.
    pub waiting_for_ffmpeg: u64,
}

/// Wakes a fingerprinter that waits between passes: new work may have appeared.
#[derive(Clone, Debug, Default)]
pub struct Nudge(Arc<AtomicBool>);

impl Nudge {
    pub fn new() -> Nudge {
        Nudge::default()
    }

    pub fn nudge(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn take(&self) -> bool {
        self.0.swap(false, Ordering::SeqCst)
    }
}

/// The wait between passes: short while there is work, doubling while there is none.
#[derive(Debug)]
struct Backoff {
    wait: Duration,
}

impl Backoff {
    fn new() -> Backoff {
        Backoff { wait: IDLE }
    }

    /// The wait after a pass that did (`worked`) or did not do any work.
    fn after_pass(&mut self, worked: bool) -> Duration {
        let wait = self.wait;
        self.wait = if worked {
            IDLE
        } else {
            (self.wait * 2).min(IDLE_MAX)
        };
        if worked { IDLE } else { wait }
    }

    fn reset(&mut self) {
        self.wait = IDLE;
    }
}

/// When a track that could not be read may be tried again.
#[derive(Debug, Default)]
struct Retries {
    by_hash: HashMap<Vec<u8>, (Instant, Duration)>,
}

impl Retries {
    fn is_waiting(&self, hash: &[u8], now: Instant) -> bool {
        self.by_hash.get(hash).is_some_and(|(at, _)| now < *at)
    }

    fn failed(&mut self, hash: &[u8], now: Instant) {
        let delay = self
            .by_hash
            .get(hash)
            .map_or(RETRY_FIRST, |(_, last)| (*last * 2).min(RETRY_MAX));
        self.by_hash.insert(hash.to_vec(), (now + delay, delay));
    }

    fn settled(&mut self, hash: &[u8]) {
        self.by_hash.remove(hash);
    }
}

type Compute = fn(&Path, Option<&Path>) -> Result<AudioFingerprint, FingerprintError>;

/// Fingerprints the pending tracks until `stop` is cancelled. Blocking: run it on a
/// thread of its own.
pub fn run(
    db: &Db,
    opts: &FingerprintOptions,
    gate: &WorkGate,
    stop: &CancellationToken,
    nudge: &Nudge,
) {
    let mut backoff = Backoff::new();
    let mut retries = Retries::default();
    while !stop.is_cancelled() {
        let report = pass(
            db,
            opts,
            gate,
            stop,
            &mut retries,
            Instant::now(),
            fingerprint,
        );
        if report.done + report.failed > 0 {
            tracing::info!(
                done = report.done,
                failed = report.failed,
                skipped = report.skipped,
                "fingerprint pass finished"
            );
        } else if report.skipped > 0 {
            tracing::debug!(skipped = report.skipped, "fingerprint pass only skipped");
        }
        let wait = backoff.after_pass(report.done + report.failed > 0);
        if sleep(wait, stop, nudge) {
            backoff.reset();
        }
    }
}

/// Waits `wait`, in short slices so that a stop is heard at once. True if it was
/// nudged, which ends the wait.
fn sleep(wait: Duration, stop: &CancellationToken, nudge: &Nudge) -> bool {
    let mut waited = Duration::ZERO;
    while waited < wait && !stop.is_cancelled() {
        if nudge.take() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
        waited += Duration::from_millis(100);
    }
    false
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
    pass(
        db,
        opts,
        gate,
        stop,
        &mut Retries::default(),
        Instant::now(),
        fingerprint,
    )
}

fn pass(
    db: &Db,
    opts: &FingerprintOptions,
    gate: &WorkGate,
    stop: &CancellationToken,
    retries: &mut Retries,
    now: Instant,
    compute: Compute,
) -> FingerprintReport {
    let mut report = FingerprintReport::default();
    let mut ffmpeg_problem: Option<String> = None;
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
            match process(db, root, job, opts, retries, now, compute) {
                Ok(Step::Done) => report.done += 1,
                Ok(Step::Failed) => report.failed += 1,
                Ok(Step::Skipped) => report.skipped += 1,
                Ok(Step::Unreadable { ffmpeg }) => {
                    report.skipped += 1;
                    if let Some(reason) = ffmpeg {
                        report.waiting_for_ffmpeg += 1;
                        ffmpeg_problem.get_or_insert(reason);
                    }
                }
                // Another track with the same audio was settled meanwhile.
                Ok(Step::Shared) => {}
                Err(err) => {
                    tracing::warn!(path = %job.rel_path, error = %err, "cannot store a fingerprint");
                    report.skipped += 1;
                }
            }
        }
    }
    if let Some(reason) = ffmpeg_problem {
        let program = opts.ffmpeg.as_deref().map(Path::display);
        tracing::warn!(
            tracks = report.waiting_for_ffmpeg,
            ffmpeg = %program.map(|p| p.to_string()).unwrap_or_default(),
            error = %reason,
            "tracks wait for ffmpeg: it cannot be started"
        );
    }
    report
}

enum Step {
    Done,
    Failed,
    Skipped,
    /// The file could not be read; `ffmpeg` is the reason when ffmpeg could not be
    /// started. The track stays pending.
    Unreadable {
        ffmpeg: Option<String>,
    },
    Shared,
}

/// Fingerprints one track and writes the outcome in one transaction.
fn process(
    db: &Db,
    root: &Path,
    job: &FingerprintJob,
    opts: &FingerprintOptions,
    retries: &mut Retries,
    now: Instant,
    compute: Compute,
) -> armonika_core::Result<Step> {
    // A copy of the same audio earlier in the pass may have settled it.
    if db.read_blocking(|c| fingerprints::state(c, &job.content_hash))? != FingerprintState::Pending
    {
        return Ok(Step::Shared);
    }
    // A file that could not be read is left alone until its delay is over.
    if retries.is_waiting(&job.content_hash, now) {
        return Ok(Step::Skipped);
    }
    let abs = root.join(rel_path_from_db(&job.rel_path));
    // A file that is gone or changed since the scan is the scan's business.
    if !still_as_scanned(&abs, job) {
        return Ok(Step::Skipped);
    }
    let computed = compute_isolated(&abs, opts.ffmpeg.as_deref(), compute);
    // An I/O error (a share that hiccups, a file in use, ffmpeg that cannot be started)
    // says nothing about the audio: the track stays pending for a later pass.
    if let Err(Failure::Transient { reason, ffmpeg }) = &computed {
        tracing::debug!(path = %abs.display(), error = %reason, "cannot read file to fingerprint it");
        retries.failed(&job.content_hash, now);
        return Ok(Step::Unreadable {
            ffmpeg: ffmpeg.then(|| reason.clone()),
        });
    }
    if !still_as_scanned(&abs, job) {
        return Ok(Step::Skipped);
    }
    let step = db.write_blocking(|conn| {
        let tx = conn.transaction()?;
        let step = match &computed {
            Ok(fp) => {
                fingerprints::store(&tx, &job.content_hash, &fp.fingerprint, fp.decoder.as_str())?;
                Step::Done
            }
            Err(Failure::Audio(reason)) => {
                tracing::warn!(path = %abs.display(), error = %reason, "cannot fingerprint file");
                fingerprints::store_failure(&tx, &job.content_hash, reason)?;
                Step::Failed
            }
            // Handled above: never stored.
            Err(Failure::Transient { .. }) => return Ok(Step::Skipped),
        };
        tx.commit()?;
        Ok(step)
    })?;
    retries.settled(&job.content_hash);
    Ok(step)
}

/// Whether the file at `abs` still has the size and modification time the scan saw.
fn still_as_scanned(abs: &Path, job: &FingerprintJob) -> bool {
    fs::metadata(abs)
        .and_then(|meta| Ok(meta.len() == job.size && mtime_ms(&meta)? == job.mtime_ms))
        .unwrap_or(false)
}

/// Why a file gave no fingerprint. The reasons name no path: they are stored for the
/// audio, which several files can share.
enum Failure {
    /// The audio itself: stored, the file is not tried again.
    Audio(String),
    /// A read or process error that may pass: not stored. `ffmpeg` is set when the
    /// error is ffmpeg failing to start.
    Transient { reason: String, ffmpeg: bool },
}

/// Fingerprints one file; a panic of a decoder on a malformed file is an error of that
/// file only.
fn compute_isolated(
    path: &Path,
    ffmpeg: Option<&Path>,
    compute: Compute,
) -> Result<AudioFingerprint, Failure> {
    match panic::catch_unwind(AssertUnwindSafe(|| compute(path, ffmpeg))) {
        Ok(Ok(fp)) => Ok(fp),
        Ok(Err(err)) if err.is_transient() => {
            let ffmpeg = matches!(
                (&err, ffmpeg),
                (FingerprintError::Io { path: failed, .. }, Some(program)) if failed == program
            );
            Err(Failure::Transient {
                reason: err.reason(),
                ffmpeg,
            })
        }
        Ok(Err(err)) => Err(Failure::Audio(err.reason())),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(Failure::Audio(format!(
                "the audio decoder failed: {message}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Condvar, Mutex};

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

    /// A database with one present, hashed track (hash 0x01) whose file matches its row.
    struct One {
        _data: tempfile::TempDir,
        _music: tempfile::TempDir,
        db: Db,
        lib: LibraryId,
    }

    fn one_track() -> One {
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
        One {
            _data: data,
            _music: music,
            db,
            lib,
        }
    }

    fn run_pass(
        one: &One,
        opts: &FingerprintOptions,
        retries: &mut Retries,
        now: Instant,
        compute: Compute,
    ) -> FingerprintReport {
        pass(
            &one.db,
            opts,
            &WorkGate::new(),
            &CancellationToken::new(),
            retries,
            now,
            compute,
        )
    }

    fn state(one: &One) -> FingerprintState {
        one.db
            .read_blocking(|c| fingerprints::state(c, &[1]))
            .unwrap()
    }

    #[test]
    fn a_panicking_decoder_is_an_error_of_that_file() {
        fn boom(_: &Path, _: Option<&Path>) -> Result<AudioFingerprint, FingerprintError> {
            panic!("bad frame");
        }
        let Err(Failure::Audio(err)) = compute_isolated(Path::new("x.mp3"), None, boom) else {
            panic!("a panic is an error of the audio");
        };
        assert!(err.contains("bad frame"), "{err}");
        assert!(!err.contains("x.mp3"), "{err}");
    }

    fn io_failure(path: &Path, _: Option<&Path>) -> Result<AudioFingerprint, FingerprintError> {
        Err(FingerprintError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other("share went away"),
        })
    }

    #[test]
    fn an_io_error_leaves_the_track_pending() {
        let one = one_track();
        let report = run_pass(
            &one,
            &FingerprintOptions::default(),
            &mut Retries::default(),
            Instant::now(),
            io_failure,
        );
        assert_eq!(
            (report.done, report.failed, report.skipped),
            (0, 0, 1),
            "{report:?}"
        );
        assert_eq!(report.waiting_for_ffmpeg, 0);
        assert_eq!(state(&one), FingerprintState::Pending);
    }

    #[test]
    fn a_stored_failure_names_no_path() {
        fn bad(path: &Path, _: Option<&Path>) -> Result<AudioFingerprint, FingerprintError> {
            Err(FingerprintError::Decode {
                path: path.to_path_buf(),
                message: "bad frame".to_owned(),
            })
        }
        let one = one_track();
        let report = run_pass(
            &one,
            &FingerprintOptions::default(),
            &mut Retries::default(),
            Instant::now(),
            bad,
        );
        assert_eq!(report.failed, 1, "{report:?}");
        let FingerprintState::Failed(reason) = state(&one) else {
            panic!("the failure is stored");
        };
        assert!(reason.contains("bad frame"), "{reason}");
        assert!(!reason.contains("a.mp3"), "{reason}");
    }

    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counted_io_failure(
        path: &Path,
        ffmpeg: Option<&Path>,
    ) -> Result<AudioFingerprint, FingerprintError> {
        CALLS.fetch_add(1, Ordering::SeqCst);
        io_failure(path, ffmpeg)
    }

    #[test]
    fn a_transient_skip_is_not_retried_before_its_delay() {
        let one = one_track();
        let opts = FingerprintOptions::default();
        let mut retries = Retries::default();
        let t0 = Instant::now();
        let calls = || CALLS.load(Ordering::SeqCst);

        run_pass(&one, &opts, &mut retries, t0, counted_io_failure);
        assert_eq!(calls(), 1);
        // Within the first delay the file is not touched again.
        let report = run_pass(
            &one,
            &opts,
            &mut retries,
            t0 + Duration::from_secs(30),
            counted_io_failure,
        );
        assert_eq!((calls(), report.skipped), (1, 1));
        // After it, once more; the next delay is twice as long.
        run_pass(
            &one,
            &opts,
            &mut retries,
            t0 + Duration::from_secs(61),
            counted_io_failure,
        );
        assert_eq!(calls(), 2);
        run_pass(
            &one,
            &opts,
            &mut retries,
            t0 + Duration::from_secs(61 + 100),
            counted_io_failure,
        );
        assert_eq!(calls(), 2);
        run_pass(
            &one,
            &opts,
            &mut retries,
            t0 + Duration::from_secs(61 + 121),
            counted_io_failure,
        );
        assert_eq!(calls(), 3);
        assert_eq!(state(&one), FingerprintState::Pending);
    }

    #[test]
    fn retry_delays_double_up_to_a_cap() {
        let mut retries = Retries::default();
        let t0 = Instant::now();
        let mut delays = Vec::new();
        for _ in 0..8 {
            retries.failed(b"h", t0);
            delays.push(retries.by_hash[&b"h"[..]].1);
        }
        assert_eq!(delays[0], RETRY_FIRST);
        assert_eq!(delays[1], RETRY_FIRST * 2);
        assert_eq!(*delays.last().unwrap(), RETRY_MAX);
        retries.settled(b"h");
        assert!(!retries.is_waiting(b"h", t0));
    }

    #[test]
    fn a_missing_ffmpeg_is_counted_apart() {
        let one = one_track();
        let opts = FingerprintOptions {
            ffmpeg: Some(PathBuf::from("no-such-ffmpeg")),
        };
        // The error names the program, as the media crate reports a failed start.
        fn no_ffmpeg(
            _: &Path,
            ffmpeg: Option<&Path>,
        ) -> Result<AudioFingerprint, FingerprintError> {
            Err(FingerprintError::Io {
                path: ffmpeg.unwrap().to_path_buf(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            })
        }
        let report = run_pass(
            &one,
            &opts,
            &mut Retries::default(),
            Instant::now(),
            no_ffmpeg,
        );
        assert_eq!(
            (report.skipped, report.waiting_for_ffmpeg),
            (1, 1),
            "{report:?}"
        );
        assert_eq!(state(&one), FingerprintState::Pending);
    }

    #[test]
    fn an_idle_pass_grows_the_wait_and_work_resets_it() {
        let mut backoff = Backoff::new();
        let waits: Vec<u64> = (0..10)
            .map(|_| backoff.after_pass(false).as_secs())
            .collect();
        assert_eq!(waits, [2, 4, 8, 16, 32, 64, 128, 256, 300, 300]);
        assert_eq!(backoff.after_pass(true), IDLE);
        assert_eq!(backoff.after_pass(false), IDLE);
        backoff.after_pass(false);
        backoff.reset();
        assert_eq!(backoff.after_pass(false), IDLE);
    }

    #[test]
    fn a_nudge_ends_the_wait() {
        let stop = CancellationToken::new();
        let nudge = Nudge::new();
        let started = Instant::now();
        nudge.nudge();
        assert!(sleep(Duration::from_secs(30), &stop, &nudge));
        assert!(started.elapsed() < Duration::from_secs(5));
        // The nudge was taken; a stop also ends the wait, without a nudge.
        stop.cancel();
        assert!(!sleep(Duration::from_secs(30), &stop, &nudge));
    }

    #[test]
    fn scan_pauses_the_fingerprinter() {
        let one = one_track();
        let (db, lib) = (one.db.clone(), one.lib);
        let gate = WorkGate::new();
        let stop = CancellationToken::new();
        let worker = {
            let (db, gate, stop) = (db.clone(), gate.clone(), stop.clone());
            std::thread::spawn(move || {
                pass(
                    &db,
                    &FingerprintOptions::default(),
                    &gate,
                    &stop,
                    &mut Retries::default(),
                    Instant::now(),
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
        assert!(matches!(state(&one), FingerprintState::Done(_)));
    }
}
