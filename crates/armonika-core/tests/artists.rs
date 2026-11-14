use armonika_core::artists::{
    CreditArtist, link_album_artists, link_track_artists, reconcile_artists, resolve_artists,
    split_credit,
};
use armonika_core::db::{Connection, Db};
use armonika_core::ids::{ArtistId, LibraryAlbumId, LibraryId, TrackId, resolve_artist};
use rusqlite::params;

struct Fixture {
    _dir: tempfile::TempDir,
    db: Db,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("armonika.db")).unwrap();
        Fixture { _dir: dir, db }
    }
}

fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

fn names(list: &[CreditArtist]) -> Vec<&str> {
    list.iter().map(|a| a.credited_name.as_str()).collect()
}

fn joins(list: &[CreditArtist]) -> Vec<&str> {
    list.iter().map(|a| a.join_phrase.as_str()).collect()
}

fn mbids(list: &[CreditArtist]) -> Vec<Option<&str>> {
    list.iter().map(|a| a.mbid.as_deref()).collect()
}

fn rebuild(list: &[CreditArtist]) -> String {
    list.iter()
        .map(|a| format!("{}{}", a.credited_name, a.join_phrase))
        .collect()
}

fn text_only(credit: &str) -> Vec<CreditArtist> {
    split_credit(credit, &[], &[])
}

fn named(name: &str, mbid: Option<&str>) -> CreditArtist {
    CreditArtist {
        credited_name: name.to_owned(),
        join_phrase: String::new(),
        mbid: mbid.map(str::to_owned),
    }
}

/// Inserts a library with raw SQL (no root checks needed here).
fn add_library(conn: &Connection, name: &str) -> LibraryId {
    conn.execute(
        "INSERT INTO libraries (name, root, created_at) VALUES (?1, ?2, 0)",
        params![name, format!("/music/{name}")],
    )
    .unwrap();
    LibraryId(conn.last_insert_rowid())
}

/// Inserts a present track in `library` credited to `artist` and returns its key.
fn add_track(conn: &Connection, library: LibraryId, artist: &ArtistId, credited: &str) -> i64 {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms,
                             tag_schema, format, added_at, updated_at)
         VALUES (?1, ?2, 'present', ?3, 1, 0, 1, 'flac', 0, 0)",
        params![format!("t-{n}x"), library.0, format!("track{n}.flac")],
    )
    .unwrap();
    let track = conn.last_insert_rowid();
    let artist_key = resolve_artist(conn, artist).unwrap().unwrap();
    conn.execute(
        "INSERT INTO track_artists (track_id, position, artist_id, credited_name, join_phrase)
         VALUES (?1, 0, ?2, ?3, '')",
        params![track, artist_key, credited],
    )
    .unwrap();
    track
}

/// Inserts an album in `library` credited to `artist`.
fn add_album(conn: &Connection, library: LibraryId, artist: &ArtistId) {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM library_albums", [], |r| r.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO library_albums (public_id, library_id, group_key, title, artist_credit,
                                     created_at, updated_at)
         VALUES (?1, ?2, ?3, 'Album', 'Artist', 0, 0)",
        params![format!("al-{n}x"), library.0, format!("key{n}")],
    )
    .unwrap();
    let album = conn.last_insert_rowid();
    let artist_key = resolve_artist(conn, artist).unwrap().unwrap();
    conn.execute(
        "INSERT INTO album_artists (album_id, position, artist_id, credited_name, join_phrase)
         VALUES (?1, 0, ?2, 'Artist', '')",
        params![album, artist_key],
    )
    .unwrap();
}

fn artist_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM artists", [], |r| r.get(0))
        .unwrap()
}

fn sort_name(conn: &Connection, id: &ArtistId) -> Option<String> {
    conn.query_row(
        "SELECT sort_name FROM artists WHERE public_id = ?1",
        [id.as_str()],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn ampersand_is_never_split() {
    for credit in [
        "Simon & Garfunkel",
        "Hall and Oates",
        "Al Bano e Romina Power",
        "AC/DC",
        "Tyler x Tyler",
    ] {
        let list = text_only(credit);
        assert_eq!(names(&list), [credit]);
        assert_eq!(joins(&list), [""]);
    }
}

#[test]
fn comma_is_never_split() {
    let list = text_only("Earth, Wind & Fire");
    assert_eq!(names(&list), ["Earth, Wind & Fire"]);
    assert_eq!(mbids(&list), [None]);
}

#[test]
fn feat_is_split() {
    let list = text_only("Queen feat. David Bowie");
    assert_eq!(names(&list), ["Queen", "David Bowie"]);
    assert_eq!(joins(&list), [" feat. ", ""]);

    let list = text_only("A (ft. B)");
    assert_eq!(names(&list), ["A", "B"]);
    assert_eq!(joins(&list), [" (ft. ", ")"]);
    assert_eq!(rebuild(&list), "A (ft. B)");

    let list = text_only("A Featuring B");
    assert_eq!(names(&list), ["A", "B"]);
    assert_eq!(joins(&list), [" Featuring ", ""]);

    let list = text_only("A FT. B");
    assert_eq!(names(&list), ["A", "B"]);

    // Whole words only: no split inside a name.
    for credit in ["Daft. Punk", "Defeat. Band", "Featuringly", "Left Feat"] {
        assert_eq!(names(&text_only(credit)), [credit]);
    }

    // Brackets that cannot be split safely keep the credit whole.
    for credit in [
        "A (ft. B) & C",
        "feat. B",
        "A feat.",
        // A separator inside an open bracket, an unclosed bracket.
        "A (ft. B; C)",
        "A [feat. B; C]",
        "A (ft. B",
        "A (ft. B feat. C)",
    ] {
        assert_eq!(names(&text_only(credit)), [credit]);
    }
}

#[test]
fn semicolon_is_split() {
    let list = text_only("A; B;C");
    assert_eq!(names(&list), ["A", "B", "C"]);
    assert_eq!(joins(&list), ["; ", ";", ""]);
    // An empty part means the credit cannot be split safely.
    assert_eq!(names(&text_only("A;;B")), ["A;;B"]);
}

#[test]
fn artists_tag_wins_over_text() {
    let list = split_credit("A feat. B", &s(&["X", "Y", "Z"]), &[]);
    assert_eq!(names(&list), ["X", "Y", "Z"]);
    assert_eq!(mbids(&list), [None, None, None]);
    // The tag splits even what the text would never split.
    let list = split_credit("Simon & Garfunkel", &s(&["Simon", "Garfunkel"]), &[]);
    assert_eq!(names(&list), ["Simon", "Garfunkel"]);
}

#[test]
fn mbids_pair_with_artists_tag() {
    let list = split_credit("A & B", &s(&["A", "B"]), &s(&["mb-a", "mb-b"]));
    assert_eq!(names(&list), ["A", "B"]);
    assert_eq!(mbids(&list), [Some("mb-a"), Some("mb-b")]);
    assert_eq!(joins(&list), [" & ", ""]);

    // Different lengths with more than one mbid: the mbids are ignored.
    let list = split_credit("A & B", &s(&["A", "B"]), &s(&["mb-a", "mb-b", "mb-c"]));
    assert_eq!(names(&list), ["A", "B"]);
    assert_eq!(mbids(&list), [None, None]);
    let list = split_credit("A feat. B", &[], &s(&["mb-a", "mb-b"]));
    assert_eq!(names(&list), ["A", "B"]);
    assert_eq!(mbids(&list), [None, None]);
}

#[test]
fn single_mbid_applies_to_whole_credit() {
    let list = split_credit("Simon & Garfunkel", &[], &s(&["mb-sg"]));
    assert_eq!(names(&list), ["Simon & Garfunkel"]);
    assert_eq!(mbids(&list), [Some("mb-sg")]);
    // Even over a text that would split, and over an ARTISTS tag of another length.
    let list = split_credit("A feat. B", &s(&["A", "B"]), &s(&["mb-ab"]));
    assert_eq!(names(&list), ["A feat. B"]);
    assert_eq!(mbids(&list), [Some("mb-ab")]);
}

#[test]
fn name_only_joins_unique_mbid_artist_in_library() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let with_mbid = resolve_artists(&tx, lib, &[named("Queen", Some("mb-queen"))], None)?;
        add_track(&tx, lib, &with_mbid[0], "Queen");
        let name_only = resolve_artists(&tx, lib, &[named("QUEEN", None)], None)?;
        assert_eq!(name_only, with_mbid);
        assert_eq!(artist_count(&tx), 1);

        // A link through an album counts too.
        let other = resolve_artists(&tx, lib, &[named("Muse", Some("mb-muse"))], None)?;
        add_album(&tx, lib, &other[0]);
        assert_eq!(
            resolve_artists(&tx, lib, &[named("muse", None)], None)?,
            other
        );
        assert_eq!(artist_count(&tx), 2);
        tx.commit()?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn name_only_stays_separate_with_homonyms() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let first = resolve_artists(&tx, lib, &[named("John Smith", Some("mb-1"))], None)?;
        let second = resolve_artists(&tx, lib, &[named("John Smith", Some("mb-2"))], None)?;
        assert_ne!(first, second);
        add_track(&tx, lib, &first[0], "John Smith");
        add_track(&tx, lib, &second[0], "John Smith");
        let name_only = resolve_artists(&tx, lib, &[named("John Smith", None)], None)?;
        assert_ne!(name_only, first);
        assert_ne!(name_only, second);
        assert_eq!(artist_count(&tx), 3);
        // The same name-only artist again.
        assert_eq!(
            resolve_artists(&tx, lib, &[named("john smith", None)], None)?,
            name_only
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn mbid_artist_in_other_library_does_not_capture_name_only() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let main = add_library(&tx, "Main");
        let other = add_library(&tx, "Other");
        let with_mbid = resolve_artists(&tx, other, &[named("Queen", Some("mb-queen"))], None)?;
        add_track(&tx, other, &with_mbid[0], "Queen");
        let name_only = resolve_artists(&tx, main, &[named("Queen", None)], None)?;
        assert_ne!(name_only, with_mbid);
        assert_eq!(artist_count(&tx), 2);
        // An mbid artist with no tracks or albums at all does not capture either.
        let unlinked = resolve_artists(&tx, main, &[named("Muse", Some("mb-muse"))], None)?;
        assert_ne!(
            resolve_artists(&tx, main, &[named("Muse", None)], None)?,
            unlinked
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn same_name_different_case_and_accents_is_one_artist() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let first_credit = text_only("Beyoncé");
        let second_credit = text_only("beyonce");
        let first = resolve_artists(&tx, lib, &first_credit, None)?;
        let second = resolve_artists(&tx, lib, &second_credit, None)?;
        assert_eq!(first, second);
        assert_eq!(artist_count(&tx), 1);
        let t1 = add_track(&tx, lib, &first[0], &first_credit[0].credited_name);
        let t2 = add_track(&tx, lib, &second[0], &second_credit[0].credited_name);
        let credited = |track: i64| -> String {
            tx.query_row(
                "SELECT credited_name FROM track_artists WHERE track_id = ?1",
                [track],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(credited(t1), "Beyoncé");
        assert_eq!(credited(t2), "beyonce");
        // The artist keeps the first name seen.
        let name: String = tx.query_row("SELECT name FROM artists", [], |r| r.get(0))?;
        assert_eq!(name, "Beyoncé");
        Ok(())
    })
    .unwrap();
}

#[test]
fn join_phrases_rebuild_credit() {
    let list = text_only("A feat. B; C");
    assert_eq!(names(&list), ["A", "B", "C"]);
    assert_eq!(joins(&list), [" feat. ", "; ", ""]);
    assert_eq!(rebuild(&list), "A feat. B; C");

    for credit in [
        "Queen feat. David Bowie",
        "A (ft. B)",
        "A [feat. B]",
        "A (ft. B) feat. C",
        "A Featuring B ft. C",
        "Simon & Garfunkel",
    ] {
        assert_eq!(rebuild(&text_only(credit)), credit);
    }
}

#[test]
fn join_phrase_from_artists_tag() {
    let list = split_credit("A & B", &s(&["A", "B"]), &[]);
    assert_eq!(joins(&list), [" & ", ""]);
    // Case is ignored when finding the names in the credit.
    let list = split_credit("Simon & GARFUNKEL", &s(&["simon", "Garfunkel"]), &[]);
    assert_eq!(joins(&list), [" & ", ""]);
    // Names out of order or missing from the credit give no join phrase.
    let list = split_credit("B & A", &s(&["A", "B"]), &[]);
    assert_eq!(joins(&list), ["", ""]);
    let list = split_credit("A & B", &s(&["A", "X", "B"]), &[]);
    assert_eq!(joins(&list), ["", "", ""]);
}

#[test]
fn sort_name_only_for_single_artist_credit() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let single = resolve_artists(&tx, lib, &text_only("The Beatles"), Some("Beatles, The"))?;
        assert_eq!(sort_name(&tx, &single[0]).as_deref(), Some("Beatles, The"));

        let pair = resolve_artists(
            &tx,
            lib,
            &text_only("The Who feat. The Kinks"),
            Some("Who, The feat. Kinks, The"),
        )?;
        assert_eq!(pair.len(), 2);
        assert_eq!(sort_name(&tx, &pair[0]), None);
        assert_eq!(sort_name(&tx, &pair[1]), None);

        // A later single-artist credit updates the sort name and `updated_at`.
        tx.execute("UPDATE artists SET updated_at = 0", [])?;
        let again = resolve_artists(&tx, lib, &text_only("The Beatles"), Some("Beatles"))?;
        assert_eq!(again, single);
        assert_eq!(sort_name(&tx, &single[0]).as_deref(), Some("Beatles"));
        let updated: i64 = tx.query_row(
            "SELECT updated_at FROM artists WHERE public_id = ?1",
            [single[0].as_str()],
            |r| r.get(0),
        )?;
        assert!(updated > 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn mbid_artist_name_follows_credit() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let first = resolve_artists(&tx, lib, &[named("Prince", Some("mb-p"))], None)?;
        tx.execute("UPDATE artists SET updated_at = 0", [])?;
        let second = resolve_artists(&tx, lib, &[named("The Artist", Some("mb-p"))], None)?;
        assert_eq!(first, second);
        let updated: i64 = tx.query_row("SELECT updated_at FROM artists", [], |r| r.get(0))?;
        assert!(updated > 0);
        let (name, norm): (String, String) =
            tx.query_row("SELECT name, norm_name FROM artists", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        assert_eq!((name.as_str(), norm.as_str()), ("The Artist", "the artist"));
        Ok(())
    })
    .unwrap();
}

/// Inserts a present track in `library` with no artists.
fn new_track(conn: &Connection, library: LibraryId) -> TrackId {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))
        .unwrap();
    let id = format!("t-new{n}");
    conn.execute(
        "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms,
                             tag_schema, format, added_at, updated_at)
         VALUES (?1, ?2, 'present', ?3, 1, 0, 1, 'flac', 0, 0)",
        params![id, library.0, format!("new{n}.flac")],
    )
    .unwrap();
    id.parse().unwrap()
}

/// Inserts an album in `library` with no artists.
fn new_album(conn: &Connection, library: LibraryId) -> LibraryAlbumId {
    let n: i64 = conn
        .query_row("SELECT count(*) FROM library_albums", [], |r| r.get(0))
        .unwrap();
    let id = format!("al-new{n}");
    conn.execute(
        "INSERT INTO library_albums (public_id, library_id, group_key, title, artist_credit,
                                     created_at, updated_at)
         VALUES (?1, ?2, ?3, 'Album', 'Artist', 0, 0)",
        params![id, library.0, format!("newkey{n}")],
    )
    .unwrap();
    id.parse().unwrap()
}

type TrackRow = (ArtistId, String, String, bool);

/// Rows of the track: artist, credited name, join phrase, matched_by_name.
fn track_rows(conn: &Connection, track: &TrackId) -> Vec<TrackRow> {
    conn.prepare(
        "SELECT a.public_id, ta.credited_name, ta.join_phrase, ta.matched_by_name
         FROM track_artists ta JOIN tracks t ON t.id = ta.track_id
         JOIN artists a ON a.id = ta.artist_id
         WHERE t.public_id = ?1 ORDER BY ta.position",
    )
    .unwrap()
    .query_map([track.as_str()], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

/// Rows of the album: artist, matched_by_name.
fn album_rows(conn: &Connection, album: &LibraryAlbumId) -> Vec<(ArtistId, bool)> {
    conn.prepare(
        "SELECT a.public_id, aa.matched_by_name
         FROM album_artists aa JOIN library_albums al ON al.id = aa.album_id
         JOIN artists a ON a.id = aa.artist_id
         WHERE al.public_id = ?1 ORDER BY aa.position",
    )
    .unwrap()
    .query_map([album.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}

#[test]
fn link_track_artists_replaces_rows() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let track = new_track(&tx, lib);
        let ids = link_track_artists(&tx, &track, &text_only("A feat. B"), None)?;
        assert_eq!(ids.len(), 2);
        assert_eq!(
            track_rows(&tx, &track),
            [
                (ids[0].clone(), "A".to_owned(), " feat. ".to_owned(), false),
                (ids[1].clone(), "B".to_owned(), String::new(), false),
            ]
        );
        let ids = link_track_artists(&tx, &track, &text_only("C"), Some("C, The"))?;
        assert_eq!(
            track_rows(&tx, &track),
            [(ids[0].clone(), "C".to_owned(), String::new(), false)]
        );
        assert_eq!(sort_name(&tx, &ids[0]).as_deref(), Some("C, The"));

        let album = new_album(&tx, lib);
        let ids = link_album_artists(&tx, &album, &text_only("A; C"), None)?;
        assert_eq!(
            album_rows(&tx, &album),
            [(ids[0].clone(), false), (ids[1].clone(), false)]
        );
        let ids = link_album_artists(&tx, &album, &text_only("B"), None)?;
        assert_eq!(album_rows(&tx, &album), [(ids[0].clone(), false)]);

        // An unknown track is an error.
        let missing: TrackId = "t-missing".parse().unwrap();
        assert!(link_track_artists(&tx, &missing, &text_only("A"), None).is_err());
        Ok(())
    })
    .unwrap();
}

#[test]
fn name_only_seen_before_mbid_joins_after_reconcile() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let early = new_track(&tx, lib);
        let name_only = link_track_artists(&tx, &early, &[named("Queen", None)], None)?;
        let album = new_album(&tx, lib);
        let with_mbid = link_album_artists(&tx, &album, &[named("Queen", Some("mb-q"))], None)?;
        assert_ne!(name_only, with_mbid);

        tx.execute("UPDATE artists SET updated_at = 0", [])?;
        let mut touched = reconcile_artists(&tx, lib)?;
        touched.sort();
        let mut expected = vec![name_only[0].clone(), with_mbid[0].clone()];
        expected.sort();
        assert_eq!(touched, expected);
        assert_eq!(
            track_rows(&tx, &early),
            [(
                with_mbid[0].clone(),
                "Queen".to_owned(),
                String::new(),
                true
            )]
        );
        assert_eq!(album_rows(&tx, &album), [(with_mbid[0].clone(), false)]);
        let stale: i64 = tx.query_row(
            "SELECT count(*) FROM artists WHERE updated_at = 0 AND public_id IN (?1, ?2)",
            [name_only[0].as_str(), with_mbid[0].as_str()],
            |r| r.get(0),
        )?;
        assert_eq!(stale, 0);
        Ok(())
    })
    .unwrap();
}

#[test]
fn homonym_appearing_later_splits_name_only_links() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let first = new_track(&tx, lib);
        let mb1 = link_track_artists(&tx, &first, &[named("John Smith", Some("mb-1"))], None)?;
        let plain = new_track(&tx, lib);
        // Matched at link time to the only mbid artist with that name.
        assert_eq!(
            link_track_artists(&tx, &plain, &[named("john smith", None)], None)?,
            mb1
        );
        assert!(track_rows(&tx, &plain)[0].3);
        // A homonym with another mbid appears in the library.
        let second = new_track(&tx, lib);
        let mb2 = link_track_artists(&tx, &second, &[named("John Smith", Some("mb-2"))], None)?;
        assert_ne!(mb1, mb2);

        reconcile_artists(&tx, lib)?;
        let rows = track_rows(&tx, &plain);
        assert_ne!(rows[0].0, mb1[0]);
        assert_ne!(rows[0].0, mb2[0]);
        assert_eq!(rows[0].1, "john smith");
        assert!(!rows[0].3);
        assert_eq!(track_rows(&tx, &first)[0].0, mb1[0]);
        assert_eq!(track_rows(&tx, &second)[0].0, mb2[0]);
        assert_eq!(artist_count(&tx), 3);
        Ok(())
    })
    .unwrap();
}

#[test]
fn reconcile_is_idempotent() {
    let f = Fixture::new();
    f.db.write_blocking(|c| {
        let tx = c.transaction()?;
        let lib = add_library(&tx, "Main");
        let a = new_track(&tx, lib);
        link_track_artists(&tx, &a, &text_only("Muse feat. Queen"), None)?;
        let b = new_track(&tx, lib);
        link_track_artists(&tx, &b, &[named("Muse", Some("mb-muse"))], None)?;
        assert!(!reconcile_artists(&tx, lib)?.is_empty());
        let before = (track_rows(&tx, &a), track_rows(&tx, &b), artist_count(&tx));
        assert!(reconcile_artists(&tx, lib)?.is_empty());
        let after = (track_rows(&tx, &a), track_rows(&tx, &b), artist_count(&tx));
        assert_eq!(before, after);
        Ok(())
    })
    .unwrap();
}
