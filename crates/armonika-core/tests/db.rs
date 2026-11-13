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
    assert_eq!(version, 1);
    assert_eq!(tables, 1);
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
