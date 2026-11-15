#![allow(dead_code)]

use std::sync::Arc;

use armonika_core::db::Db;
use armonika_core::ids::{TrackId, UserId};
use armonika_core::model::{AudioFormat, FileFacts, TagFacts};
use armonika_core::secret::SecretBox;
use armonika_core::users::{self, NewUser};
use armonika_core::{libraries, track_files};
use armonika_scrobble::listenbrainz::ListenBrainz;
use armonika_scrobble::outbox::Worker;
use armonika_scrobble::{ScrobbleService, ScrobbleSink, accounts};

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub db: Db,
    pub sb: SecretBox,
    pub user: UserId,
    pub track: TrackId,
}

pub fn setup() -> Fixture {
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
                duration_ms: Some(215_000),
                bitrate_kbps: None,
                sample_rate: None,
                channels: None,
                bit_depth: None,
                tags: TagFacts {
                    title: Some("Song".to_owned()),
                    artist_credit: Some("Artist feat. Guest".to_owned()),
                    album_title: Some("Album".to_owned()),
                    track_no: Some(3),
                    mb_recording_id: Some("11111111-2222-3333-4444-555555555555".to_owned()),
                    ..TagFacts::default()
                },
            };
            let tx = c.transaction()?;
            let track = track_files::insert_track(&tx, lib, "one.flac", &facts, 1)?;
            tx.commit()?;
            Ok((user, track))
        })
        .unwrap();
    Fixture {
        dir,
        db,
        sb,
        user,
        track,
    }
}

impl Fixture {
    pub fn link(&self, token: &str) {
        let (user, sb) = (self.user.clone(), self.sb.clone());
        let token = token.to_owned();
        self.db
            .write_blocking(|c| accounts::link(c, &sb, &user, "listenbrainz", &token, Some("ann")))
            .unwrap();
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.db
            .read_blocking(|c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .unwrap()
    }

    /// Writes `n` outbox rows for the fixture track, played at `played_at_ms` and after.
    pub fn enqueue(&self, n: usize, played_at_ms: i64) {
        let (user, track) = (self.user.clone(), self.track.clone());
        let sink = ScrobbleSink::default();
        self.db
            .write_blocking(|c| {
                for i in 0..n {
                    sink.submitted(c, &user, &track, played_at_ms + i as i64)?;
                }
                Ok(())
            })
            .unwrap();
    }

    pub fn worker(&self, url: &str) -> Worker {
        let svc: Arc<dyn ScrobbleService> = Arc::new(ListenBrainz::new(url));
        Worker::new(self.db.clone(), self.sb.clone(), vec![svc])
    }
}
