use std::cell::RefCell;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use armonika_core::db::Db;
use armonika_core::ids::{LibraryId, TrackId};
use armonika_core::libraries;
use armonika_media::probe::TAG_SCHEMA_VERSION;
use armonika_scanner::{
    LibraryChange, RootProblem, ScanError, ScanOptions, ScanReport, scan_library,
};
use armonika_testdata::AudioFormat;
use armonika_testdata::fixture::{FixtureTags, tagged_file};
use rusqlite::{OptionalExtension, params};

struct Lib {
    _data: tempfile::TempDir,
    music: tempfile::TempDir,
    /// A directory outside the library, to move files out and back.
    outside: tempfile::TempDir,
    db: Db,
    id: LibraryId,
}

impl Lib {
    fn new() -> Lib {
        let data = tempfile::tempdir().unwrap();
        let music = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let db = Db::open(&data.path().join("armonika.db")).unwrap();
        let id = db
            .write_blocking(|c| libraries::create(c, "Music", music.path(), data.path()))
            .unwrap();
        Lib {
            _data: data,
            music,
            outside,
            db,
            id,
        }
    }

    fn root(&self) -> &Path {
        self.music.path()
    }

    /// Writes an mp3 at `rel` (with `/` separators) titled `title`, with a modification
    /// time no other file of the test run has.
    fn add(&self, rel: &str, title: &str) -> PathBuf {
        self.add_format(rel, title, AudioFormat::Mp3)
    }

    fn add_format(&self, rel: &str, title: &str, format: AudioFormat) -> PathBuf {
        let path = self.root().join(rel);
        let dir = path.parent().unwrap();
        fs::create_dir_all(dir).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let path = tagged_file(dir, name, format, &tags(title));
        set_mtime(&path, unique_mtime());
        path
    }

    fn scan(&self) -> ScanReport {
        self.scan_with(ScanOptions {
            threads: 2,
            batch: 500,
            allow_empty: false,
        })
        .0
    }

    /// Scans accepting an empty root, for tests that take away the only file.
    fn scan_allow_empty(&self) -> ScanReport {
        self.scan_with(ScanOptions {
            allow_empty: true,
            ..ScanOptions::default()
        })
        .0
    }

    fn scan_with(&self, opts: ScanOptions) -> (ScanReport, Vec<LibraryChange>) {
        let events = RefCell::new(Vec::new());
        let report = scan_library(&self.db, self.id, opts, &|_| {}, &|c| {
            events.borrow_mut().push(c);
        })
        .unwrap();
        (report, events.into_inner())
    }

    /// The track whose present file is at `rel`.
    fn present_at(&self, rel: &str) -> Option<TrackId> {
        self.db
            .read_blocking(|c| {
                Ok(c.query_row(
                    "SELECT public_id FROM tracks
                     WHERE library_id = ?1 AND rel_path = ?2 AND state = 'present'",
                    params![self.id.0, rel],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .unwrap()
    }

    fn row(&self, id: &TrackId) -> Row {
        self.db
            .read_blocking(|c| {
                Ok(c.query_row(
                    "SELECT rel_path, state, missing_since, title, content_hash, hash_state,
                            prev_content_hash, prev_rel_path, tag_schema, library_album_id
                     FROM tracks WHERE public_id = ?1",
                    [id],
                    |r| {
                        Ok(Row {
                            rel_path: r.get(0)?,
                            state: r.get(1)?,
                            missing_since: r.get(2)?,
                            title: r.get(3)?,
                            content_hash: r.get(4)?,
                            hash_state: r.get(5)?,
                            prev_content_hash: r.get(6)?,
                            prev_rel_path: r.get(7)?,
                            tag_schema: r.get(8)?,
                            library_album_id: r.get(9)?,
                        })
                    },
                )?)
            })
            .unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.db
            .read_blocking(|c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .unwrap()
    }

    fn exec(&self, sql: &str) {
        self.db
            .write_blocking(|c| {
                c.execute_batch(sql)?;
                Ok(())
            })
            .unwrap();
    }

    /// Pretends the background hashing ran on every track.
    fn hash_everything(&self) {
        self.exec("UPDATE tracks SET content_hash = X'0102', hash_state = 'done'");
    }
}

#[derive(Debug)]
struct Row {
    rel_path: String,
    state: String,
    missing_since: Option<i64>,
    title: Option<String>,
    content_hash: Option<Vec<u8>>,
    hash_state: String,
    prev_content_hash: Option<Vec<u8>>,
    prev_rel_path: Option<String>,
    tag_schema: i64,
    library_album_id: Option<i64>,
}

fn tags(title: &str) -> FixtureTags {
    FixtureTags {
        title: Some(title.to_owned()),
        artist_credit: Some("Queen feat. David Bowie".to_owned()),
        album_title: Some("Hot Space".to_owned()),
        ..FixtureTags::default()
    }
}

fn set_mtime(path: &Path, secs: u64) {
    let file = File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

fn unique_mtime() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    1_600_000_000 + 1000 * NEXT.fetch_add(1, Ordering::Relaxed)
}

fn mtime_of(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

#[test]
fn first_scan_creates_tracks() {
    let lib = Lib::new();
    lib.add("a.mp3", "One");
    lib.add_format("Album/Disc 1/b.flac", "Two", AudioFormat::Flac);
    lib.add_format("c.ogg", "Three", AudioFormat::OggVorbis);

    let report = lib.scan();
    assert_eq!(report.seen, 3);
    assert_eq!(report.new, 3);
    assert_eq!(report.files_read, 3);
    assert_eq!(report.unreadable, 0);
    assert_eq!(report.missing, 0);

    let id = lib
        .present_at("Album/Disc 1/b.flac")
        .expect("path with / separators");
    let row = lib.row(&id);
    assert_eq!(row.state, "present");
    assert_eq!(row.title.as_deref(), Some("Two"));
    assert_eq!(row.hash_state, "pending");
    assert_eq!(row.content_hash, None);
    assert_eq!(row.tag_schema, i64::from(TAG_SCHEMA_VERSION));
    assert!(row.library_album_id.is_some(), "every track has an album");
    // "Queen feat. David Bowie" is split into two artists on every track.
    assert_eq!(lib.count("SELECT count(*) FROM track_artists"), 6);
    assert_eq!(lib.count("SELECT count(*) FROM artists"), 2);
}

#[test]
fn rescan_without_changes_reads_nothing() {
    let lib = Lib::new();
    for i in 0..4 {
        lib.add(&format!("dir/{i}.mp3"), &format!("Song {i}"));
    }
    lib.scan();
    let (report, events) = lib.scan_with(ScanOptions::default());
    assert_eq!(report.seen, 4);
    assert_eq!(report.unchanged, 4);
    assert_eq!(report.files_read, 0);
    assert_eq!(report.new + report.changed + report.missing, 0);
    assert!(events.is_empty(), "nothing changed: {events:?}");
}

#[test]
fn tag_schema_bump_rereads_once() {
    let lib = Lib::new();
    lib.add("a.mp3", "One");
    lib.add("b.mp3", "Two");
    lib.scan();
    lib.hash_everything();
    lib.exec("UPDATE tracks SET tag_schema = 0, title = 'stale'");

    let report = lib.scan();
    assert_eq!(report.files_read, 2);
    assert_eq!(report.changed, 2);
    let row = lib.row(&lib.present_at("a.mp3").unwrap());
    assert_eq!(row.title.as_deref(), Some("One"));
    assert_eq!(row.tag_schema, i64::from(TAG_SCHEMA_VERSION));
    assert_eq!(
        row.content_hash,
        Some(vec![1, 2]),
        "the file did not change"
    );
    assert_eq!(row.hash_state, "done");

    let report = lib.scan();
    assert_eq!(report.files_read, 0);
    assert_eq!(report.unchanged, 2);
}

#[test]
fn move_between_scans_is_relinked() {
    let lib = Lib::new();
    let old = lib.add("Old/song.mp3", "Song");
    lib.add("other.mp3", "Other");
    lib.scan();
    let id = lib.present_at("Old/song.mp3").unwrap();

    // Moved while the server was stopped: the scan sees the new path and misses the
    // old one in the same walk.
    fs::create_dir_all(lib.root().join("New")).unwrap();
    fs::rename(&old, lib.root().join("New/song.mp3")).unwrap();

    let report = lib.scan();
    assert_eq!(report.relinked_prefilter, 1);
    assert_eq!(report.new, 0);
    assert_eq!(report.missing, 0);
    assert_eq!(lib.present_at("New/song.mp3"), Some(id.clone()));
    let row = lib.row(&id);
    assert_eq!(row.prev_rel_path.as_deref(), Some("Old/song.mp3"));
    assert_eq!(lib.count("SELECT count(*) FROM tracks"), 2);
}

#[test]
fn changed_tags_update_same_track() {
    let lib = Lib::new();
    lib.add("a.mp3", "Before");
    lib.scan();
    let id = lib.present_at("a.mp3").unwrap();
    lib.hash_everything();

    lib.add("a.mp3", "After, with a longer title");
    let report = lib.scan();
    assert_eq!(report.changed, 1);
    assert_eq!(report.new, 0);
    assert_eq!(lib.present_at("a.mp3"), Some(id.clone()));
    let row = lib.row(&id);
    assert_eq!(row.title.as_deref(), Some("After, with a longer title"));
    assert_eq!(row.content_hash, None);
    assert_eq!(row.hash_state, "pending");
}

#[test]
fn deleted_file_makes_track_missing_and_keeps_annotations() {
    let lib = Lib::new();
    let path = lib.add("a.mp3", "Gone");
    lib.add("b.mp3", "Stays");
    lib.scan();
    let id = lib.present_at("a.mp3").unwrap();
    lib.db
        .write_blocking(|c| {
            c.execute(
                "INSERT INTO users (public_id, name, password_hash, subsonic_password,
                                    created_at)
                 VALUES ('u-1', 'anna', 'x', X'00', 0)",
                [],
            )?;
            c.execute(
                "INSERT INTO track_annotations (user_id, track_id, starred_at)
                 SELECT u.id, t.id, 42 FROM users u, tracks t
                 WHERE u.public_id = 'u-1' AND t.public_id = ?1",
                [&id],
            )?;
            Ok(())
        })
        .unwrap();

    fs::remove_file(path).unwrap();
    let (report, events) = lib.scan_with(ScanOptions::default());
    assert_eq!(report.missing, 1);
    assert_eq!(report.unchanged, 1);
    let row = lib.row(&id);
    assert_eq!(row.state, "missing");
    assert!(row.missing_since.is_some());
    assert_eq!(
        lib.count("SELECT count(*) FROM track_annotations WHERE starred_at = 42"),
        1
    );
    assert!(events.iter().any(|e| e.tracks_hidden.contains(&id)));
}

#[test]
fn reappeared_file_restores_same_track() {
    let lib = Lib::new();
    let path = lib.add("a.mp3", "Back");
    lib.scan();
    let id = lib.present_at("a.mp3").unwrap();
    let away = lib.outside.path().join("a.mp3");
    fs::rename(&path, &away).unwrap();
    assert_eq!(lib.scan_allow_empty().missing, 1);

    fs::rename(&away, &path).unwrap();
    let report = lib.scan();
    assert_eq!(report.reappeared, 1);
    assert_eq!(report.files_read, 0);
    assert_eq!(report.new, 0);
    let row = lib.row(&id);
    assert_eq!(row.state, "present");
    assert_eq!(row.missing_since, None);
    assert_eq!(lib.count("SELECT count(*) FROM tracks"), 1);
}

#[test]
fn moved_file_is_relinked_by_prefilter() {
    let lib = Lib::new();
    let path = lib.add("Artist/song.mp3", "Song");
    lib.scan();
    let id = lib.present_at("Artist/song.mp3").unwrap();
    lib.hash_everything();

    // Out of the library for one scan (missing), then back in another folder.
    let away = lib.outside.path().join("song.mp3");
    fs::rename(&path, &away).unwrap();
    assert_eq!(lib.scan_allow_empty().missing, 1);
    fs::create_dir_all(lib.root().join("Sorted/Artist")).unwrap();
    fs::rename(&away, lib.root().join("Sorted/Artist/song.mp3")).unwrap();

    let report = lib.scan();
    assert_eq!(report.relinked_prefilter, 1);
    assert_eq!(report.new, 0);
    assert_eq!(lib.present_at("Sorted/Artist/song.mp3"), Some(id.clone()));
    let row = lib.row(&id);
    assert_eq!(row.rel_path, "Sorted/Artist/song.mp3");
    assert_eq!(row.prev_content_hash, Some(vec![1, 2]));
    assert_eq!(row.prev_rel_path.as_deref(), Some("Artist/song.mp3"));
    assert_eq!(row.content_hash, None);
    assert_eq!(row.hash_state, "pending");
    assert_eq!(row.missing_since, None);
}

#[test]
fn two_missing_candidates_do_not_relink() {
    let lib = Lib::new();
    let a = lib.add("a.mp3", "Same");
    let b = lib.add("b.mp3", "Same");
    set_mtime(&a, 1_500_000_000);
    set_mtime(&b, 1_500_000_000);
    lib.scan();
    let old_a = lib.present_at("a.mp3").unwrap();
    let old_b = lib.present_at("b.mp3").unwrap();

    fs::create_dir_all(lib.root().join("copies")).unwrap();
    fs::rename(&a, lib.root().join("copies/a.mp3")).unwrap();
    fs::rename(&b, lib.root().join("copies/b.mp3")).unwrap();
    let report = lib.scan();
    assert_eq!(report.relinked_prefilter, 0);
    assert_eq!(report.new, 2);
    assert_eq!(report.missing, 2);
    let new_a = lib.present_at("copies/a.mp3").unwrap();
    assert_ne!(new_a, old_a);
    assert_ne!(new_a, old_b);
    assert_eq!(lib.row(&old_a).state, "missing");
}

#[test]
fn case_only_rename_keeps_track() {
    let lib = Lib::new();
    let path = lib.add("song.mp3", "Song");
    lib.scan();
    let id = lib.present_at("song.mp3").unwrap();
    let before = mtime_of(&path);

    // On Windows and macOS the file system ignores case: the old name is gone from the
    // walk and the new one differs only in case.
    fs::rename(&path, lib.root().join("SONG.mp3")).unwrap();
    assert_eq!(mtime_of(&lib.root().join("SONG.mp3")), before);
    let report = lib.scan();
    assert_eq!(report.new, 0);
    assert_eq!(report.missing, 0);
    assert_eq!(report.files_read, 0);
    assert_eq!(lib.present_at("SONG.mp3"), Some(id));
    assert_eq!(lib.present_at("song.mp3"), None);
}

#[test]
fn unavailable_root_aborts_without_marking_missing() {
    let lib = Lib::new();
    lib.add("a.mp3", "Song");
    lib.scan();
    let id = lib.present_at("a.mp3").unwrap();

    let unmounted = lib.outside.path().join("unmounted");
    fs::rename(lib.root(), &unmounted).unwrap();
    let events = RefCell::new(0);
    let result = scan_library(&lib.db, lib.id, ScanOptions::default(), &|_| {}, &|_| {
        *events.borrow_mut() += 1;
    });
    match result {
        Err(ScanError::RootUnavailable { reason, .. }) => assert_eq!(reason, RootProblem::NotFound),
        other => panic!("expected RootUnavailable, got {other:?}"),
    }
    assert_eq!(*events.borrow(), 0);
    assert_eq!(lib.row(&id).state, "present");

    // A file where the root should be is not a directory.
    File::create(lib.root()).unwrap();
    match scan_library(&lib.db, lib.id, ScanOptions::default(), &|_| {}, &|_| {}) {
        Err(ScanError::RootUnavailable { reason, .. }) => {
            assert_eq!(reason, RootProblem::NotADirectory);
        }
        other => panic!("expected RootUnavailable, got {other:?}"),
    }
    assert_eq!(lib.row(&id).state, "present");
    fs::remove_file(lib.root()).unwrap();
    fs::rename(&unmounted, lib.root()).unwrap();
}

#[test]
fn unreadable_file_is_skipped_and_retried() {
    let lib = Lib::new();
    fs::write(lib.root().join("broken.mp3"), b"this is not audio at all").unwrap();
    let good = lib.add("good.mp3", "Good");

    let report = lib.scan();
    assert_eq!(report.unreadable, 1);
    assert_eq!(report.new, 1);
    assert_eq!(lib.present_at("broken.mp3"), None);
    let report = lib.scan();
    assert_eq!(report.unreadable, 1, "tried again");
    assert_eq!(report.files_read, 1);

    // A known track whose file breaks stays as it was and is read again next time.
    let id = lib.present_at("good.mp3").unwrap();
    fs::write(&good, b"broken now").unwrap();
    let report = lib.scan();
    assert_eq!(report.unreadable, 2);
    assert_eq!(report.missing, 0);
    let row = lib.row(&id);
    assert_eq!(row.state, "present");
    assert_eq!(row.title.as_deref(), Some("Good"));
    assert_eq!(lib.scan().files_read, 2);

    lib.add("good.mp3", "Good again");
    let report = lib.scan();
    assert_eq!(report.changed, 1);
    assert_eq!(lib.present_at("good.mp3"), Some(id));
}

#[test]
fn dot_armonika_is_ignored() {
    let lib = Lib::new();
    lib.add(".armonika/trash/a.mp3", "Hidden");
    lib.add("music/b.mp3", "Shown");
    let report = lib.scan();
    assert_eq!(report.seen, 1);
    assert_eq!(lib.count("SELECT count(*) FROM tracks"), 1);
}

#[test]
fn non_audio_files_are_ignored() {
    let lib = Lib::new();
    fs::write(lib.root().join("cover.jpg"), b"jpeg").unwrap();
    fs::write(lib.root().join("notes.txt"), b"text").unwrap();
    fs::write(lib.root().join("noextension"), b"data").unwrap();
    lib.add("LOUD.MP3", "Upper case extension");
    let report = lib.scan();
    assert_eq!(report.seen, 1);
    assert_eq!(report.new, 1);
    assert_eq!(report.unreadable, 0);
}

#[test]
fn changes_are_emitted_in_batches() {
    let lib = Lib::new();
    for i in 0..5 {
        lib.add(&format!("{i}.mp3"), &format!("Song {i}"));
    }
    let (report, events) = lib.scan_with(ScanOptions {
        threads: 2,
        batch: 2,
        allow_empty: false,
    });
    assert_eq!(report.new, 5);
    assert!(events.len() >= 3, "{} events", events.len());
    let upserted: usize = events.iter().map(|e| e.tracks_upserted.len()).sum();
    assert_eq!(upserted, 5);
    assert!(events.iter().all(|e| e.library == lib.id));
    assert!(events.iter().any(|e| !e.artists_touched.is_empty()));
}

#[cfg(unix)]
#[test]
fn symlink_loop_does_not_hang() {
    let lib = Lib::new();
    lib.add("dir/a.mp3", "Song");
    std::os::unix::fs::symlink(lib.root().join("dir"), lib.root().join("dir/loop")).unwrap();
    let report = lib.scan();
    assert_eq!(report.new, 1);
    assert!(lib.present_at("dir/a.mp3").is_some());
}

#[test]
fn cancelled_scan_stops_after_the_current_batch() {
    let lib = Lib::new();
    for n in 1..=3 {
        lib.add(&format!("t{n}.mp3"), &format!("Song {n}"));
    }
    let batches = std::cell::Cell::new(0);
    let result = armonika_scanner::scan_library_until(
        &lib.db,
        lib.id,
        ScanOptions {
            threads: 1,
            batch: 1,
            allow_empty: false,
        },
        &|_| {},
        &|_| batches.set(batches.get() + 1),
        &|| batches.get() >= 1,
    );
    assert!(matches!(result, Err(ScanError::Cancelled)), "{result:?}");
    assert_eq!(batches.get(), 1);
    // What was committed stays; the next scan finishes the job.
    let report = lib.scan();
    assert_eq!((report.new, report.unchanged), (2, 1));
}

/// Moves every file out of the library, as a failed bind mount would look.
fn empty_root(lib: &Lib, rels: &[&str]) {
    for rel in rels {
        fs::rename(lib.root().join(rel), lib.outside.path().join(rel)).unwrap();
    }
}

#[test]
fn empty_root_with_present_tracks_marks_nothing_missing() {
    let lib = Lib::new();
    lib.add("a.mp3", "One");
    lib.add("b.mp3", "Two");
    lib.scan();
    empty_root(&lib, &["a.mp3", "b.mp3"]);

    let (report, events) = lib.scan_with(ScanOptions::default());
    assert!(report.empty_root_guard);
    assert_eq!(report.missing, 0);
    assert!(events.is_empty());
    assert_eq!(
        lib.count("SELECT count(*) FROM tracks WHERE state = 'present'"),
        2
    );
}

#[test]
fn allow_empty_marks_the_tracks_missing() {
    let lib = Lib::new();
    lib.add("a.mp3", "One");
    lib.add("b.mp3", "Two");
    lib.scan();
    empty_root(&lib, &["a.mp3", "b.mp3"]);

    let (report, _) = lib.scan_with(ScanOptions {
        allow_empty: true,
        ..ScanOptions::default()
    });
    assert!(!report.empty_root_guard);
    assert_eq!(report.missing, 2);
    assert_eq!(
        lib.count("SELECT count(*) FROM tracks WHERE state = 'present'"),
        0
    );
}

#[test]
fn one_deleted_file_still_goes_missing() {
    let lib = Lib::new();
    lib.add("a.mp3", "One");
    lib.add("b.mp3", "Two");
    lib.scan();
    fs::remove_file(lib.root().join("a.mp3")).unwrap();

    let (report, _) = lib.scan_with(ScanOptions::default());
    assert!(!report.empty_root_guard);
    assert_eq!(report.missing, 1);
    assert!(lib.present_at("b.mp3").is_some());
    assert!(lib.present_at("a.mp3").is_none());
}

#[test]
fn fresh_empty_library_scans_without_the_guard() {
    let lib = Lib::new();
    let report = lib.scan();
    assert!(!report.empty_root_guard);
    assert_eq!(report.seen, 0);

    // Only missing tracks, none present: nothing to protect either.
    lib.add("a.mp3", "One");
    lib.scan();
    fs::remove_file(lib.root().join("a.mp3")).unwrap();
    assert_eq!(lib.scan_allow_empty().missing, 1);
    assert!(!lib.scan().empty_root_guard);
}
