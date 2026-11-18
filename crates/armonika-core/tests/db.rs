use std::str::FromStr;

use armonika_core::db::Db;
use armonika_core::ids::{self, ArtistId, LibraryAlbumId, TrackId};
use rusqlite::{Connection, params};

fn open_temp() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("armonika.db")).unwrap();
    (dir, db)
}

fn insert_library(conn: &Connection) -> i64 {
    conn.execute(
        "INSERT INTO libraries (name, root, created_at) VALUES ('Music', '/music', 0)",
        [],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn insert_track(
    conn: &Connection,
    library_id: i64,
    public_id: &str,
    rel_path: &str,
    state: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms, tag_schema,
                             format, added_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 1, 1, 1, 'flac', 0, 0)",
        params![public_id, library_id, state, rel_path],
    )?;
    Ok(conn.last_insert_rowid())
}

#[test]
fn open_applies_migrations_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("armonika.db");
    drop(Db::open(&path).unwrap());
    let db = Db::open(&path).unwrap();
    let (version, tables): (i64, i64) = db
        .read_blocking(|c| {
            let version = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            let tables = c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'tracks'",
                [],
                |r| r.get(0),
            )?;
            Ok((version, tables))
        })
        .unwrap();
    assert_eq!(version, 6);
    assert_eq!(tables, 1);
}

/// Two processes opening a new database at the same time must not both apply the
/// migrations.
#[test]
fn concurrent_open_applies_migrations_once() {
    for _ in 0..10 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("armonika.db");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Db::open(&path).map(drop)
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        let version: i64 = Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 6);
    }
}

#[test]
fn migration_2_adds_artist_links() {
    let (_dir, db) = open_temp();
    db.read_blocking(|c| {
        for table in ["track_artists", "album_artists"] {
            let n: i64 = c.query_row(
                "SELECT count(*) FROM pragma_table_info(?1) WHERE name = 'matched_by_name'",
                [table],
                |r| r.get(0),
            )?;
            assert_eq!(n, 1, "{table}");
        }
        let n: i64 = c.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'artists_norm_mbid'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(n, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn migration_3_adds_album_dates_and_tag_lists() {
    let (_dir, db) = open_temp();
    db.read_blocking(|c| {
        let columns = |table: &str| -> rusqlite::Result<Vec<String>> {
            let mut stmt = c.prepare("SELECT name FROM pragma_table_info(?1)")?;
            stmt.query_map([table], |r| r.get(0))?.collect()
        };
        let albums = columns("library_albums")?;
        for col in [
            "date_y",
            "date_m",
            "date_d",
            "orig_date_y",
            "orig_date_m",
            "orig_date_d",
        ] {
            assert!(albums.iter().any(|c| c == col), "library_albums.{col}");
        }
        let tracks = columns("tracks")?;
        for col in [
            "artists_tag",
            "album_artists_tag",
            "mb_artist_ids",
            "mb_album_artist_ids",
            "album_title_norm",
            "isrcs",
            "id3v2_major",
        ] {
            assert!(tracks.iter().any(|c| c == col), "tracks.{col}");
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn foreign_keys_are_enforced() {
    let (_dir, db) = open_temp();
    let result = db.write_blocking(|c| {
        let lib = insert_library(c);
        c.execute(
            "INSERT INTO library_access (user_id, library_id, permission)
             VALUES (999, ?1, 'listen')",
            [lib],
        )?;
        Ok(())
    });
    assert!(result.is_err());
}

#[test]
fn wal_mode_is_on() {
    let (_dir, db) = open_temp();
    let (mode, fk): (String, i64) = db
        .read_blocking(|c| {
            let mode = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
            let fk = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
            Ok((mode, fk))
        })
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
    assert_eq!(fk, 1);
}

#[test]
fn present_path_is_unique_but_missing_is_not() {
    let (_dir, db) = open_temp();
    let duplicate_present = db
        .write_blocking(|c| {
            let lib = insert_library(c);
            insert_track(c, lib, "t-1", "a/b.flac", "missing")?;
            insert_track(c, lib, "t-2", "a/b.flac", "missing")?;
            insert_track(c, lib, "t-3", "a/b.flac", "present")?;
            Ok(insert_track(c, lib, "t-4", "a/b.flac", "present").is_err())
        })
        .unwrap();
    assert!(duplicate_present);
}

#[tokio::test]
async fn readers_see_committed_writes() {
    let (_dir, db) = open_temp();
    db.write(|c| {
        let tx = c.transaction()?;
        insert_library(&tx);
        tx.commit()?;
        Ok(())
    })
    .await
    .unwrap();
    let count: i64 = db
        .read(|c| Ok(c.query_row("SELECT count(*) FROM libraries", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn resolve_follows_redirect() {
    let (_dir, db) = open_temp();
    let old = TrackId::generate();
    let new = TrackId::generate();
    let (key, resolved_old, resolved_new, unknown) = db
        .write_blocking({
            let (old, new) = (old.clone(), new.clone());
            move |c| {
                let lib = insert_library(c);
                let key = insert_track(c, lib, new.as_str(), "x.flac", "present")?;
                let tx = c.transaction()?;
                ids::add_redirect(&tx, old.as_str(), new.as_str())?;
                tx.commit()?;
                Ok((
                    key,
                    ids::resolve_track(c, &old)?,
                    ids::resolve_track(c, &new)?,
                    ids::resolve_track(c, &TrackId::generate())?,
                ))
            }
        })
        .unwrap();
    assert_eq!(resolved_old, Some(key));
    assert_eq!(resolved_new, Some(key));
    assert_eq!(unknown, None);
}

#[test]
fn resolve_works_for_albums_and_artists() {
    let (_dir, db) = open_temp();
    let album = LibraryAlbumId::generate();
    let artist = ArtistId::generate();
    let (a, b) = db
        .write_blocking(move |c| {
            let lib = insert_library(c);
            c.execute(
                "INSERT INTO library_albums (public_id, library_id, group_key, title,
                                             artist_credit, created_at, updated_at)
                 VALUES (?1, ?2, 'k', 'T', 'A', 0, 0)",
                params![album.as_str(), lib],
            )?;
            c.execute(
                "INSERT INTO artists (public_id, name, norm_name, updated_at)
                 VALUES (?1, 'A', 'a', 0)",
                [artist.as_str()],
            )?;
            Ok((
                ids::resolve_album(c, &album)?,
                ids::resolve_artist(c, &artist)?,
            ))
        })
        .unwrap();
    assert!(a.is_some());
    assert!(b.is_some());
}

#[test]
fn redirect_chains_are_flattened() {
    let (_dir, db) = open_temp();
    let a = TrackId::generate();
    let b = TrackId::generate();
    let c_id = TrackId::generate();
    let (target, key, resolved) = db
        .write_blocking({
            let c_id = c_id.clone();
            move |c| {
                let lib = insert_library(c);
                let key = insert_track(c, lib, c_id.as_str(), "x.flac", "present")?;
                let tx = c.transaction()?;
                ids::add_redirect(&tx, a.as_str(), b.as_str())?;
                ids::add_redirect(&tx, b.as_str(), c_id.as_str())?;
                tx.commit()?;
                // A single read of id_redirects must lead from A straight to C.
                let target: String = c.query_row(
                    "SELECT new_public_id FROM id_redirects WHERE old_public_id = ?1",
                    [a.as_str()],
                    |r| r.get(0),
                )?;
                Ok((target, key, ids::resolve_track(c, &a)?))
            }
        })
        .unwrap();
    assert_eq!(TrackId::from_str(&target).unwrap(), c_id);
    assert_eq!(resolved, Some(key));
}

#[test]
fn redirect_to_an_already_redirected_id_points_at_the_final_target() {
    let (_dir, db) = open_temp();
    let (a, b, c_id) = (
        TrackId::generate(),
        TrackId::generate(),
        TrackId::generate(),
    );
    let target: String = db
        .write_blocking({
            let c_id = c_id.clone();
            move |c| {
                let tx = c.transaction()?;
                ids::add_redirect(&tx, b.as_str(), c_id.as_str())?;
                ids::add_redirect(&tx, a.as_str(), b.as_str())?;
                tx.commit()?;
                Ok(c.query_row(
                    "SELECT new_public_id FROM id_redirects WHERE old_public_id = ?1",
                    [a.as_str()],
                    |r| r.get(0),
                )?)
            }
        })
        .unwrap();
    assert_eq!(target, c_id.as_str());
}

#[test]
fn reversing_a_merge_keeps_one_redirect() {
    let (_dir, db) = open_temp();
    let (a, b) = (TrackId::generate(), TrackId::generate());
    let rows: Vec<(String, String)> = db
        .write_blocking({
            let (a, b) = (a.clone(), b.clone());
            move |c| {
                let tx = c.transaction()?;
                ids::add_redirect(&tx, a.as_str(), b.as_str())?;
                ids::add_redirect(&tx, b.as_str(), a.as_str())?;
                tx.commit()?;
                let mut stmt =
                    c.prepare("SELECT old_public_id, new_public_id FROM id_redirects")?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                Ok(rows)
            }
        })
        .unwrap();
    assert_eq!(rows, vec![(b.to_string(), a.to_string())]);
}

/// SQLite does not index the child side of a foreign key: without an index every
/// cascade scans the whole child table. Each foreign key must be the leading part of a
/// non-partial index (a primary key counts).
#[test]
fn child_side_of_every_foreign_key_is_indexed() {
    let (_dir, db) = open_temp();
    let missing = db
        .read_blocking(|c| {
            let tables: Vec<String> = c
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let mut missing = Vec::new();
            for table in tables {
                // Column lists of the foreign keys, in key order.
                let mut fks: std::collections::BTreeMap<i64, Vec<String>> = Default::default();
                let mut stmt = c.prepare("SELECT id, \"from\" FROM pragma_foreign_key_list(?1)")?;
                let rows = stmt.query_map([&table], |r| Ok((r.get(0)?, r.get(1)?)))?;
                for row in rows {
                    let (id, col): (i64, String) = row?;
                    fks.entry(id).or_default().push(col);
                }
                // Leading columns of every non-partial index.
                let mut indexes: Vec<Vec<String>> = Vec::new();
                let names: Vec<String> = c
                    .prepare("SELECT name FROM pragma_index_list(?1) WHERE partial = 0")?
                    .query_map([&table], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                for name in names {
                    indexes.push(
                        c.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?
                            .query_map([&name], |r| r.get(0))?
                            .collect::<rusqlite::Result<_>>()?,
                    );
                }
                // An INTEGER PRIMARY KEY is the rowid, indexed by the table itself.
                let rowid: Vec<String> = c
                    .prepare(
                        "SELECT name FROM pragma_table_info(?1)
                         WHERE pk = 1 AND upper(type) = 'INTEGER'
                           AND (SELECT count(*) FROM pragma_table_info(?1) WHERE pk > 0) = 1",
                    )?
                    .query_map([&table], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                if !rowid.is_empty() {
                    indexes.push(rowid);
                }
                for cols in fks.into_values() {
                    if !indexes.iter().any(|ix| ix.starts_with(&cols)) {
                        missing.push(format!("{table}({})", cols.join(", ")));
                    }
                }
            }
            Ok(missing)
        })
        .unwrap();
    assert!(
        missing.is_empty(),
        "foreign keys without an index: {missing:?}"
    );
}

#[test]
fn api_key_labels_are_unique_per_user() {
    let (_dir, db) = open_temp();
    let duplicate = db
        .write_blocking(|c| {
            c.execute_batch(
                "INSERT INTO users (id, public_id, name, password_hash, subsonic_password,
                                    created_at)
                 VALUES (1, 'u-1', 'alice', 'x', x'00', 0), (2, 'u-2', 'bob', 'x', x'00', 0);
                 INSERT INTO api_keys (key_hash, user_id, label, created_at)
                 VALUES ('h1', 1, 'phone', 0), ('h2', 2, 'phone', 0);",
            )?;
            Ok(c.execute(
                "INSERT INTO api_keys (key_hash, user_id, label, created_at)
                 VALUES ('h3', 1, 'phone', 0)",
                [],
            )
            .is_err())
        })
        .unwrap();
    assert!(
        duplicate,
        "a second key with the same label for one user was accepted"
    );
}

#[test]
fn library_names_are_unique_without_case() {
    let (_dir, db) = open_temp();
    let duplicate = db
        .write_blocking(|c| {
            insert_library(c);
            Ok(c.execute(
                "INSERT INTO libraries (name, root, created_at) VALUES ('MUSIC', '/other', 0)",
                [],
            )
            .is_err())
        })
        .unwrap();
    assert!(duplicate);
}

/// Opening must wait (busy_timeout) for another connection that holds the database
/// lock, even before switching to WAL.
#[test]
fn open_waits_for_a_locked_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("armonika.db");
    let other = Connection::open(&path).unwrap();
    other
        .execute_batch("CREATE TABLE t (x); BEGIN EXCLUSIVE; INSERT INTO t VALUES (1);")
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        other.execute_batch("COMMIT").unwrap();
    });
    let opened = Db::open(&path);
    release.join().unwrap();
    opened.unwrap();
}

#[test]
fn negative_user_version_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("armonika.db");
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", -1)
        .unwrap();
    let err = Db::open(&path).unwrap_err();
    assert!(
        matches!(err, armonika_core::Error::InvalidSchemaVersion(-1)),
        "{err:?}"
    );
}

#[test]
fn panic_inside_a_write_rolls_back_an_open_transaction() {
    let (_dir, db) = open_temp();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        db.write_blocking(|c| -> armonika_core::Result<()> {
            // A transaction begun with SQL is not rolled back by any drop guard.
            c.execute_batch(
                "BEGIN; INSERT INTO settings (key, value) VALUES ('half', 'written');",
            )?;
            panic!("closure fails in the middle of a batch");
        })
    }));
    assert!(panicked.is_err());

    let (autocommit, rows) = db
        .write_blocking(|c| {
            let rows: i64 = c.query_row("SELECT count(*) FROM settings", [], |r| r.get(0))?;
            Ok((c.is_autocommit(), rows))
        })
        .unwrap();
    assert!(
        autocommit,
        "the next writer must not inherit the transaction"
    );
    assert_eq!(rows, 0, "the half-written batch is rolled back");
}
