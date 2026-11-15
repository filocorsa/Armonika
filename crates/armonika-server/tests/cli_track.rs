use std::path::Path;

use armonika_core::db::Db;

/// Runs `armonika <args>` with a fresh environment pointing at `dir`.
/// Returns (success, stdout, stderr).
fn armonika(dir: &Path, args: &[&str], stdin: &str) -> (bool, String, String) {
    let out = assert_cmd::Command::cargo_bin("armonika")
        .unwrap()
        .args(args)
        .env_remove("ARMONIKA_CONFIG")
        .env_remove("ARMONIKA_SECRET_KEY")
        .env_remove("ARMONIKA_ADMIN_USER")
        .env_remove("ARMONIKA_ADMIN_PASSWORD")
        .env("ARMONIKA_DATA_DIR", dir.join("data"))
        .current_dir(dir)
        .write_stdin(stdin)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// A library "Main" with a user and four tracks: `present`, `old` (missing for 40 days),
/// `recent` (missing for 1 hour) and `other` (missing, in another library). The user has
/// an annotation, a history entry and a playlist entry on each of them.
fn setup(dir: &Path) {
    for name in ["music", "other"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }
    for (name, path) in [("Main", "music"), ("Other", "other")] {
        let path = dir.join(path);
        let (ok, _, err) = armonika(dir, &["library", "add", name, path.to_str().unwrap()], "");
        assert!(ok, "{err}");
    }
    let (ok, _, err) = armonika(
        dir,
        &["user", "create", "bob", "--password-stdin"],
        "battery staple\n",
    );
    assert!(ok, "{err}");

    let db = Db::open(&dir.join("data").join("armonika.db")).unwrap();
    let now = armonika_core::db::now_ms();
    db.write_blocking(|c| {
        let user: i64 = c.query_row("SELECT id FROM users", [], |r| r.get(0))?;
        let main: i64 = c.query_row("SELECT id FROM libraries WHERE name = 'Main'", [], |r| {
            r.get(0)
        })?;
        let other: i64 = c.query_row("SELECT id FROM libraries WHERE name = 'Other'", [], |r| {
            r.get(0)
        })?;
        c.execute(
            "INSERT INTO playlists (id, public_id, owner_id, name, created_at, updated_at)
             VALUES (1, 'pl1', ?1, 'Mix', 0, 0)",
            [user],
        )?;
        let tracks = [
            ("present", main, "present", None),
            ("old", main, "missing", Some(now - 40 * DAY_MS)),
            ("recent", main, "missing", Some(now - 3_600_000)),
            ("other", other, "missing", Some(now - 40 * DAY_MS)),
        ];
        for (i, (name, library, state, since)) in tracks.into_iter().enumerate() {
            let key = i64::try_from(i).unwrap() + 1;
            c.execute(
                "INSERT INTO tracks (id, public_id, library_id, state, rel_path, size, mtime_ms,
                                     tag_schema, format, added_at, updated_at, missing_since)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, 1, 'mp3', 0, 0, ?6)",
                rusqlite::params![key, name, library, state, format!("{name}.mp3"), since],
            )?;
            c.execute(
                "INSERT INTO track_annotations (user_id, track_id, play_count) VALUES (?1, ?2, 1)",
                [user, key],
            )?;
            c.execute(
                "INSERT INTO play_history (user_id, track_id, played_at) VALUES (?1, ?2, 5)",
                [user, key],
            )?;
            c.execute(
                "INSERT INTO playlist_entries (playlist_id, position, track_id) VALUES (1, ?1, ?1)",
                [key],
            )?;
        }
        // The saved queue holds `present` and `old`.
        c.execute(
            "INSERT INTO play_queues (user_id, changed_at, changed_by) VALUES (?1, 0, 'test')",
            [user],
        )?;
        c.execute(
            "INSERT INTO play_queue_entries (user_id, position, track_id)
             VALUES (?1, 0, 1), (?1, 1, 2)",
            [user],
        )?;
        Ok(())
    })
    .unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let db = Db::open(&dir.join("data").join("armonika.db")).unwrap();
    db.read_blocking(|c| {
        let mut stmt = c.prepare("SELECT public_id FROM tracks ORDER BY id")?;
        Ok(stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    })
    .unwrap()
}

fn count(dir: &Path, table: &str) -> i64 {
    let db = Db::open(&dir.join("data").join("armonika.db")).unwrap();
    db.read_blocking(|c| {
        Ok(c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
    })
    .unwrap()
}

#[test]
fn purge_missing_requires_yes() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());

    let (ok, out, err) = armonika(dir.path(), &["track", "purge-missing", "Main"], "");
    assert!(ok, "{out}{err}");
    assert!(
        out.contains("2 tracks")
            && out.contains("2 annotations")
            && out.contains("2 history entries")
            && out.contains("1 playlists")
            && out.contains("2 playlist entries")
            && out.contains("1 play queue entries")
            && out.contains("--yes"),
        "{out}"
    );
    assert_eq!(names(dir.path()).len(), 4);
    assert_eq!(count(dir.path(), "track_annotations"), 4);

    let (ok, out, err) = armonika(
        dir.path(),
        &["track", "purge-missing", "Main", "--older-than", "30d"],
        "",
    );
    assert!(ok, "{out}{err}");
    assert!(
        out.contains("1 tracks") && out.contains("nothing done"),
        "{out}"
    );
    assert_eq!(names(dir.path()).len(), 4);

    let (ok, _, err) = armonika(dir.path(), &["track", "purge-missing", "Nope"], "");
    assert!(!ok);
    assert!(err.contains("Nope"), "{err}");
    let (ok, _, _) = armonika(
        dir.path(),
        &["track", "purge-missing", "Main", "--older-than", "soon"],
        "",
    );
    assert!(!ok);
}

#[test]
fn purge_missing_deletes_only_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path());

    // Only what is missing for 30 days or more.
    let (ok, out, err) = armonika(
        dir.path(),
        &[
            "track",
            "purge-missing",
            "Main",
            "--older-than",
            "30d",
            "--yes",
        ],
        "",
    );
    assert!(ok, "{out}{err}");
    assert!(out.contains("1 missing tracks deleted"), "{out}");
    assert_eq!(names(dir.path()), ["present", "recent", "other"]);

    // Then every missing track of the library; other libraries and present tracks stay.
    let (ok, out, err) = armonika(dir.path(), &["track", "purge-missing", "Main", "--yes"], "");
    assert!(ok, "{out}{err}");
    assert!(
        out.contains("1 missing tracks deleted") && out.contains("1 annotations"),
        "{out}"
    );
    assert_eq!(names(dir.path()), ["present", "other"]);
    // Personal data goes with the tracks, by cascade.
    assert_eq!(count(dir.path(), "track_annotations"), 2);
    assert_eq!(count(dir.path(), "play_history"), 2);
    assert_eq!(count(dir.path(), "playlist_entries"), 2);
    assert_eq!(count(dir.path(), "playlists"), 1);
}
