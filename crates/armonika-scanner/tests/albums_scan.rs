use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use armonika_core::albums::{self, LibraryAlbum};
use armonika_core::db::Db;
use armonika_core::ids::{LibraryAlbumId, LibraryId, TrackId, resolve_album};
use armonika_core::libraries;
use armonika_core::model::PartialDate;
use armonika_scanner::{LibraryChange, ScanOptions, scan_library};
use armonika_testdata::AudioFormat;
use armonika_testdata::fixture::{FixtureTags, tagged_file};
use rusqlite::{OptionalExtension, params};

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

    /// Writes (or rewrites) an mp3 at `rel` with `tags`, with a modification time no
    /// other file of the test run has.
    fn put(&self, rel: &str, tags: &FixtureTags) {
        let path = self.music.path().join(rel);
        let dir = path.parent().unwrap();
        fs::create_dir_all(dir).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let path = tagged_file(dir, name, AudioFormat::Mp3, tags);
        set_mtime(&path, unique_mtime());
    }

    fn remove(&self, rel: &str) {
        fs::remove_file(self.music.path().join(rel)).unwrap();
    }

    fn scan(&self) -> Vec<LibraryChange> {
        self.scan_with(500)
    }

    fn scan_with(&self, batch: usize) -> Vec<LibraryChange> {
        let events = RefCell::new(Vec::new());
        scan_library(
            &self.db,
            self.id,
            ScanOptions { threads: 2, batch },
            &|_| {},
            &|c| events.borrow_mut().push(c),
        )
        .unwrap();
        events.into_inner()
    }

    fn track_at(&self, rel: &str) -> TrackId {
        self.db
            .read_blocking(|c| {
                Ok(c.query_row(
                    "SELECT public_id FROM tracks WHERE library_id = ?1 AND rel_path = ?2",
                    params![self.id.0, rel],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    }

    /// The album of the track at `rel`.
    fn album_of(&self, rel: &str) -> LibraryAlbumId {
        let track = self.track_at(rel);
        self.db
            .read_blocking(|c| albums::track_album(c, &track))
            .unwrap()
            .unwrap_or_else(|| panic!("{rel} has no album"))
    }

    fn album(&self, id: &LibraryAlbumId) -> LibraryAlbum {
        self.db
            .read_blocking(|c| albums::get(c, id))
            .unwrap()
            .unwrap_or_else(|| panic!("album {id} not found"))
    }

    fn album_count(&self) -> i64 {
        self.count("SELECT count(*) FROM library_albums")
    }

    fn count(&self, sql: &str) -> i64 {
        self.db
            .read_blocking(|c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .unwrap()
    }

    /// The names of the album artists of `album`, in order.
    fn album_artists(&self, album: &LibraryAlbumId) -> Vec<String> {
        self.db
            .read_blocking(|c| {
                let key = resolve_album(c, album)?.unwrap();
                let mut stmt = c.prepare(
                    "SELECT a.name FROM album_artists l JOIN artists a ON a.id = l.artist_id
                     WHERE l.album_id = ?1 ORDER BY l.position",
                )?;
                let names = stmt
                    .query_map([key], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                Ok(names)
            })
            .unwrap()
    }

    /// Creates a user and returns its internal key.
    fn user(&self, name: &str) -> i64 {
        self.db
            .write_blocking(|c| {
                c.execute(
                    "INSERT INTO users (public_id, name, password_hash, subsonic_password,
                                        created_at)
                     VALUES (?1, ?2, '', X'00', 0)",
                    params![format!("u-{name}"), name],
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap()
    }

    fn annotate(
        &self,
        user: i64,
        album: &LibraryAlbumId,
        starred_at: Option<i64>,
        rating: Option<i64>,
    ) {
        self.db
            .write_blocking(|c| {
                let key = resolve_album(c, album)?.unwrap();
                c.execute(
                    "INSERT INTO album_annotations (user_id, album_id, starred_at, rating)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![user, key, starred_at, rating],
                )?;
                Ok(())
            })
            .unwrap();
    }

    /// `(starred_at, rating)` of `user` on `album`.
    fn annotation(&self, user: i64, album: &LibraryAlbumId) -> Option<(Option<i64>, Option<i64>)> {
        self.db
            .read_blocking(|c| {
                let key = resolve_album(c, album)?.unwrap();
                Ok(c.query_row(
                    "SELECT starred_at, rating FROM album_annotations
                     WHERE user_id = ?1 AND album_id = ?2",
                    params![user, key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?)
            })
            .unwrap()
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

fn s(v: &str) -> Option<String> {
    Some(v.to_owned())
}

/// A track of the album `album` by `artist`, without album artist tag.
fn song(title: &str, artist: &str, album: Option<&str>) -> FixtureTags {
    FixtureTags {
        title: s(title),
        artist_credit: s(artist),
        album_title: album.map(str::to_owned),
        ..FixtureTags::default()
    }
}

/// A track of the album `album` with album artist `album_artist`.
fn album_song(title: &str, album: &str, album_artist: &str) -> FixtureTags {
    FixtureTags {
        album_artist_credit: s(album_artist),
        ..song(title, album_artist, Some(album))
    }
}

fn year(y: i32) -> PartialDate {
    PartialDate {
        year: y,
        month: None,
        day: None,
    }
}

#[test]
fn same_release_mbid_is_one_album() {
    let lib = Lib::new();
    let mut a = album_song("One", "Abbey Road", "The Beatles");
    a.mb_release_id = s("11111111-1111-1111-1111-111111111111");
    let mut b = album_song("Two", "Abbey Road (Remastered)", "Beatles");
    b.mb_release_id = a.mb_release_id.clone();
    lib.put("x/1.mp3", &a);
    lib.put("y/2.mp3", &b);
    lib.scan();

    let album = lib.album_of("x/1.mp3");
    assert_eq!(lib.album_of("y/2.mp3"), album);
    assert_eq!(lib.album_count(), 1);
    let row = lib.album(&album);
    assert_eq!(row.release_mbid, a.mb_release_id);
    assert!(row.visible);
}

#[test]
fn title_and_credit_group_without_mbid() {
    let lib = Lib::new();
    lib.put("a/1.mp3", &album_song("One", "Abbey Road", "The Beatles"));
    lib.put("b/2.mp3", &album_song("Two", "ABBEY ROAD", "the beatles"));
    lib.put(
        "c/3.mp3",
        &album_song("Three", "Abbey Road", "Abbey Road Tribute Band"),
    );
    lib.scan();

    let album = lib.album_of("a/1.mp3");
    assert_eq!(lib.album_of("b/2.mp3"), album);
    assert_ne!(lib.album_of("c/3.mp3"), album);
    assert_eq!(lib.album_count(), 2);
    let row = lib.album(&album);
    assert_eq!(row.release_mbid, None);
    assert!(!row.is_singles);
}

#[test]
fn multi_disc_folders_are_one_album() {
    let lib = Lib::new();
    lib.put(
        "Queen/Live/Disc 1/1.mp3",
        &album_song("One", "Live", "Queen"),
    );
    lib.put(
        "Queen/Live/Disc 2/1.mp3",
        &album_song("Two", "Live", "Queen"),
    );
    // A compilation without album artist: disc 2 alone has one artist, but the album
    // folder is the parent of the disc folders, where the artists are mixed.
    lib.put("Mix/CD1/1.mp3", &song("A", "Artist A", Some("Mix")));
    lib.put("Mix/CD1/2.mp3", &song("B", "Artist B", Some("Mix")));
    lib.put("Mix/CD2/1.mp3", &song("C", "Artist C", Some("Mix")));
    lib.put("Mix/CD2/2.mp3", &song("D", "Artist C", Some("Mix")));
    lib.scan();

    assert_eq!(
        lib.album_of("Queen/Live/Disc 1/1.mp3"),
        lib.album_of("Queen/Live/Disc 2/1.mp3")
    );
    let mix = lib.album_of("Mix/CD1/1.mp3");
    for rel in ["Mix/CD1/2.mp3", "Mix/CD2/1.mp3", "Mix/CD2/2.mp3"] {
        assert_eq!(lib.album_of(rel), mix, "{rel}");
    }
    assert_eq!(lib.album(&mix).artist_credit, "Various Artists");
    assert_eq!(lib.album_count(), 2);
}

#[test]
fn compilation_flag_gives_various_artists() {
    let lib = Lib::new();
    let mut a = song("One", "Artist A", Some("Hits 1990"));
    a.compilation = true;
    let mut b = song("Two", "Artist B", Some("Hits 1990"));
    b.compilation = true;
    // In different folders: the compilation flag alone is enough.
    lib.put("a/1.mp3", &a);
    lib.put("b/2.mp3", &b);
    lib.scan();

    let album = lib.album_of("a/1.mp3");
    assert_eq!(lib.album_of("b/2.mp3"), album);
    assert_eq!(lib.album(&album).artist_credit, "Various Artists");
    assert_eq!(lib.album_artists(&album), ["Various Artists"]);
}

#[test]
fn mixed_artists_in_folder_give_various_artists() {
    let lib = Lib::new();
    lib.put("Mix/1.mp3", &song("One", "Artist A", Some("Summer Mix")));
    lib.put("Mix/2.mp3", &song("Two", "Artist B", Some("Summer Mix")));
    lib.put("Mix/3.mp3", &song("Three", "Artist C", Some("Summer Mix")));
    // Same album title elsewhere, one artist: an album of its own.
    lib.put("Other/1.mp3", &song("Four", "Artist D", Some("Summer Mix")));
    lib.put("Other/2.mp3", &song("Five", "Artist D", Some("Summer Mix")));
    lib.scan();

    let mix = lib.album_of("Mix/1.mp3");
    assert_eq!(lib.album_of("Mix/2.mp3"), mix);
    assert_eq!(lib.album_of("Mix/3.mp3"), mix);
    assert_eq!(lib.album(&mix).artist_credit, "Various Artists");
    let other = lib.album_of("Other/1.mp3");
    assert_ne!(other, mix);
    assert_eq!(lib.album_of("Other/2.mp3"), other);
    assert_eq!(lib.album(&other).artist_credit, "Artist D");
    assert_eq!(lib.album_count(), 2);
}

#[test]
fn no_album_title_goes_to_artist_singles() {
    let lib = Lib::new();
    lib.put("a/1.mp3", &song("One", "Queen", None));
    lib.put("b/2.mp3", &song("Two", "Queen feat. David Bowie", None));
    lib.put("c/3.mp3", &song("Three", "David Bowie", None));
    lib.scan();

    let queen = lib.album_of("a/1.mp3");
    assert_eq!(lib.album_of("b/2.mp3"), queen, "main artist of the credit");
    let bowie = lib.album_of("c/3.mp3");
    assert_ne!(bowie, queen);
    let row = lib.album(&queen);
    assert_eq!(row.title, "[Singles]");
    assert!(row.is_singles);
    assert_eq!(row.artist_credit, "Queen");
    assert_eq!(lib.album_artists(&queen), ["Queen"]);
    assert_eq!(lib.album(&bowie).title, "[Singles]");
    assert_eq!(lib.album_artists(&bowie), ["David Bowie"]);
}

#[test]
fn folder_rule_joins_track_without_mbid() {
    let lib = Lib::new();
    let release = s("22222222-2222-2222-2222-222222222222");
    for i in 1..=3 {
        let mut t = album_song(&format!("T{i}"), "Wish You Were Here", "Pink Floyd");
        t.mb_release_id = release.clone();
        lib.put(&format!("PF/{i}.mp3"), &t);
    }
    lib.put(
        "PF/4.mp3",
        &album_song("T4", "Wish You Were Here", "Pink Floyd"),
    );
    // Elsewhere, the same title and credit without mbid stay a tag album.
    lib.put(
        "Elsewhere/1.mp3",
        &album_song("T5", "Wish You Were Here", "Pink Floyd"),
    );
    lib.scan();

    let album = lib.album_of("PF/1.mp3");
    assert_eq!(lib.album_of("PF/4.mp3"), album);
    assert_eq!(lib.album(&album).release_mbid, release);
    assert_ne!(lib.album_of("Elsewhere/1.mp3"), album);
}

#[test]
fn split_keeps_id_on_larger_part() {
    let lib = Lib::new();
    for i in 1..=8 {
        lib.put(
            &format!("A/{i}.mp3"),
            &album_song(&format!("T{i}"), "Album", "Band"),
        );
    }
    lib.scan();
    let album = lib.album_of("A/1.mp3");
    let user = lib.user("ann");
    lib.annotate(user, &album, Some(1000), Some(4));

    for i in 6..=8 {
        lib.put(
            &format!("A/{i}.mp3"),
            &album_song(&format!("T{i}"), "Other Album", "Band"),
        );
    }
    let events = lib.scan();

    for i in 1..=5 {
        assert_eq!(lib.album_of(&format!("A/{i}.mp3")), album, "track {i}");
    }
    let other = lib.album_of("A/6.mp3");
    assert_ne!(other, album);
    assert_eq!(lib.album_of("A/8.mp3"), other);
    assert_eq!(lib.annotation(user, &album), Some((Some(1000), Some(4))));
    assert_eq!(lib.annotation(user, &other), None);
    assert_eq!(lib.album(&album).title, "Album");
    assert_eq!(lib.album(&other).title, "Other Album");
    let touched: BTreeSet<_> = events
        .iter()
        .flat_map(|e| e.albums_touched.clone())
        .collect();
    assert!(touched.contains(&other), "{touched:?}");
}

#[test]
fn split_keeps_id_when_the_larger_part_is_retagged() {
    let lib = Lib::new();
    for i in 1..=8 {
        lib.put(
            &format!("A/{i}.mp3"),
            &album_song(&format!("T{i}"), "Albm", "Band"),
        );
    }
    lib.scan();
    let album = lib.album_of("A/1.mp3");
    // The title typo is fixed on 5 tracks: the 5 keep the id and the album its new title.
    for i in 1..=5 {
        lib.put(
            &format!("A/{i}.mp3"),
            &album_song(&format!("T{i}"), "Album", "Band"),
        );
    }
    lib.scan();

    assert_eq!(lib.album_of("A/1.mp3"), album);
    assert_eq!(lib.album(&album).title, "Album");
    assert_ne!(lib.album_of("A/8.mp3"), album);
    assert_eq!(lib.album(&lib.album_of("A/8.mp3")).title, "Albm");
}

#[test]
fn merge_keeps_id_of_larger_and_moves_stars() {
    let lib = Lib::new();
    for i in 1..=5 {
        lib.put(
            &format!("Big/{i}.mp3"),
            &album_song(&format!("B{i}"), "Album", "Band"),
        );
    }
    for i in 1..=3 {
        lib.put(
            &format!("Small/{i}.mp3"),
            &album_song(&format!("S{i}"), "Album (Bonus)", "Band"),
        );
    }
    lib.scan();
    let big = lib.album_of("Big/1.mp3");
    let small = lib.album_of("Small/1.mp3");
    assert_ne!(big, small);
    let ann = lib.user("ann");
    let bob = lib.user("bob");
    let cid = lib.user("cid");
    lib.annotate(ann, &small, Some(500), Some(5));
    lib.annotate(bob, &big, Some(2000), Some(2));
    lib.annotate(bob, &small, Some(1000), Some(3));
    lib.annotate(cid, &big, None, Some(1));

    for i in 1..=3 {
        lib.put(
            &format!("Small/{i}.mp3"),
            &album_song(&format!("S{i}"), "Album", "Band"),
        );
    }
    let events = lib.scan();

    assert_eq!(lib.album_of("Small/1.mp3"), big);
    assert_eq!(lib.album_count(), 1);
    assert_eq!(lib.annotation(ann, &big), Some((Some(500), Some(5))));
    assert_eq!(
        lib.annotation(bob, &big),
        Some((Some(1000), Some(3))),
        "oldest star, highest rating"
    );
    assert_eq!(lib.annotation(cid, &big), Some((None, Some(1))));
    let touched: BTreeSet<_> = events
        .iter()
        .flat_map(|e| e.albums_touched.clone())
        .collect();
    assert!(touched.contains(&big), "{touched:?}");
}

#[test]
fn merged_album_old_id_still_resolves() {
    let lib = Lib::new();
    for i in 1..=3 {
        lib.put(
            &format!("Big/{i}.mp3"),
            &album_song(&format!("B{i}"), "Album", "Band"),
        );
    }
    lib.put("Small/1.mp3", &album_song("S1", "Album", "The Band"));
    lib.scan();
    let big = lib.album_of("Big/1.mp3");
    let small = lib.album_of("Small/1.mp3");
    assert_ne!(big, small);

    lib.put("Small/1.mp3", &album_song("S1", "Album", "Band"));
    lib.scan();

    let (kept, old) = lib
        .db
        .read_blocking(|c| Ok((resolve_album(c, &big)?, resolve_album(c, &small)?)))
        .unwrap();
    assert!(kept.is_some());
    assert_eq!(old, kept);
    assert_eq!(lib.album(&small).id, big, "get follows the redirect");
}

#[test]
fn empty_album_with_star_is_kept_hidden() {
    let lib = Lib::new();
    lib.put("A/1.mp3", &album_song("One", "Album", "Band"));
    lib.put("A/2.mp3", &album_song("Two", "Album", "Band"));
    lib.put("B/1.mp3", &album_song("Other", "Other", "Band"));
    lib.scan();
    let album = lib.album_of("A/1.mp3");
    let user = lib.user("ann");
    lib.annotate(user, &album, Some(1000), None);

    lib.remove("A/1.mp3");
    lib.remove("A/2.mp3");
    let events = lib.scan();

    let row = lib.album(&album);
    assert!(!row.visible);
    assert_eq!(lib.annotation(user, &album), Some((Some(1000), None)));
    assert_eq!(lib.album_count(), 2);
    let touched: BTreeSet<_> = events
        .iter()
        .flat_map(|e| e.albums_touched.clone())
        .collect();
    assert!(
        touched.contains(&album),
        "hidden album reported: {touched:?}"
    );

    // The files come back: same album, star still there.
    lib.put("A/1.mp3", &album_song("One", "Album", "Band"));
    lib.scan();
    assert_eq!(lib.album_of("A/1.mp3"), album);
    assert!(lib.album(&album).visible);
}

#[test]
fn album_original_date_from_tracks() {
    let lib = Lib::new();
    for i in 1..=3 {
        let mut t = album_song(&format!("T{i}"), "Physical Graffiti", "Led Zeppelin");
        t.date = s("2011");
        t.original_date = s("1975");
        t.album_sort = s("Physical Graffiti (Remaster)");
        lib.put(&format!("LZ/{i}.mp3"), &t);
    }
    // One track with another date does not win.
    let mut odd = album_song("T4", "Physical Graffiti", "Led Zeppelin");
    odd.date = s("2015");
    odd.original_date = s("1975-02-24");
    lib.put("LZ/4.mp3", &odd);
    lib.scan();

    let row = lib.album(&lib.album_of("LZ/1.mp3"));
    assert_eq!(row.date, Some(year(2011)));
    assert_eq!(row.original_date, Some(year(1975)));
    assert_eq!(
        row.sort_title.as_deref(),
        Some("Physical Graffiti (Remaster)")
    );
}

#[test]
fn grouping_does_not_depend_on_batch_size() {
    let layout = |lib: &Lib| {
        lib.put("Mix/1.mp3", &song("One", "Artist A", Some("Summer Mix")));
        lib.put("Mix/2.mp3", &song("Two", "Artist B", Some("Summer Mix")));
        lib.put("Mix/3.mp3", &song("Three", "Artist C", Some("Summer Mix")));
        let mut t = album_song("T1", "Animals", "Pink Floyd");
        t.mb_release_id = s("33333333-3333-3333-3333-333333333333");
        lib.put("PF/2.mp3", &t);
        lib.put("PF/1.mp3", &album_song("T0", "Animals", "Pink Floyd"));
    };
    let summary = |lib: &Lib| {
        let mut out = Vec::new();
        for rel in [
            "Mix/1.mp3",
            "Mix/2.mp3",
            "Mix/3.mp3",
            "PF/1.mp3",
            "PF/2.mp3",
        ] {
            let row = lib.album(&lib.album_of(rel));
            out.push((rel, row.title, row.artist_credit, row.release_mbid));
        }
        (out, lib.album_count())
    };
    let one = Lib::new();
    layout(&one);
    one.scan_with(1);
    let all = Lib::new();
    layout(&all);
    all.scan_with(500);
    assert_eq!(summary(&one), summary(&all));
    assert_eq!(one.album_count(), 2);
}

#[test]
fn rescan_without_changes_touches_no_album() {
    let lib = Lib::new();
    lib.put("Mix/1.mp3", &song("One", "Artist A", Some("Summer Mix")));
    lib.put("Mix/2.mp3", &song("Two", "Artist B", Some("Summer Mix")));
    lib.put("x.mp3", &song("Single", "Artist A", None));
    lib.scan();
    let updated = lib.count("SELECT max(updated_at) FROM library_albums");
    let events = lib.scan();
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(
        lib.count("SELECT max(updated_at) FROM library_albums"),
        updated
    );
}
