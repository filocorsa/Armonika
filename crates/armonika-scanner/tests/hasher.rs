//! Background content hash: confirms or undoes the pre-filter links and relinks files
//! that were moved and retagged.

use std::cell::RefCell;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use armonika_core::db::Db;
use armonika_core::ids::{LibraryId, TrackId, resolve_track};
use armonika_core::libraries;
use armonika_scanner::hasher::{HashReport, Hasher};
use armonika_scanner::{LibraryChange, ScanOptions, ScanReport, scan_library};
use armonika_testdata::AudioFormat;
use armonika_testdata::fixture::{FixtureTags, tagged_file};
use rusqlite::{OptionalExtension, params};
use tokio_util::sync::CancellationToken;

struct Lib {
    _data: tempfile::TempDir,
    music: tempfile::TempDir,
    db: Db,
    id: LibraryId,
    hasher: Hasher,
}

#[derive(Debug)]
struct Row {
    rel_path: String,
    state: String,
    title: Option<String>,
    content_hash: Option<Vec<u8>>,
    hash_state: String,
    prev_content_hash: Option<Vec<u8>>,
    prev_rel_path: Option<String>,
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
            hasher: Hasher::new(),
        }
    }

    fn root(&self) -> &Path {
        self.music.path()
    }

    /// An mp3 at `rel` titled `title`, with a modification time no other file has.
    fn add(&self, rel: &str, title: &str) -> PathBuf {
        let path = self.root().join(rel);
        let dir = path.parent().unwrap();
        fs::create_dir_all(dir).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let path = tagged_file(dir, name, AudioFormat::Mp3, &tags(title));
        set_mtime(&path, unique_mtime());
        path
    }

    fn scan(&self) -> (ScanReport, Vec<LibraryChange>) {
        let events = RefCell::new(Vec::new());
        let opts = ScanOptions {
            threads: 2,
            batch: 500,
        };
        let report = scan_library(&self.db, self.id, opts, &|_| {}, &|c| {
            events.borrow_mut().push(c);
        })
        .unwrap();
        (report, events.into_inner())
    }

    fn hash(&self) -> (HashReport, Vec<LibraryChange>) {
        let events = RefCell::new(Vec::new());
        let report = self
            .hasher
            .hash_pending(&self.db, &CancellationToken::new(), &|c| {
                events.borrow_mut().push(c);
            });
        (report, events.into_inner())
    }

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
                    "SELECT rel_path, state, title, content_hash, hash_state,
                            prev_content_hash, prev_rel_path
                     FROM tracks WHERE public_id = ?1",
                    [id],
                    |r| {
                        Ok(Row {
                            rel_path: r.get(0)?,
                            state: r.get(1)?,
                            title: r.get(2)?,
                            content_hash: r.get(3)?,
                            hash_state: r.get(4)?,
                            prev_content_hash: r.get(5)?,
                            prev_rel_path: r.get(6)?,
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
}

fn tags(title: &str) -> FixtureTags {
    FixtureTags {
        title: Some(title.to_owned()),
        artist_credit: Some("Queen".to_owned()),
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

fn mtime_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn prefilter_link_confirmed_by_hash() {
    let lib = Lib::new();
    let old = lib.add("Old/song.mp3", "Song");
    lib.add("other.mp3", "Other, a different length");
    lib.scan();
    let id = lib.present_at("Old/song.mp3").unwrap();
    let (first, _) = lib.hash();
    assert_eq!(first.hashed, 2);
    let hash = lib.row(&id).content_hash.expect("hashed");
    assert_eq!(hash.len(), 32);

    fs::create_dir_all(lib.root().join("New")).unwrap();
    fs::rename(&old, lib.root().join("New/song.mp3")).unwrap();
    let (scan, _) = lib.scan();
    assert_eq!(scan.relinked_prefilter, 1);
    let row = lib.row(&id);
    assert_eq!(row.prev_content_hash.as_deref(), Some(hash.as_slice()));
    assert_eq!(row.hash_state, "pending");

    let (report, events) = lib.hash();
    assert_eq!(report.confirmed, 1);
    assert_eq!(report.reverted + report.merged + report.failed, 0);
    assert!(events.is_empty(), "a confirmed link changes nothing");
    let row = lib.row(&id);
    assert_eq!(lib.present_at("New/song.mp3"), Some(id));
    assert_eq!(row.content_hash.as_deref(), Some(hash.as_slice()));
    assert_eq!(row.hash_state, "done");
    assert_eq!(row.prev_content_hash, None);
    assert_eq!(row.prev_rel_path, None);
}

#[test]
fn prefilter_link_reverted_on_mismatch() {
    let lib = Lib::new();
    let old = lib.add("a.mp3", "Song");
    lib.scan();
    let id = lib.present_at("a.mp3").unwrap();
    lib.hash();
    let old_hash = lib.row(&id).content_hash.unwrap();

    // A different file with the same size and modification time at another path: the
    // pre-filter takes it for the moved file.
    let mut bytes = fs::read(&old).unwrap();
    let at = bytes.len() - 200;
    bytes[at] ^= 0xFF;
    let mtime = mtime_secs(&old);
    fs::remove_file(&old).unwrap();
    let new = lib.root().join("b.mp3");
    fs::write(&new, bytes).unwrap();
    set_mtime(&new, mtime);

    let (scan, _) = lib.scan();
    assert_eq!(scan.relinked_prefilter, 1, "the pre-filter is fooled");
    assert_eq!(lib.present_at("b.mp3"), Some(id.clone()));

    let (report, events) = lib.hash();
    assert_eq!(report.reverted, 1);
    let row = lib.row(&id);
    assert_eq!(row.state, "missing");
    assert_eq!(row.rel_path, "a.mp3");
    assert_eq!(row.content_hash.as_deref(), Some(old_hash.as_slice()));
    assert_eq!(row.prev_content_hash, None);
    assert_eq!(row.prev_rel_path, None);

    let fresh = lib.present_at("b.mp3").expect("a new track owns the file");
    assert_ne!(fresh, id);
    let fresh_row = lib.row(&fresh);
    assert_eq!(fresh_row.hash_state, "done");
    assert_ne!(fresh_row.content_hash, Some(old_hash));
    assert_eq!(fresh_row.title.as_deref(), Some("Song"));
    assert_eq!(lib.count("SELECT count(*) FROM tracks"), 2);
    assert_eq!(
        lib.count("SELECT count(*) FROM tracks WHERE library_album_id IS NOT NULL"),
        2
    );

    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].tracks_upserted.contains(&fresh));
    assert!(events[0].tracks_hidden.contains(&id));
    assert!(!events[0].albums_touched.is_empty());
}

/// Personal data on both tracks of a relink: the kept track and the absorbed one.
fn personal_data(lib: &Lib, kept: &TrackId, absorbed: &TrackId) {
    lib.db
        .write_blocking(|c| {
            let k = resolve_track(c, kept)?.unwrap();
            let a = resolve_track(c, absorbed)?.unwrap();
            c.execute_batch(&format!(
                "INSERT INTO users (public_id, name, password_hash, subsonic_password, created_at)
                 VALUES ('u-1', 'anna', 'x', X'00', 0);
                 INSERT INTO playlists (public_id, owner_id, name, created_at, updated_at)
                 SELECT 'p-1', id, 'Mine', 0, 0 FROM users;
                 INSERT INTO track_annotations
                   (user_id, track_id, starred_at, rating, play_count, last_played_at)
                 SELECT id, {k}, 100, 2, 5, 10 FROM users;
                 INSERT INTO track_annotations
                   (user_id, track_id, starred_at, rating, play_count, last_played_at)
                 SELECT id, {a}, NULL, 4, 3, 20 FROM users;
                 INSERT INTO playlist_entries (playlist_id, position, track_id)
                 SELECT id, 0, {k} FROM playlists;
                 INSERT INTO playlist_entries (playlist_id, position, track_id)
                 SELECT id, 1, {a} FROM playlists;
                 INSERT INTO play_history (user_id, track_id, played_at)
                 SELECT id, {a}, 7 FROM users;
                 INSERT INTO play_queues (user_id, changed_at, changed_by)
                 SELECT id, 0, 'test' FROM users;
                 INSERT INTO play_queue_entries (user_id, position, track_id)
                 SELECT id, 0, {a} FROM users;"
            ))?;
            Ok(())
        })
        .unwrap();
}

/// A library where `a.mp3` was moved to `Moved/b.mp3` and retagged, so that the
/// pre-filter finds nothing and the scan creates a new track for it. Returns the old
/// and the new track.
fn moved_and_retagged(lib: &Lib) -> (TrackId, TrackId) {
    lib.add("a.mp3", "Song");
    lib.add("keep.mp3", "Another one, much longer than the rest");
    lib.scan();
    let old = lib.present_at("a.mp3").unwrap();
    lib.hash();

    fs::remove_file(lib.root().join("a.mp3")).unwrap();
    lib.add("Moved/b.mp3", "Song (2011 Remaster), retagged");
    let (scan, _) = lib.scan();
    assert_eq!(scan.relinked_prefilter, 0);
    assert_eq!(scan.new, 1);
    assert_eq!(scan.missing, 1);
    let new = lib.present_at("Moved/b.mp3").unwrap();
    assert_ne!(new, old);
    (old, new)
}

#[test]
fn moved_and_retagged_file_relinked_by_hash() {
    let lib = Lib::new();
    let (old, new) = moved_and_retagged(&lib);
    personal_data(&lib, &old, &new);

    let (report, events) = lib.hash();
    assert_eq!(report.merged, 1);

    // The old track owns the file and its tags; the new one is gone.
    assert_eq!(lib.present_at("Moved/b.mp3"), Some(old.clone()));
    let row = lib.row(&old);
    assert_eq!(row.state, "present");
    assert_eq!(row.title.as_deref(), Some("Song (2011 Remaster), retagged"));
    assert_eq!(row.hash_state, "done");
    assert_eq!(lib.count("SELECT count(*) FROM tracks"), 2);

    // Personal data of both, merged on the old track.
    assert_eq!(lib.count("SELECT count(*) FROM track_annotations"), 1);
    assert_eq!(
        lib.count(
            "SELECT count(*) FROM track_annotations
             WHERE starred_at = 100 AND rating = 4 AND play_count = 8
               AND last_played_at = 20
               AND track_id = (SELECT id FROM tracks WHERE title LIKE 'Song (2011%')"
        ),
        1
    );
    assert_eq!(lib.count("SELECT count(*) FROM playlist_entries"), 2);
    for table in ["playlist_entries", "play_history", "play_queue_entries"] {
        assert_eq!(
            lib.count(&format!(
                "SELECT count(*) FROM {table}
                 WHERE track_id = (SELECT id FROM tracks WHERE title LIKE 'Song (2011%')"
            )),
            lib.count(&format!("SELECT count(*) FROM {table}")),
            "{table} points to the kept track"
        );
    }

    // Links of the kept track and a single event for the library.
    assert_eq!(
        lib.count("SELECT count(*) FROM tracks WHERE library_album_id IS NULL"),
        0
    );
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].tracks_upserted.contains(&old));
    assert!(!events[0].albums_touched.is_empty());

    // A second pass finds nothing to do.
    let (again, _) = lib.hash();
    assert_eq!(again.hashed, 0);
}

#[test]
fn merged_track_old_id_still_resolves() {
    let lib = Lib::new();
    let (old, new) = moved_and_retagged(&lib);
    lib.hash();

    let (from_new, from_old) = lib
        .db
        .read_blocking(|c| Ok((resolve_track(c, &new)?, resolve_track(c, &old)?)))
        .unwrap();
    assert!(from_old.is_some());
    assert_eq!(from_new, from_old, "the merged identifier redirects");
    assert_eq!(
        lib.count(&format!(
            "SELECT count(*) FROM id_redirects
             WHERE old_public_id = '{new}' AND new_public_id = '{old}'"
        )),
        1
    );
    assert_eq!(
        lib.count(&format!(
            "SELECT count(*) FROM tracks WHERE public_id = '{new}'"
        )),
        0
    );
}

#[test]
fn ambiguous_hash_does_not_relink() {
    let lib = Lib::new();
    lib.add("x/one.mp3", "One");
    lib.add("y/two.mp3", "Two, a bit longer");
    lib.scan();
    lib.hash();
    fs::remove_file(lib.root().join("x/one.mp3")).unwrap();
    fs::remove_file(lib.root().join("y/two.mp3")).unwrap();
    lib.add("z/three.mp3", "Three, longer still than the others");
    lib.scan();
    let three = lib.present_at("z/three.mp3").unwrap();

    let (report, events) = lib.hash();
    assert_eq!(report.merged, 0);
    assert_eq!(report.hashed, 1);
    assert_eq!(lib.present_at("z/three.mp3"), Some(three.clone()));
    assert_eq!(lib.row(&three).hash_state, "done");
    assert_eq!(lib.count("SELECT count(*) FROM tracks"), 3);
    assert_eq!(
        lib.count("SELECT count(*) FROM tracks WHERE state = 'missing'"),
        2
    );
    assert_eq!(lib.count("SELECT count(*) FROM id_redirects"), 0);
    assert!(events.is_empty());
}

#[test]
fn same_file_name_breaks_a_tie() {
    let lib = Lib::new();
    lib.add("x/song.mp3", "One");
    lib.add("y/other.mp3", "Two, a bit longer");
    lib.scan();
    lib.hash();
    let song = lib.present_at("x/song.mp3").unwrap();
    fs::remove_file(lib.root().join("x/song.mp3")).unwrap();
    fs::remove_file(lib.root().join("y/other.mp3")).unwrap();
    lib.add("z/song.mp3", "Three, longer still than the others");
    lib.scan();

    let (report, _) = lib.hash();
    assert_eq!(report.merged, 1);
    assert_eq!(lib.present_at("z/song.mp3"), Some(song));
}

#[test]
fn failed_hash_is_not_retried_until_file_changes() {
    let lib = Lib::new();
    let path = lib.add("a.mp3", "Song");
    lib.scan();
    let id = lib.present_at("a.mp3").unwrap();

    // Garbage of the same length and date: the scan does not notice, the hash fails.
    let len = fs::metadata(&path).unwrap().len();
    let mtime = mtime_secs(&path);
    fs::write(&path, vec![b'x'; usize::try_from(len).unwrap()]).unwrap();
    set_mtime(&path, mtime);

    let (report, _) = lib.hash();
    assert_eq!(report.failed, 1);
    assert_eq!(lib.row(&id).hash_state, "failed");
    let (again, _) = lib.hash();
    assert_eq!(again.hashed + again.failed, 0, "not retried: {again:?}");
    assert_eq!(lib.row(&id).hash_state, "failed");

    // A new version of the file: the scan makes it pending again.
    let fixed = lib.add("a.mp3", "Song, now with a longer title");
    assert_ne!(fs::metadata(fixed).unwrap().len(), len);
    lib.scan();
    assert_eq!(lib.row(&id).hash_state, "pending");
    let (report, _) = lib.hash();
    assert_eq!(report.hashed, 1);
    let row = lib.row(&id);
    assert_eq!(row.hash_state, "done");
    assert_eq!(row.content_hash.map(|h| h.len()), Some(32));
}

#[test]
fn paused_library_is_not_hashed_until_resumed() {
    let lib = Lib::new();
    lib.add("a.mp3", "Song");
    lib.scan();
    let handle = lib.hasher.handle();

    handle.pause(lib.id);
    let (report, _) = lib.hash();
    assert_eq!(report.hashed, 0);
    assert_eq!(
        lib.count("SELECT count(*) FROM tracks WHERE hash_state = 'pending'"),
        1
    );

    handle.resume(lib.id);
    let (report, _) = lib.hash();
    assert_eq!(report.hashed, 1);
}
