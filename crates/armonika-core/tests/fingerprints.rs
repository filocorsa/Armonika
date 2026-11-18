use armonika_core::db::Db;
use armonika_core::fingerprints::{self, FingerprintCounts, FingerprintState, StoredFingerprint};
use armonika_core::ids::LibraryId;
use rusqlite::{Connection, params};

fn open_temp() -> (tempfile::TempDir, Db, LibraryId) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("armonika.db")).unwrap();
    let library = db
        .write_blocking(|c| {
            c.execute(
                "INSERT INTO libraries (name, root, created_at) VALUES ('Music', '/music', 0)",
                [],
            )?;
            Ok(LibraryId(c.last_insert_rowid()))
        })
        .unwrap();
    (dir, db, library)
}

fn track(conn: &Connection, library: LibraryId, rel: &str, state: &str, hash: Option<&[u8]>) {
    let hash_state = if hash.is_some() { "done" } else { "pending" };
    conn.execute(
        "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms, tag_schema,
                             format, added_at, updated_at, content_hash, hash_state)
         VALUES (?1, ?2, ?3, ?1, 1, 1, 1, 'flac', 0, 0, ?4, ?5)",
        params![rel, library.0, state, hash, hash_state],
    )
    .unwrap();
}

#[test]
fn pending_lists_only_present_hashed_tracks_without_fingerprint() {
    let (_dir, db, lib) = open_temp();
    db.write_blocking(|c| {
        track(c, lib, "ok.flac", "present", Some(b"a"));
        track(c, lib, "unhashed.flac", "present", None);
        track(c, lib, "missing.flac", "missing", Some(b"b"));
        track(c, lib, "done.flac", "present", Some(b"c"));
        track(c, lib, "failed.flac", "present", Some(b"d"));
        fingerprints::store(c, b"c", "AQAA", "symphonia")?;
        fingerprints::store_failure(c, b"d", "no audio")?;
        Ok(())
    })
    .unwrap();
    let jobs = db
        .read_blocking(|c| fingerprints::pending(c, 0, &[], 20))
        .unwrap();
    let paths: Vec<&str> = jobs.iter().map(|j| j.rel_path.as_str()).collect();
    assert_eq!(paths, ["ok.flac"]);
    assert_eq!(jobs[0].content_hash, b"a");
    assert_eq!(jobs[0].library, lib);

    let none = db
        .read_blocking(|c| fingerprints::pending(c, 0, &[lib], 20))
        .unwrap();
    assert!(none.is_empty(), "a paused library is left out");
    let after = db
        .read_blocking(|c| fingerprints::pending(c, jobs[0].key, &[], 20))
        .unwrap();
    assert!(after.is_empty());
}

#[test]
fn copies_with_the_same_audio_share_one_fingerprint() {
    let (_dir, db, lib) = open_temp();
    db.write_blocking(|c| {
        track(c, lib, "one.flac", "present", Some(b"same"));
        track(c, lib, "two.flac", "present", Some(b"same"));
        Ok(())
    })
    .unwrap();
    let counts = db.read_blocking(|c| fingerprints::counts(c, lib)).unwrap();
    assert_eq!(
        counts,
        FingerprintCounts {
            done: 0,
            failed: 0,
            pending: 2
        }
    );
    db.write_blocking(|c| fingerprints::store(c, b"same", "AQAA", "symphonia"))
        .unwrap();
    let (counts, rows, state) = db
        .read_blocking(|c| {
            let rows: i64 =
                c.query_row("SELECT COUNT(*) FROM audio_fingerprints", [], |r| r.get(0))?;
            Ok((
                fingerprints::counts(c, lib)?,
                rows,
                fingerprints::state(c, b"same")?,
            ))
        })
        .unwrap();
    assert_eq!(counts.done, 2);
    assert_eq!(counts.pending, 0);
    assert_eq!(rows, 1);
    assert_eq!(
        state,
        FingerprintState::Done(StoredFingerprint {
            fingerprint: "AQAA".to_owned(),
            decoder: "symphonia".to_owned()
        })
    );
}

#[test]
fn failure_is_not_pending_any_more() {
    let (_dir, db, lib) = open_temp();
    db.write_blocking(|c| {
        track(c, lib, "bad.flac", "present", Some(b"bad"));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        db.read_blocking(|c| fingerprints::state(c, b"bad"))
            .unwrap(),
        FingerprintState::Pending
    );
    db.write_blocking(|c| fingerprints::store_failure(c, b"bad", "cannot decode"))
        .unwrap();
    let (jobs, counts, state) = db
        .read_blocking(|c| {
            Ok((
                fingerprints::pending(c, 0, &[], 20)?,
                fingerprints::counts(c, lib)?,
                fingerprints::state(c, b"bad")?,
            ))
        })
        .unwrap();
    assert!(jobs.is_empty());
    assert_eq!((counts.failed, counts.pending, counts.done), (1, 0, 0));
    assert_eq!(state, FingerprintState::Failed("cannot decode".to_owned()));
}
