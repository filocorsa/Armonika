use std::path::PathBuf;

use armonika_core::Error;
use armonika_core::db::Db;
use armonika_core::libraries::{self, LibraryConfig, RemovalSummary, SyncNote};
use armonika_core::paths::{RootError, check_library_root};

struct Fixture {
    dir: tempfile::TempDir,
    db: Db,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("data")).unwrap();
        let db = Db::open(&dir.path().join("data").join("armonika.db")).unwrap();
        Fixture { dir, db }
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    /// Creates the directory `rel` inside the temporary directory and returns its path.
    fn mkdir(&self, rel: &str) -> PathBuf {
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

fn root_error(result: armonika_core::Result<impl std::fmt::Debug>) -> RootError {
    match result {
        Err(Error::LibraryRoot { reason, .. }) => reason,
        other => panic!("expected a library root error, got {other:?}"),
    }
}

#[test]
fn nested_roots_are_rejected() {
    let f = Fixture::new();
    let music = f.mkdir("music");
    let jazz = f.mkdir("music/jazz");
    let other = f.mkdir("other");
    let data = f.data_dir();

    // Directly on the check, in both directions and for the same directory.
    assert!(matches!(
        check_library_root(&jazz, std::slice::from_ref(&music), &data),
        Err(RootError::Overlaps(_))
    ));
    assert!(matches!(
        check_library_root(&music, std::slice::from_ref(&jazz), &data),
        Err(RootError::Overlaps(_))
    ));
    assert!(matches!(
        check_library_root(&music, std::slice::from_ref(&music), &data),
        Err(RootError::Overlaps(_))
    ));
    assert!(check_library_root(&other, &[music.clone(), jazz.clone()], &data).is_ok());
    // A sibling whose name starts with the other name is not inside it.
    let music2 = f.mkdir("music2");
    assert!(check_library_root(&music2, std::slice::from_ref(&music), &data).is_ok());

    // Through the libraries, both directions.
    f.db.write_blocking(|c| {
        libraries::create(c, "Main", &music, &data)?;
        assert!(matches!(
            root_error(libraries::create(c, "Jazz", &jazz, &data)),
            RootError::Overlaps(_)
        ));
        libraries::create(c, "Other", &other, &data)?;
        // The parent of an existing library.
        let shelf = f.mkdir("shelf");
        libraries::create(c, "Shelf A", &f.mkdir("shelf/a"), &data)?;
        assert!(matches!(
            root_error(libraries::create(c, "Shelf", &shelf, &data)),
            RootError::Overlaps(_)
        ));
        // A path that names the same directory in another way.
        let dotted = music.join("..").join("music");
        assert!(matches!(
            root_error(libraries::create(c, "Again", &dotted, &data)),
            RootError::Overlaps(_)
        ));
        assert_eq!(libraries::list(c)?.len(), 3);
        Ok(())
    })
    .unwrap();
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn same_root_different_case_rejected_on_case_insensitive_os() {
    let f = Fixture::new();
    let music = f.mkdir("Music");
    let upper = f.dir.path().join("MUSIC");
    let data = f.data_dir();
    assert!(matches!(
        check_library_root(&upper, std::slice::from_ref(&music), &data),
        Err(RootError::Overlaps(_))
    ));
    f.db.write_blocking(|c| {
        libraries::create(c, "Main", &music, &data)?;
        assert!(matches!(
            root_error(libraries::create(c, "Shout", &upper, &data)),
            RootError::Overlaps(_)
        ));
        let nested = f.mkdir("Music/Jazz");
        let nested_upper = f.dir.path().join("MUSIC").join("JAZZ");
        assert!(nested.is_dir());
        assert!(matches!(
            root_error(libraries::create(c, "Jazz", &nested_upper, &data)),
            RootError::Overlaps(_)
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn data_dir_inside_library_rejected() {
    let f = Fixture::new();
    let data = f.data_dir();
    let inside = f.mkdir("data/music");
    let parent = f.dir.path().to_path_buf();
    assert_eq!(
        check_library_root(&parent, &[], &data),
        Err(RootError::ContainsDataDir)
    );
    assert_eq!(
        check_library_root(&data, &[], &data),
        Err(RootError::ContainsDataDir)
    );
    assert_eq!(
        check_library_root(&inside, &[], &data),
        Err(RootError::InsideDataDir)
    );
    let missing = f.dir.path().join("missing");
    assert_eq!(
        check_library_root(&missing, &[], &data),
        Err(RootError::NotADirectory)
    );
    let file = f.dir.path().join("file.txt");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(
        check_library_root(&file, &[], &data),
        Err(RootError::NotADirectory)
    );
    f.db.write_blocking(|c| {
        assert_eq!(
            root_error(libraries::create(c, "All", &parent, &data)),
            RootError::ContainsDataDir
        );
        assert!(libraries::list(c)?.is_empty());
        Ok(())
    })
    .unwrap();
}

#[test]
fn create_rename_and_lookup() {
    let f = Fixture::new();
    let music = f.mkdir("music");
    let other = f.mkdir("other");
    let data = f.data_dir();
    f.db.write_blocking(|c| {
        let id = libraries::create(c, "Main", &music, &data)?;
        let lib = libraries::by_name(c, "Main")?.unwrap();
        assert_eq!(lib.id, id);
        assert_eq!(lib.name, "Main");
        // The stored root is canonical (and without the `\\?\` prefix on Windows).
        assert_eq!(lib.root, dunce::canonicalize(&music).unwrap());
        assert!(!lib.root.to_string_lossy().starts_with(r"\\?\"));

        assert!(matches!(
            libraries::create(c, "Main", &other, &data),
            Err(Error::LibraryExists(_))
        ));
        assert!(matches!(
            libraries::create(c, " padded", &other, &data),
            Err(Error::InvalidLibraryName(_))
        ));
        assert!(matches!(
            libraries::create(c, "", &other, &data),
            Err(Error::InvalidLibraryName(_))
        ));

        let second = libraries::create(c, "Second", &other, &data)?;
        assert!(matches!(
            libraries::rename(c, second, "Main"),
            Err(Error::LibraryExists(_))
        ));
        libraries::rename(c, id, "Renamed")?;
        assert!(libraries::by_name(c, "Main")?.is_none());
        assert_eq!(libraries::by_name(c, "Renamed")?.unwrap().id, id);
        let names: Vec<String> = libraries::list(c)?.into_iter().map(|l| l.name).collect();
        assert_eq!(names, ["Renamed", "Second"]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn config_creates_only_missing_libraries() {
    let f = Fixture::new();
    let music = f.mkdir("music");
    let jazz = f.mkdir("jazz");
    let data = f.data_dir();
    let config = vec![
        LibraryConfig {
            name: "Main".into(),
            path: music.clone(),
        },
        LibraryConfig {
            name: "Jazz".into(),
            path: jazz.clone(),
        },
    ];
    f.db.write_blocking(|c| {
        let main = libraries::create(c, "Main", &music, &data)?;
        let notes = libraries::sync_from_config(c, &config, &data)?;
        assert!(notes.is_empty(), "{notes:?}");
        let list = libraries::list(c)?;
        assert_eq!(list.len(), 2);
        assert_eq!(libraries::by_name(c, "Main")?.unwrap().id, main);
        let jazz_id = libraries::by_name(c, "Jazz")?.unwrap().id;

        // Running again changes nothing.
        assert!(libraries::sync_from_config(c, &config, &data)?.is_empty());
        assert_eq!(libraries::list(c)?.len(), 2);
        assert_eq!(libraries::by_name(c, "Jazz")?.unwrap().id, jazz_id);

        // An invalid configured library is reported, not created, and the others go on.
        let bad = vec![
            LibraryConfig {
                name: "Nested".into(),
                path: music.join("sub"),
            },
            LibraryConfig {
                name: "Missing".into(),
                path: f.dir.path().join("nowhere"),
            },
        ];
        std::fs::create_dir(music.join("sub")).unwrap();
        let notes = libraries::sync_from_config(c, &bad, &data)?;
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes
                .iter()
                .all(|n| matches!(n, SyncNote::NotCreated { .. }))
        );
        assert_eq!(libraries::list(c)?.len(), 2);
        Ok(())
    })
    .unwrap();
}

#[test]
fn database_wins_over_config_path() {
    let f = Fixture::new();
    let music = f.mkdir("music");
    let moved = f.mkdir("moved");
    let data = f.data_dir();
    f.db.write_blocking(|c| {
        let id = libraries::create(c, "Main", &music, &data)?;
        let config = [LibraryConfig {
            name: "Main".into(),
            path: moved.clone(),
        }];
        let notes = libraries::sync_from_config(c, &config, &data)?;
        assert_eq!(notes.len(), 1, "{notes:?}");
        let SyncNote::PathDiffers {
            name,
            config_path,
            db_root,
        } = &notes[0]
        else {
            panic!("unexpected note {:?}", notes[0]);
        };
        assert_eq!(name, "Main");
        assert_eq!(config_path, &moved);
        assert_eq!(db_root, &dunce::canonicalize(&music).unwrap());
        let text = notes[0].to_string();
        assert!(text.contains("Main") && text.contains("set-path"), "{text}");

        let lib = libraries::by_name(c, "Main")?.unwrap();
        assert_eq!(lib.id, id);
        assert_eq!(lib.root, dunce::canonicalize(&music).unwrap());
        Ok(())
    })
    .unwrap();
}

#[test]
fn set_root_keeps_id() {
    let f = Fixture::new();
    let music = f.mkdir("music");
    let moved = f.mkdir("moved");
    let other = f.mkdir("other");
    let data = f.data_dir();
    f.db.write_blocking(|c| {
        let id = libraries::create(c, "Main", &music, &data)?;
        libraries::create(c, "Other", &other, &data)?;
        insert_track(c, id, "a.flac");

        // The library may move to a directory inside its own old root.
        let inner = f.mkdir("music/inner");
        libraries::set_root(c, id, &inner, &data)?;
        libraries::set_root(c, id, &moved, &data)?;
        let lib = libraries::by_name(c, "Main")?.unwrap();
        assert_eq!(lib.id, id);
        assert_eq!(lib.root, dunce::canonicalize(&moved).unwrap());
        let tracks: i64 = c.query_row(
            "SELECT COUNT(*) FROM tracks WHERE library_id = ?1",
            [id.0],
            |r| r.get(0),
        )?;
        assert_eq!(tracks, 1);

        assert!(matches!(
            root_error(libraries::set_root(c, id, &other, &data)),
            RootError::Overlaps(_)
        ));
        assert!(matches!(
            libraries::set_root(c, armonika_core::ids::LibraryId(999), &music, &data),
            Err(Error::LibraryNotFound(_))
        ));
        Ok(())
    })
    .unwrap();
}

#[test]
fn remove_keeps_files_on_disk() {
    let f = Fixture::new();
    let music = f.mkdir("music");
    let other = f.mkdir("other");
    let data = f.data_dir();
    let song = music.join("song.flac");
    std::fs::write(&song, b"not really flac").unwrap();
    f.db.write_blocking(|c| {
        let id = libraries::create(c, "Main", &music, &data)?;
        let keep = libraries::create(c, "Keep", &other, &data)?;
        let t1 = insert_track(c, id, "song.flac");
        let t2 = insert_track(c, id, "other.flac");
        let kept = insert_track(c, keep, "kept.flac");
        c.execute(
            "INSERT INTO users (public_id, name, password_hash, subsonic_password, created_at)
             VALUES ('u-1', 'alice', 'x', x'00', 0)",
            [],
        )?;
        for (pl, entries) in [(1, vec![t1, kept]), (2, vec![t2]), (3, vec![kept])] {
            c.execute(
                "INSERT INTO playlists (id, public_id, owner_id, name, created_at, updated_at)
                 VALUES (?1, 'pl-' || ?1, 1, 'p', 0, 0)",
                [pl],
            )?;
            for (pos, track) in entries.into_iter().enumerate() {
                c.execute(
                    "INSERT INTO playlist_entries (playlist_id, position, track_id)
                     VALUES (?1, ?2, ?3)",
                    [pl, pos as i64, track],
                )?;
            }
        }

        let expected = RemovalSummary {
            tracks: 2,
            playlists_touched: 2,
        };
        assert_eq!(libraries::removal_summary(c, id)?, expected);
        assert_eq!(libraries::remove(c, id)?, expected);

        assert!(libraries::by_name(c, "Main")?.is_none());
        let count = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM tracks"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM playlist_entries"), 2);
        assert_eq!(count("SELECT COUNT(*) FROM playlists"), 3);
        assert!(matches!(
            libraries::remove(c, id),
            Err(Error::LibraryNotFound(_))
        ));
        Ok(())
    })
    .unwrap();
    assert!(song.is_file(), "removing a library must never delete files");
    assert_eq!(std::fs::read(&song).unwrap(), b"not really flac");
}

/// Inserts a minimal present track and returns its internal key.
fn insert_track(
    c: &rusqlite::Connection,
    library: armonika_core::ids::LibraryId,
    rel: &str,
) -> i64 {
    c.execute(
        "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms,
                             tag_schema, format, added_at, updated_at)
         VALUES ('t-' || lower(hex(randomblob(8))), ?1, 'present', ?2, 1, 0, 1, 'flac', 0, 0)",
        rusqlite::params![library.0, rel],
    )
    .unwrap();
    c.last_insert_rowid()
}
