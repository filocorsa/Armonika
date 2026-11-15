use armonika_scrobble::listenbrainz::ListenBrainz;
use armonika_scrobble::{Listen, ScrobbleService, SubmitError};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn listen(n: i64) -> Listen {
    Listen {
        listened_at: Some(1_700_000_000 + n),
        title: "Song".to_owned(),
        artist: "Artist feat. Guest".to_owned(),
        album: Some("Album".to_owned()),
        duration_ms: Some(215_000),
        track_number: Some(3),
        recording_mbid: Some("11111111-2222-3333-4444-555555555555".to_owned()),
    }
}

async fn call<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

async fn body_of(server: &MockServer, i: usize) -> Value {
    let requests = server.received_requests().await.unwrap();
    serde_json::from_slice(&requests[i].body).unwrap()
}

#[tokio::test]
async fn listenbrainz_submit_payload_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/1/submit-listens"))
        .and(header("Authorization", "Token secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
        .expect(1)
        .mount(&server)
        .await;
    let lb = ListenBrainz::new(&server.uri());
    call(move || lb.submit("secret-token", &[listen(0)]))
        .await
        .unwrap();

    let body = body_of(&server, 0).await;
    assert_eq!(body["listen_type"], "single");
    let item = &body["payload"][0];
    assert_eq!(item["listened_at"], 1_700_000_000);
    let meta = &item["track_metadata"];
    assert_eq!(meta["artist_name"], "Artist feat. Guest");
    assert_eq!(meta["track_name"], "Song");
    assert_eq!(meta["release_name"], "Album");
    let info = &meta["additional_info"];
    assert_eq!(
        info["recording_mbid"],
        "11111111-2222-3333-4444-555555555555"
    );
    assert_eq!(info["duration_ms"], 215_000);
    assert_eq!(info["tracknumber"], 3);
    assert_eq!(info["submission_client"], "Armonika");
    let requests = server.received_requests().await.unwrap();
    let agent = requests[0]
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        agent,
        format!(
            "Armonika/{} ( https://github.com/filocorsa/Armonika )",
            armonika_core::VERSION
        )
    );
}

#[tokio::test]
async fn listenbrainz_submit_omits_unknown_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut l = listen(0);
    l.recording_mbid = None;
    l.album = None;
    let lb = ListenBrainz::new(&server.uri());
    call(move || lb.submit("t", &[l])).await.unwrap();
    let body = body_of(&server, 0).await;
    let meta = &body["payload"][0]["track_metadata"];
    assert!(meta.get("release_name").is_none());
    assert!(meta["additional_info"].get("recording_mbid").is_none());
}

#[tokio::test]
async fn listenbrainz_now_playing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/1/submit-listens"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let mut l = listen(0);
    l.listened_at = None;
    let lb = ListenBrainz::new(&server.uri());
    call(move || lb.now_playing("t", &l)).await.unwrap();
    let body = body_of(&server, 0).await;
    assert_eq!(body["listen_type"], "playing_now");
    assert!(body["payload"][0].get("listened_at").is_none());
    assert_eq!(body["payload"][0]["track_metadata"]["track_name"], "Song");
}

#[tokio::test]
async fn listenbrainz_batch_of_100() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let batch: Vec<Listen> = (0..100).map(listen).collect();
    let lb = ListenBrainz::new(&server.uri());
    assert_eq!(lb.max_batch(), 100);
    call(move || lb.submit("t", &batch)).await.unwrap();
    let body = body_of(&server, 0).await;
    assert_eq!(body["listen_type"], "import");
    assert_eq!(body["payload"].as_array().unwrap().len(), 100);
}

#[tokio::test]
async fn listenbrainz_maps_status_to_errors() {
    for (status, expected) in [
        (500, SubmitError::Retry),
        (429, SubmitError::Retry),
        (401, SubmitError::Auth),
        (400, SubmitError::Rejected),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let lb = ListenBrainz::new(&server.uri());
        let got = call(move || lb.submit("t", &[listen(0)])).await;
        assert_eq!(got, Err(expected), "status {status}");
    }
    // Nothing listens: a network error is worth a retry.
    let lb = ListenBrainz::new("http://127.0.0.1:9");
    assert_eq!(
        call(move || lb.submit("t", &[listen(0)])).await,
        Err(SubmitError::Retry)
    );
}

#[tokio::test]
async fn listenbrainz_validate_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/1/validate-token"))
        .and(header("Authorization", "Token good"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code": 200, "valid": true, "user_name": "annlb"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/1/validate-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"code": 200, "valid": false})),
        )
        .mount(&server)
        .await;
    let lb = ListenBrainz::new(&server.uri());
    let lb2 = ListenBrainz::new(&server.uri());
    assert_eq!(
        call(move || lb.validate_token("good")).await,
        Ok(Some("annlb".to_owned()))
    );
    assert_eq!(call(move || lb2.validate_token("bad")).await, Ok(None));
}

#[tokio::test]
async fn listenbrainz_drops_malformed_mbid_and_zero_duration() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(3)
        .mount(&server)
        .await;
    let mut bad_mbid = listen(0);
    bad_mbid.recording_mbid = Some("not-a-uuid".to_owned());
    let mut zero = listen(1);
    zero.duration_ms = Some(0);
    let mut upper = listen(2);
    upper.recording_mbid = Some("  AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE ".to_owned());
    let lb = ListenBrainz::new(&server.uri());
    for l in [bad_mbid, zero, upper] {
        let lb = lb.clone();
        call(move || lb.submit("t", &[l])).await.unwrap();
    }
    let requests = server.received_requests().await.unwrap();
    let infos: Vec<Value> = requests
        .iter()
        .map(|r| {
            let b: Value = serde_json::from_slice(&r.body).unwrap();
            b["payload"][0]["track_metadata"]["additional_info"].clone()
        })
        .collect();
    assert!(infos[0].get("recording_mbid").is_none());
    assert!(infos[0].get("duration_ms").is_some());
    assert!(infos[1].get("duration_ms").is_none());
    assert!(infos[1].get("recording_mbid").is_some());
    assert_eq!(
        infos[2]["recording_mbid"],
        "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    );
}
