//! Scan controller: coalescing of requests, status, folder watching, periodic rescans
//! and the hasher pausing during a scan.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use armonika_core::db::Db;
use armonika_core::ids::LibraryId;
use armonika_core::libraries;
use armonika_scanner::controller::{LibraryScanState, ScanController};
use armonika_scanner::watch;
use armonika_scanner::{LibraryChange, ScanOptions};
use armonika_testdata::AudioFormat;
use armonika_testdata::fixture::{FixtureTags, tagged_file};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

struct Env {
    _data: tempfile::TempDir,
    music: tempfile::TempDir,
    db: Db,
    id: LibraryId,
    controller: ScanController,
    _changes: UnboundedReceiver<LibraryChange>,
}

impl Env {
    fn new() -> Env {
        let data = tempfile::tempdir().unwrap();
        let music = tempfile::tempdir().unwrap();
        let db = Db::open(&data.path().join("armonika.db")).unwrap();
        let id = db
            .write_blocking(|c| libraries::create(c, "Music", music.path(), data.path()))
            .unwrap();
        let (tx, rx) = unbounded_channel();
        let opts = ScanOptions {
            threads: 2,
            batch: 500,
        };
        let controller = ScanController::new(db.clone(), opts, tx);
        Env {
            _data: data,
            music,
            db,
            id,
            controller,
            _changes: rx,
        }
    }

    fn add(&self, name: &str) {
        add_song(self.music.path(), name);
    }

    fn state(&self) -> LibraryScanState {
        self.controller
            .status()
            .libraries
            .into_iter()
            .find(|(id, _)| *id == self.id)
            .map(|(_, s)| s)
            .unwrap_or_default()
    }

    fn tracks(&self) -> i64 {
        self.db
            .read_blocking(|c| Ok(c.query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))?))
            .unwrap()
    }

    /// Holds the database writer until the returned sender is used or dropped: a scan
    /// with something to write stays running meanwhile.
    fn block_writer(&self) -> mpsc::Sender<()> {
        let (release, wait) = mpsc::channel::<()>();
        let (held, is_held) = mpsc::channel::<()>();
        let db = self.db.clone();
        std::thread::spawn(move || {
            db.write_blocking(|_| {
                held.send(()).unwrap();
                let _ = wait.recv();
                Ok(())
            })
            .unwrap();
        });
        is_held.recv_timeout(Duration::from_secs(10)).unwrap();
        release
    }
}

fn add_song(dir: &Path, name: &str) {
    let tags = FixtureTags {
        title: Some(name.to_owned()),
        ..FixtureTags::default()
    };
    tagged_file(dir, &format!("{name}.mp3"), AudioFormat::Mp3, &tags);
}

/// Polls `cond` until it is true; fails after `secs` seconds.
fn wait_until(secs: u64, what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn concurrent_requests_coalesce() {
    let env = Env::new();
    env.add("one");
    let release = env.block_writer();
    env.controller.request(env.id);
    wait_until(10, "the scan to run", || env.state().running);
    for _ in 0..10 {
        env.controller.request(env.id);
    }
    assert!(env.state().queued);
    drop(release);
    wait_until(30, "the scans to end", || {
        !env.controller.status().scanning && env.state().scans_finished >= 1
    });
    let state = env.state();
    assert_eq!(state.scans_finished, 2, "{state:?}");
    assert!(!state.queued);
    assert_eq!(env.tracks(), 1);
}

#[test]
fn status_reports_running_scan() {
    let env = Env::new();
    assert!(!env.controller.status().scanning);
    env.add("one");
    env.add("two");
    let release = env.block_writer();
    env.controller.request(env.id);
    wait_until(10, "the walk to find the files", || {
        env.state().files_seen == 2
    });
    let status = env.controller.status();
    assert!(status.scanning);
    assert!(env.state().running);
    assert_eq!(env.state().last_report, None);
    drop(release);
    wait_until(30, "the scan to end", || !env.controller.status().scanning);
    let state = env.state();
    assert_eq!(state.last_error, None);
    let report = state.last_report.expect("a report");
    assert_eq!((report.seen, report.new), (2, 2));
    assert_eq!(state.files_seen, 2);
}

#[test]
fn scan_of_missing_root_reports_an_error() {
    let env = Env::new();
    let root = env.music.path().to_path_buf();
    let moved = root.with_extension("gone");
    std::fs::rename(&root, &moved).unwrap();
    env.controller.request(env.id);
    wait_until(10, "the scan to fail", || {
        env.state().scans_finished == 1 && !env.state().running
    });
    let state = env.state();
    assert!(
        state
            .last_error
            .is_some_and(|e| e.contains("does not exist"))
    );
    assert_eq!(state.last_report, None);
    std::fs::rename(&moved, &root).unwrap();
}

#[test]
fn watcher_triggers_scan_on_new_file() {
    let env = Env::new();
    let root = env
        .db
        .read_blocking(|c| libraries::get(c, env.id))
        .unwrap()
        .unwrap()
        .root;
    let _watch = watch::spawn(env.controller.clone(), vec![(env.id, root)], Duration::ZERO);
    // Let the watcher settle, then add a file under a new folder.
    std::thread::sleep(Duration::from_millis(500));
    let dir = env.music.path().join("Album");
    std::fs::create_dir_all(&dir).unwrap();
    add_song(&dir, "fresh");
    wait_until(15, "the watcher to trigger a scan", || env.tracks() == 1);
}

#[test]
fn periodic_scan_runs() {
    let env = Env::new();
    env.add("one");
    let _watch = watch::spawn(
        env.controller.clone(),
        Vec::new(),
        Duration::from_millis(200),
    );
    wait_until(15, "two periodic scans", || env.state().scans_finished >= 2);
    assert_eq!(env.tracks(), 1);
}

#[test]
fn zero_interval_never_scans() {
    let env = Env::new();
    let _watch = watch::spawn(env.controller.clone(), Vec::new(), Duration::ZERO);
    std::thread::sleep(Duration::from_millis(500));
    assert!(env.controller.status().libraries.is_empty());
}

#[test]
fn hasher_pauses_during_scan() {
    let env = Env::new();
    env.add("one");
    let release = env.block_writer();
    assert!(!env.controller.is_hasher_paused(env.id));
    env.controller.request(env.id);
    wait_until(10, "the hasher to pause", || {
        env.controller.is_hasher_paused(env.id)
    });
    assert!(env.state().running);
    drop(release);
    wait_until(30, "the scan to end", || {
        !env.controller.status().scanning && env.state().scans_finished == 1
    });
    assert!(!env.controller.is_hasher_paused(env.id));
    // The hasher works again and gives the new track its hash.
    wait_until(30, "the hasher to hash the track", || {
        env.db
            .read_blocking(|c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM tracks WHERE hash_state = 'done'",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .unwrap()
            == 1
    });
}
