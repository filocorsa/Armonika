//! Background audio fingerprints: computed after the content hash, one per audio.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use armonika_core::db::Db;
use armonika_core::fingerprints::{self, FingerprintCounts};
use armonika_core::ids::LibraryId;
use armonika_core::libraries;
use armonika_scanner::controller::ScanController;
use armonika_scanner::fingerprinter::{FingerprintOptions, fingerprint_pending};
use armonika_scanner::gate::WorkGate;
use armonika_scanner::hasher::Hasher;
use armonika_scanner::{ScanOptions, scan_library};
use tokio::sync::mpsc::unbounded_channel;
use tokio_util::sync::CancellationToken;

const TONES: [&str; 5] = ["tone.mp3", "tone.flac", "tone.ogg", "tone.opus", "tone.m4a"];

struct Lib {
    _data: tempfile::TempDir,
    music: tempfile::TempDir,
    db: Db,
    id: LibraryId,
}

impl Lib {
    fn new() -> Lib {
        let data = tempfile::tempdir().unwrap();
        let music = tempfile::tempdir().unwrap();
        let db = Db::open(&data.path().join("armonika.db")).unwrap();
        let id = db
            .write_blocking(|c| libraries::create(c, "Music", music.path(), data.path()))
            .unwrap();
        Lib {
            _data: data,
            music,
            db,
            id,
        }
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../armonika-media/tests/fixtures")
            .join(name)
    }

    fn add_tones(&self) {
        for name in TONES {
            fs::copy(Lib::fixture(name), self.music.path().join(name)).unwrap();
        }
    }

    fn scan_and_hash(&self) {
        let opts = ScanOptions {
            threads: 2,
            batch: 500,
            allow_empty: false,
        };
        scan_library(&self.db, self.id, opts, &|_| {}, &|_| {}).unwrap();
        Hasher::new().hash_pending(&self.db, &CancellationToken::new(), &|_| {});
    }

    fn counts(&self) -> FingerprintCounts {
        self.db
            .read_blocking(|c| fingerprints::counts(c, self.id))
            .unwrap()
    }

    fn run(&self, ffmpeg: Option<PathBuf>) -> armonika_scanner::fingerprinter::FingerprintReport {
        fingerprint_pending(
            &self.db,
            &FingerprintOptions { ffmpeg },
            &WorkGate::new(),
            &CancellationToken::new(),
        )
    }
}

#[test]
fn fingerprints_after_the_content_hash() {
    let lib = Lib::new();
    lib.add_tones();
    let opts = ScanOptions {
        threads: 2,
        batch: 500,
        allow_empty: false,
    };
    scan_library(&lib.db, lib.id, opts, &|_| {}, &|_| {}).unwrap();
    // No content hash yet: nothing to fingerprint.
    let report = lib.run(None);
    assert_eq!((report.done, report.failed, report.skipped), (0, 0, 0));

    Hasher::new().hash_pending(&lib.db, &CancellationToken::new(), &|_| {});
    let report = lib.run(None);
    assert_eq!(report.done, 5, "{report:?}");
    let counts = lib.counts();
    assert_eq!((counts.done, counts.failed, counts.pending), (5, 0, 0));
    // Nothing is left for a second pass.
    assert_eq!(lib.run(None), Default::default());
}

#[test]
fn too_short_file_is_recorded_as_failure() {
    let lib = Lib::new();
    // A file the content hash can read but that has too little audio to fingerprint
    // (the codec-patched m4a would not even get a content hash).
    let short = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/templates/silence.mp3");
    fs::copy(short, lib.music.path().join("short.mp3")).unwrap();
    lib.scan_and_hash();

    let report = lib.run(None);
    assert_eq!((report.done, report.failed), (0, 1), "{report:?}");
    let counts = lib.counts();
    assert_eq!((counts.done, counts.failed, counts.pending), (0, 1, 0));
    let reason: String = lib
        .db
        .read_blocking(|c| {
            Ok(
                c.query_row("SELECT reason FROM audio_fingerprint_failures", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert!(!reason.is_empty());
    // A failure is not tried again.
    assert_eq!(lib.run(None), Default::default());
}

#[test]
fn file_changed_since_the_scan_is_skipped() {
    let lib = Lib::new();
    fs::copy(Lib::fixture("tone.mp3"), lib.music.path().join("a.mp3")).unwrap();
    lib.scan_and_hash();
    File::options()
        .write(true)
        .open(lib.music.path().join("a.mp3"))
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();

    let report = lib.run(None);
    assert_eq!(
        (report.done, report.failed, report.skipped),
        (0, 0, 1),
        "{report:?}"
    );
    let counts = lib.counts();
    assert_eq!((counts.done, counts.failed, counts.pending), (0, 0, 1));
}

fn wait_until(secs: u64, what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn controller(lib: &Lib, fingerprint: Option<FingerprintOptions>) -> ScanController {
    let (tx, rx) = unbounded_channel();
    std::mem::forget(rx);
    let opts = ScanOptions {
        threads: 2,
        batch: 500,
        allow_empty: false,
    };
    ScanController::new(lib.db.clone(), opts, tx, fingerprint)
}

#[test]
fn background_fingerprinter_runs_when_enabled() {
    let lib = Lib::new();
    lib.add_tones();
    let controller = controller(&lib, Some(FingerprintOptions { ffmpeg: None }));
    controller.request(lib.id);
    wait_until(60, "the fingerprints", || lib.counts().done == 5);
    controller.shutdown();
}

#[test]
fn disabled_fingerprinter_does_nothing() {
    let lib = Lib::new();
    lib.add_tones();
    let controller = controller(&lib, None);
    controller.request(lib.id);
    wait_until(60, "the content hashes", || lib.counts().pending == 5);
    std::thread::sleep(Duration::from_millis(2500));
    let counts = lib.counts();
    assert_eq!((counts.done, counts.failed, counts.pending), (0, 0, 5));
    controller.shutdown();
}
