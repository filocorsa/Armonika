mod common;

use std::sync::Arc;

use armonika_scrobble::listenbrainz::ListenBrainz;
use armonika_scrobble::{ScrobbleService, ScrobbleSink, accounts};
use common::setup;
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const NOW: i64 = 1_800_000_000_000;

async fn server_with(status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/1/submit-listens"))
        .respond_with(ResponseTemplate::new(status))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn no_linked_service_writes_no_rows() {
    let f = setup();
    let (user, track) = (f.user.clone(), f.track.clone());
    let sink = ScrobbleSink::default();
    f.db.write_blocking(|c| sink.submitted(c, &user, &track, NOW))
        .unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);

    f.link("tok");
    let (user, track) = (f.user.clone(), f.track.clone());
    f.db.write_blocking(|c| sink.submitted(c, &user, &track, NOW))
        .unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 1);
}

#[tokio::test]
async fn tokens_are_stored_encrypted() {
    let f = setup();
    f.link("super-secret-token");
    let raw: Vec<u8> =
        f.db.read_blocking(|c| {
            Ok(c.query_row("SELECT secret FROM scrobble_accounts", [], |r| r.get(0))?)
        })
        .unwrap();
    let needle = b"super-secret-token";
    assert!(!raw.windows(needle.len()).any(|w| w == needle));
    let (user, sb) = (f.user.clone(), f.sb.clone());
    let token =
        f.db.read_blocking(|c| accounts::token(c, &sb, &user, "listenbrainz"))
            .unwrap();
    assert_eq!(token.as_deref(), Some("super-secret-token"));
}

#[tokio::test]
async fn unlink_removes_token_and_pending_rows() {
    let f = setup();
    f.link("tok");
    f.enqueue(2, NOW);
    let user = f.user.clone();
    let had =
        f.db.write_blocking(|c| accounts::unlink(c, &user, "listenbrainz"))
            .unwrap();
    assert!(had);
    assert_eq!(f.count("SELECT count(*) FROM scrobble_accounts"), 0);
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);
}

#[tokio::test]
async fn outbox_sends_due_rows_and_deletes_them() {
    let f = setup();
    f.link("tok");
    f.enqueue(3, NOW - 1000);
    let server = server_with(200).await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["payload"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn outbox_sends_in_batches_of_100() {
    let f = setup();
    f.link("tok");
    f.enqueue(101, NOW - 1_000_000);
    let server = server_with(200).await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let sizes: Vec<usize> = requests
        .iter()
        .map(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            b["payload"].as_array().unwrap().len()
        })
        .collect();
    assert_eq!(sizes, vec![100, 1]);
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);
}

#[tokio::test]
async fn outbox_retries_after_5xx() {
    let f = setup();
    f.link("tok");
    f.enqueue(1, NOW - 1000);
    let server = server_with(503).await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 1);
    assert_eq!(f.count("SELECT attempts FROM scrobble_outbox"), 1);
    assert_eq!(
        f.count("SELECT next_attempt_at FROM scrobble_outbox"),
        NOW + 60_000
    );

    // Not due yet: nothing is sent.
    worker.process_due(NOW + 59_999).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    // Due again, and the server is back: the row is sent and removed.
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    worker.process_due(NOW + 60_000).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);
}

#[tokio::test]
async fn outbox_backoff_doubles_up_to_six_hours() {
    let f = setup();
    f.link("tok");
    f.enqueue(1, NOW - 1000);
    let server = server_with(500).await;
    let mut worker = f.worker(&server.uri());
    let mut now = NOW;
    let mut waits = Vec::new();
    for _ in 0..12 {
        worker.process_due(now).await.unwrap();
        let next = f.count("SELECT next_attempt_at FROM scrobble_outbox");
        waits.push(next - now);
        now = next;
    }
    assert_eq!(waits[0], 60_000);
    assert_eq!(waits[1], 120_000);
    assert_eq!(waits[2], 240_000);
    assert_eq!(*waits.last().unwrap(), 6 * 60 * 60 * 1000);
    assert!(waits.windows(2).all(|w| w[0] <= w[1]));
}

#[tokio::test]
async fn outbox_keeps_rows_on_auth_error() {
    let f = setup();
    f.link("revoked");
    f.enqueue(2, NOW - 1000);
    let server = server_with(401).await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 2);
    assert_eq!(f.count("SELECT count(*) FROM scrobble_accounts"), 1);

    // After the user links a new token the rows are due at once and go out.
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    f.link("fresh");
    worker.process_due(NOW + 1).await.unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Token fresh"
    );
}

#[tokio::test]
async fn outbox_drops_after_14_days() {
    let f = setup();
    f.link("tok");
    f.enqueue(1, NOW - 15 * DAY_MS);
    f.enqueue(1, NOW - 13 * DAY_MS);
    let server = server_with(500).await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    // The old play is gone without being sent; the recent one is retried.
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 1);
    let body: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["payload"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn now_playing_is_sent_once_without_retry() {
    let f = setup();
    f.link("tok");
    let server = server_with(500).await;
    let svc: Arc<dyn ScrobbleService> = Arc::new(ListenBrainz::new(&server.uri()));
    let sink = ScrobbleSink::new(f.db.clone(), f.sb.clone(), vec![svc]);
    sink.now_playing(&f.user, &f.track).await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["listen_type"], "playing_now");
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);
}

/// Points the outbox row played at `played_at` to a track titled "Bad".
fn make_bad(f: &common::Fixture, played_at: i64) {
    f.db.write_blocking(|c| {
        c.execute_batch(
            "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms, tag_schema,
               format, title, artist_credit, added_at, updated_at)
             SELECT 'bad', library_id, state, 'bad.flac', size, mtime_ms, tag_schema, format,
               'Bad', 'Artist', added_at, updated_at FROM tracks LIMIT 1;",
        )?;
        let bad: i64 = c.query_row("SELECT id FROM tracks WHERE public_id = 'bad'", [], |r| {
            r.get(0)
        })?;
        c.execute(
            "UPDATE scrobble_outbox SET track_id = ?1 WHERE played_at = ?2",
            [bad, played_at],
        )?;
        Ok(())
    })
    .unwrap();
}

/// Answers 400 to any request whose payload holds a track named "Bad", 200 otherwise.
struct RejectsBad;

impl wiremock::Respond for RejectsBad {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let bad = body["payload"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["track_metadata"]["track_name"] == "Bad");
        ResponseTemplate::new(if bad { 400 } else { 200 })
    }
}

#[tokio::test]
async fn outbox_rejected_batch_drops_only_the_bad_listen() {
    let f = setup();
    f.link("tok");
    f.enqueue(7, NOW - 1_000_000);
    // The play with the third time stamp is the bad one: give its track a bad title.
    f.db.write_blocking(|c| {
        c.execute_batch(
            "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms, tag_schema,
               format, title, artist_credit, added_at, updated_at)
             SELECT 'bad', library_id, state, 'bad.flac', size, mtime_ms, tag_schema, format,
               'Bad', 'Artist', added_at, updated_at FROM tracks LIMIT 1;",
        )?;
        let bad: i64 = c.query_row("SELECT id FROM tracks WHERE public_id = 'bad'", [], |r| {
            r.get(0)
        })?;
        c.execute(
            "UPDATE scrobble_outbox SET track_id = ?1 WHERE played_at = ?2",
            [bad, NOW - 1_000_000 + 2],
        )?;
        Ok(())
    })
    .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(RejectsBad)
        .mount(&server)
        .await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);
    // Delivered: every good listen, in some request that was accepted.
    let mut delivered = 0;
    for r in server.received_requests().await.unwrap() {
        let b: Value = serde_json::from_slice(&r.body).unwrap();
        let items = b["payload"].as_array().unwrap();
        if items
            .iter()
            .all(|l| l["track_metadata"]["track_name"] != "Bad")
        {
            delivered += items.len();
        }
    }
    assert_eq!(delivered, 6);
}

#[tokio::test]
async fn outbox_rejected_singleton_is_dropped_when_another_play_was_accepted() {
    let f = setup();
    f.link("tok");
    f.enqueue(2, NOW - 1_000_000);
    make_bad(&f, NOW - 1_000_000 + 1);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(RejectsBad)
        .mount(&server)
        .await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 0);
}

#[tokio::test]
async fn outbox_refusal_of_everything_keeps_all_rows() {
    for status in [405, 410, 415, 400] {
        let f = setup();
        f.link("tok");
        f.enqueue(3, NOW - 1000);
        let server = server_with(status).await;
        let mut worker = f.worker(&server.uri());
        worker.process_due(NOW).await.unwrap();
        assert_eq!(
            f.count("SELECT count(*) FROM scrobble_outbox"),
            3,
            "{status}"
        );
        assert_eq!(f.count("SELECT min(attempts) FROM scrobble_outbox"), 1);
        assert_eq!(
            f.count("SELECT min(next_attempt_at) FROM scrobble_outbox"),
            NOW + 60_000
        );
    }
}

#[tokio::test]
async fn outbox_one_broken_group_does_not_block_another() {
    let f = setup();
    // The first user (lowest key, so first in the pass) has a token that cannot be opened.
    f.link("tok");
    f.enqueue(1, NOW - 1000);
    f.db.write_blocking(|c| {
        c.execute("UPDATE scrobble_accounts SET secret = x'0102'", [])?;
        Ok(())
    })
    .unwrap();
    let sb = f.sb.clone();
    let other =
        f.db.write_blocking(|c| {
            armonika_core::users::create(
                c,
                &sb,
                armonika_core::users::NewUser {
                    name: "bob".to_owned(),
                    password: "bob-password".to_owned(),
                    is_admin: false,
                    can_download: true,
                },
            )
        })
        .unwrap();
    let (sb, user, track) = (f.sb.clone(), other.clone(), f.track.clone());
    f.db.write_blocking(|c| {
        armonika_scrobble::accounts::link(c, &sb, &user, "listenbrainz", "bobtok", None)?;
        ScrobbleSink::default().submitted(c, &user, &track, NOW - 2000)
    })
    .unwrap();
    let server = server_with(200).await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Token bobtok"
    );
    // The broken group keeps its row.
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 1);
}

#[tokio::test]
async fn linking_again_resets_attempts() {
    let f = setup();
    f.link("tok");
    f.enqueue(1, NOW - 1000);
    let server = server_with(503).await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    assert_eq!(f.count("SELECT attempts FROM scrobble_outbox"), 1);
    f.link("new");
    assert_eq!(f.count("SELECT attempts FROM scrobble_outbox"), 0);
    assert_eq!(f.count("SELECT next_attempt_at FROM scrobble_outbox"), 0);
}

#[test]
fn warn_literals_have_no_stray_runs_of_spaces() {
    let source = include_str!("../src/outbox.rs");
    assert!(!source.contains("the  ") && !source.contains("until the  "));
    assert!(!source.contains("the                 "));
}

#[tokio::test]
async fn outbox_404_keeps_rows_and_schedules_a_retry() {
    let f = setup();
    f.link("tok");
    f.enqueue(2, NOW - 1000);
    let server = server_with(404).await;
    let mut worker = f.worker(&server.uri());
    worker.process_due(NOW).await.unwrap();
    assert_eq!(f.count("SELECT count(*) FROM scrobble_outbox"), 2);
    assert_eq!(f.count("SELECT min(attempts) FROM scrobble_outbox"), 1);
    assert_eq!(
        f.count("SELECT min(next_attempt_at) FROM scrobble_outbox"),
        NOW + 60_000
    );
}
