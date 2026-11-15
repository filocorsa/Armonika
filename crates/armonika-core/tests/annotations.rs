use armonika_core::Error;
use armonika_core::annotations::{record_play, set_rating, star, unstar};
use armonika_core::db::Db;
use armonika_core::ids::{ItemRef, TrackId, UserId};
use armonika_core::model::{AudioFormat, FileFacts, TagFacts};
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, NewUser};
use armonika_core::{libraries, track_files};

fn setup() -> (tempfile::TempDir, Db, UserId, TrackId) {
    let dir = tempfile::tempdir().unwrap();
    let music = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("armonika.db")).unwrap();
    let sb = SecretBox::from_key([7; 32]);
    let (user, track) = db
        .write_blocking(|c| {
            let user = users::create(
                c,
                &sb,
                NewUser {
                    name: "ann".to_owned(),
                    password: "ann-password".to_owned(),
                    is_admin: false,
                    can_download: true,
                },
            )?;
            let lib = libraries::create(c, "A", music.path(), dir.path())?;
            let facts = FileFacts {
                size: 1,
                mtime_ms: 1,
                format: AudioFormat::Flac,
                duration_ms: Some(1000),
                bitrate_kbps: None,
                sample_rate: None,
                channels: None,
                bit_depth: None,
                tags: TagFacts::default(),
            };
            let tx = c.transaction()?;
            track_files::insert_track(&tx, lib, "one.flac", &facts, 1)?;
            tx.commit()?;
            let track = c.query_row("SELECT public_id FROM tracks", [], |r| r.get(0))?;
            Ok((user, track))
        })
        .unwrap();
    (dir, db, user, track)
}

fn one(db: &Db, sql: &str) -> Option<i64> {
    db.read_blocking(|c| Ok(c.query_row(sql, [], |r| r.get(0))?))
        .unwrap()
}

#[test]
fn star_keeps_the_first_date_and_unstar_clears_it() {
    let (_dir, db, user, track) = setup();
    let item = ItemRef::Track(track);
    db.write_blocking(|c| star(c, &user, std::slice::from_ref(&item)))
        .unwrap();
    db.write_blocking(|c| {
        c.execute_batch("UPDATE track_annotations SET starred_at = 5")
            .map_err(Into::into)
    })
    .unwrap();
    db.write_blocking(|c| star(c, &user, std::slice::from_ref(&item)))
        .unwrap();
    assert_eq!(
        one(&db, "SELECT starred_at FROM track_annotations"),
        Some(5)
    );
    db.write_blocking(|c| unstar(c, &user, std::slice::from_ref(&item)))
        .unwrap();
    assert_eq!(one(&db, "SELECT starred_at FROM track_annotations"), None);
}

#[test]
fn unknown_item_writes_nothing() {
    let (_dir, db, user, track) = setup();
    let missing: TrackId = "t-nothere".parse().unwrap();
    let err = db
        .write_blocking(|c| star(c, &user, &[ItemRef::Track(track), ItemRef::Track(missing)]))
        .unwrap_err();
    assert!(matches!(err, Error::TrackNotFound(_)), "{err}");
    assert_eq!(one(&db, "SELECT count(*) FROM track_annotations"), Some(0));
}

#[test]
fn rating_is_set_cleared_and_clamped() {
    let (_dir, db, user, track) = setup();
    let item = ItemRef::Track(track);
    db.write_blocking(|c| set_rating(c, &user, &item, 3))
        .unwrap();
    assert_eq!(one(&db, "SELECT rating FROM track_annotations"), Some(3));
    db.write_blocking(|c| set_rating(c, &user, &item, 9))
        .unwrap();
    assert_eq!(one(&db, "SELECT rating FROM track_annotations"), Some(5));
    db.write_blocking(|c| set_rating(c, &user, &item, 0))
        .unwrap();
    assert_eq!(one(&db, "SELECT rating FROM track_annotations"), None);
}

#[test]
fn record_play_counts_and_never_moves_last_played_back() {
    let (_dir, db, user, track) = setup();
    db.write_blocking(|c| record_play(c, &user, &track, 2000))
        .unwrap();
    db.write_blocking(|c| record_play(c, &user, &track, 1000))
        .unwrap();
    assert_eq!(
        one(&db, "SELECT play_count FROM track_annotations"),
        Some(2)
    );
    assert_eq!(
        one(&db, "SELECT last_played_at FROM track_annotations"),
        Some(2000)
    );
    assert_eq!(one(&db, "SELECT count(*) FROM play_history"), Some(2));
}
