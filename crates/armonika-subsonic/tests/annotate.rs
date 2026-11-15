mod common;

use armonika_core::access;
use armonika_core::users::{self, UserFlagsUpdate};
use armonika_testdata::fixture::FixtureTags;
use common::*;
use serde_json::Value;

fn ann() -> String {
    login("ann", ANN_PASSWORD)
}

fn admin() -> String {
    login("admin", ADMIN_PASSWORD)
}

fn song(title: &str, artist: &str, album: &str) -> FixtureTags {
    FixtureTags {
        title: Some(title.to_owned()),
        artist_credit: Some(artist.to_owned()),
        artists: vec![artist.to_owned()],
        album_title: Some(album.to_owned()),
        album_artist_credit: Some(artist.to_owned()),
        album_artists: vec![artist.to_owned()],
        ..FixtureTags::default()
    }
}

async fn call(app: &App, method: &str, login: &str, extra: &str) -> Value {
    get(app, &format!("/rest/{method}?{login}&{extra}"))
        .await
        .json()
}

/// Ids of the first song of the library as admin sees it.
struct Ids {
    track: String,
    album: String,
    artist: String,
}

async fn first_song(app: &App) -> Ids {
    let v = call(app, "search3", &admin(), "query=").await;
    let s = &v["searchResult3"]["song"][0];
    assert!(s["id"].is_string(), "{v}");
    Ids {
        track: s["id"].as_str().unwrap().to_owned(),
        album: s["albumId"].as_str().unwrap().to_owned(),
        artist: s["artistId"].as_str().unwrap().to_owned(),
    }
}

async fn app_with_song() -> (App, Ids) {
    let app = app_empty().await;
    app.put(&app.music_a, "a/one.mp3", &song("One", "Queen", "Opera"));
    app.scan();
    let ids = first_song(&app).await;
    (app, ids)
}

fn list_len(v: &Value, node: &str, kind: &str) -> usize {
    v[node][kind].as_array().map_or(0, Vec::len)
}

async fn count(app: &App, sql: &'static str) -> i64 {
    app.db
        .read(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?))
        .await
        .unwrap()
}

#[tokio::test]
async fn star_track_album_artist_and_list() {
    let (app, ids) = app_with_song().await;
    let extra = format!(
        "id={}&albumId={}&artistId={}",
        ids.track, ids.album, ids.artist
    );
    let v = call(&app, "star", &ann(), &extra).await;
    assert_eq!(v["status"], "ok", "{v}");

    for (method, node) in [("getStarred2", "starred2"), ("getStarred", "starred")] {
        let v = call(&app, method, &ann(), "").await;
        assert_eq!(v["status"], "ok", "{v}");
        assert_eq!(v[node]["song"][0]["id"], ids.track.as_str(), "{v}");
        assert!(v[node]["song"][0]["starred"].is_string(), "{v}");
        assert_eq!(v[node]["album"][0]["id"], ids.album.as_str(), "{v}");
        assert_eq!(v[node]["artist"][0]["id"], ids.artist.as_str(), "{v}");
    }

    // Repeated ids work too, and the star shows on the item itself.
    let v = call(&app, "getSong", &ann(), &format!("id={}", ids.track)).await;
    assert!(v["song"]["starred"].is_string(), "{v}");

    let v = call(&app, "unstar", &ann(), &extra).await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(&app, "getStarred2", &ann(), "").await;
    for kind in ["song", "album", "artist"] {
        assert_eq!(list_len(&v, "starred2", kind), 0, "{v}");
    }
}

#[tokio::test]
async fn star_accepts_repeated_ids() {
    let app = app_empty().await;
    app.put(&app.music_a, "a/one.mp3", &song("One", "Queen", "Opera"));
    app.put(&app.music_a, "a/two.mp3", &song("Two", "Queen", "Opera"));
    app.scan();
    let v = call(&app, "search3", &admin(), "query=").await;
    let a = v["searchResult3"]["song"][0]["id"].as_str().unwrap();
    let b = v["searchResult3"]["song"][1]["id"].as_str().unwrap();
    let v = call(&app, "star", &ann(), &format!("id={a}&id={b}")).await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(&app, "getStarred2", &ann(), "").await;
    assert_eq!(list_len(&v, "starred2", "song"), 2, "{v}");
}

#[tokio::test]
async fn stars_are_per_user() {
    let (app, ids) = app_with_song().await;
    let v = call(&app, "star", &admin(), &format!("id={}", ids.track)).await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(&app, "getStarred2", &ann(), "").await;
    assert_eq!(list_len(&v, "starred2", "song"), 0, "{v}");
    let v = call(&app, "getSong", &ann(), &format!("id={}", ids.track)).await;
    assert!(v["song"]["starred"].is_null(), "{v}");
    let v = call(&app, "getStarred2", &admin(), "").await;
    assert_eq!(list_len(&v, "starred2", "song"), 1, "{v}");
}

#[tokio::test]
async fn rating_zero_removes_rating() {
    let (app, ids) = app_with_song().await;
    for id in [&ids.track, &ids.album, &ids.artist] {
        let v = call(&app, "setRating", &ann(), &format!("id={id}&rating=4")).await;
        assert_eq!(v["status"], "ok", "{v}");
    }
    let v = call(&app, "getSong", &ann(), &format!("id={}", ids.track)).await;
    assert_eq!(v["song"]["userRating"], 4, "{v}");
    let v = call(&app, "getAlbum", &ann(), &format!("id={}", ids.album)).await;
    assert_eq!(v["album"]["userRating"], 4, "{v}");
    let v = call(&app, "getArtist", &ann(), &format!("id={}", ids.artist)).await;
    assert_eq!(v["artist"]["userRating"], 4, "{v}");

    for id in [&ids.track, &ids.album, &ids.artist] {
        let v = call(&app, "setRating", &ann(), &format!("id={id}&rating=0")).await;
        assert_eq!(v["status"], "ok", "{v}");
    }
    let v = call(&app, "getSong", &ann(), &format!("id={}", ids.track)).await;
    assert!(v["song"]["userRating"].is_null(), "{v}");

    let v = call(
        &app,
        "setRating",
        &ann(),
        &format!("id={}&rating=6", ids.track),
    )
    .await;
    assert_eq!(v["status"], "failed", "{v}");
    let v = call(&app, "setRating", &ann(), &format!("id={}", ids.track)).await;
    assert_eq!(v["error"]["code"], 10, "{v}");
}

#[tokio::test]
async fn scrobble_submission_counts_play() {
    let (app, ids) = app_with_song().await;
    // Submission is the default; `time` is when it was played, in milliseconds.
    let v = call(
        &app,
        "scrobble",
        &ann(),
        &format!("id={}&time=1700000000000", ids.track),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(
        &app,
        "scrobble",
        &ann(),
        &format!("id={}&submission=true&time=1700000005000", ids.track),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");

    let v = call(&app, "getSong", &ann(), &format!("id={}", ids.track)).await;
    assert_eq!(v["song"]["playCount"], 2, "{v}");
    assert!(v["song"]["played"].is_string(), "{v}");
    assert_eq!(count(&app, "SELECT count(*) FROM play_history").await, 2);
    assert_eq!(
        count(&app, "SELECT max(played_at) FROM play_history").await,
        1_700_000_005_000
    );
    // A submitted play is not "now playing".
    assert_eq!(count(&app, "SELECT count(*) FROM now_playing").await, 0);
}

#[tokio::test]
async fn scrobble_time_in_the_future_is_clamped_to_now() {
    let (app, ids) = app_with_song().await;
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let v = call(
        &app,
        "scrobble",
        &ann(),
        &format!("id={}&time=99999999999999", ids.track),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let played = count(&app, "SELECT max(played_at) FROM play_history").await;
    assert!((before..=after).contains(&played), "{played}");
}

#[tokio::test]
async fn scrobble_now_playing_does_not_count() {
    let (app, ids) = app_with_song().await;
    let v = call(
        &app,
        "scrobble",
        &ann(),
        &format!("id={}&submission=false", ids.track),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(&app, "getSong", &ann(), &format!("id={}", ids.track)).await;
    assert!(v["song"]["playCount"].is_null(), "{v}");
    assert_eq!(count(&app, "SELECT count(*) FROM play_history").await, 0);
    assert_eq!(count(&app, "SELECT count(*) FROM now_playing").await, 1);
}

#[tokio::test]
async fn now_playing_private_by_default() {
    let (app, ids) = app_with_song().await;
    call(
        &app,
        "scrobble",
        &ann(),
        &format!("id={}&submission=false", ids.track),
    )
    .await;
    // Their own player always shows.
    let v = call(&app, "getNowPlaying", &ann(), "").await;
    assert_eq!(v["status"], "ok", "{v}");
    assert_eq!(list_len(&v, "nowPlaying", "entry"), 1, "{v}");
    assert_eq!(v["nowPlaying"]["entry"][0]["username"], "ann", "{v}");
    assert_eq!(v["nowPlaying"]["entry"][0]["id"], ids.track.as_str(), "{v}");
    // Others see nothing until she opts in.
    let v = call(&app, "getNowPlaying", &admin(), "").await;
    assert_eq!(v["status"], "ok", "{v}");
    assert_eq!(list_len(&v, "nowPlaying", "entry"), 0, "{v}");
}

async fn set_share(app: &App, name: &'static str, share: bool) {
    app.db
        .write(move |c| {
            let user = users::by_name(c, name)?.unwrap().id;
            users::set_flags(
                c,
                &user,
                UserFlagsUpdate {
                    share_now_playing: Some(share),
                    ..UserFlagsUpdate::default()
                },
            )
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn now_playing_shared_when_enabled() {
    let (app, ids) = app_with_song().await;
    set_share(&app, "ann", true).await;
    call(
        &app,
        "scrobble",
        &ann(),
        &format!("id={}&submission=false", ids.track),
    )
    .await;
    let v = call(&app, "getNowPlaying", &admin(), "").await;
    assert_eq!(list_len(&v, "nowPlaying", "entry"), 1, "{v}");
    let entry = &v["nowPlaying"]["entry"][0];
    assert_eq!(entry["username"], "ann", "{v}");
    assert_eq!(entry["minutesAgo"], 0, "{v}");
    assert_eq!(entry["playerName"], "test", "{v}");

    // A stale entry (older than ten minutes) is not shown.
    app.exec("UPDATE now_playing SET started_at = started_at - 11 * 60 * 1000")
        .await;
    let v = call(&app, "getNowPlaying", &admin(), "").await;
    assert_eq!(list_len(&v, "nowPlaying", "entry"), 0, "{v}");
}

#[tokio::test]
async fn now_playing_hides_tracks_the_viewer_cannot_see() {
    let app = app_empty().await;
    app.put(&app.music_b, "b/one.mp3", &song("One", "Queen", "Opera"));
    app.scan();
    set_share(&app, "admin", true).await;
    let v = call(&app, "search3", &admin(), "query=").await;
    let id = v["searchResult3"]["song"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    call(
        &app,
        "scrobble",
        &admin(),
        &format!("id={id}&submission=false"),
    )
    .await;
    // Ann has no access to library B.
    let v = call(&app, "getNowPlaying", &ann(), "").await;
    assert_eq!(list_len(&v, "nowPlaying", "entry"), 0, "{v}");
    let v = call(&app, "getNowPlaying", &admin(), "").await;
    assert_eq!(list_len(&v, "nowPlaying", "entry"), 1, "{v}");
}

#[tokio::test]
async fn annotations_survive_access_loss_and_return() {
    let (app, ids) = app_with_song().await;
    let v = call(&app, "star", &ann(), &format!("id={}", ids.track)).await;
    assert_eq!(v["status"], "ok", "{v}");
    let (lib, user) = (app.lib_a, "ann");
    let toggle = |grant: bool| {
        let db = app.db.clone();
        async move {
            db.write(move |c| {
                let id = users::by_name(c, user)?.unwrap().id;
                if grant {
                    access::grant(c, &id, lib, access::Permission::Listen)
                } else {
                    access::revoke(c, &id, lib).map(|_| ())
                }
            })
            .await
            .unwrap();
        }
    };

    toggle(false).await;
    let v = call(&app, "getStarred2", &ann(), "").await;
    assert_eq!(list_len(&v, "starred2", "song"), 0, "{v}");
    assert_eq!(list_len(&v, "starred2", "album"), 0, "{v}");
    assert_eq!(list_len(&v, "starred2", "artist"), 0, "{v}");
    // Starring what she cannot see is refused too.
    let v = call(&app, "star", &ann(), &format!("id={}", ids.track)).await;
    assert_eq!(v["error"]["code"], 70, "{v}");

    toggle(true).await;
    let v = call(&app, "getStarred2", &ann(), "").await;
    assert_eq!(v["starred2"]["song"][0]["id"], ids.track.as_str(), "{v}");
}

#[tokio::test]
async fn star_invisible_track_is_70() {
    let app = app_empty().await;
    app.put(&app.music_a, "a/one.mp3", &song("One", "Queen", "Opera"));
    app.put(&app.music_b, "b/two.mp3", &song("Two", "Other", "Else"));
    app.scan();
    let v = call(&app, "search3", &admin(), "query=").await;
    let songs = v["searchResult3"]["song"].as_array().unwrap();
    let id_of = |title: &str| {
        songs.iter().find(|s| s["title"] == title).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (mine, hidden) = (id_of("One"), id_of("Two"));

    let v = call(&app, "star", &ann(), &format!("id={hidden}")).await;
    assert_eq!(v["status"], "failed", "{v}");
    assert_eq!(v["error"]["code"], 70, "{v}");
    // One bad id in the list: nothing is written, not even the good one.
    let v = call(&app, "star", &ann(), &format!("id={mine}&id={hidden}")).await;
    assert_eq!(v["error"]["code"], 70, "{v}");
    assert_eq!(
        count(&app, "SELECT count(*) FROM track_annotations").await,
        0
    );
    let v = call(&app, "star", &ann(), "id=t-doesnotexist").await;
    assert_eq!(v["error"]["code"], 70, "{v}");
    let v = call(&app, "setRating", &ann(), &format!("id={hidden}&rating=3")).await;
    assert_eq!(v["error"]["code"], 70, "{v}");

    for extra in [
        format!("id={hidden}"),
        format!("id={mine}&id={hidden}&submission=false"),
    ] {
        let v = call(&app, "scrobble", &ann(), &extra).await;
        assert_eq!(v["error"]["code"], 70, "{v}");
    }
    assert_eq!(
        count(&app, "SELECT count(*) FROM track_annotations").await,
        0
    );
    assert_eq!(count(&app, "SELECT count(*) FROM play_history").await, 0);
    assert_eq!(count(&app, "SELECT count(*) FROM now_playing").await, 0);
}

#[tokio::test]
async fn scrobble_queues_outbox_rows_with_the_plays_for_linked_services() {
    let (app, ids) = app_with_song().await;
    let extra = format!(
        "id={0}&id={0}&time=1700000000000&time=1700000005000",
        ids.track
    );

    // No linked service: the plays count and nothing is queued.
    let v = call(&app, "scrobble", &ann(), &extra).await;
    assert_eq!(v["status"], "ok", "{v}");
    assert_eq!(count(&app, "SELECT count(*) FROM play_history").await, 2);
    assert_eq!(count(&app, "SELECT count(*) FROM scrobble_outbox").await, 0);

    app.exec(
        "INSERT INTO scrobble_accounts (user_id, service, secret)
         SELECT id, 'listenbrainz', x'00' FROM users WHERE name = 'ann'",
    )
    .await;
    let v = call(&app, "scrobble", &ann(), &extra).await;
    assert_eq!(v["status"], "ok", "{v}");
    assert_eq!(count(&app, "SELECT count(*) FROM play_history").await, 4);
    assert_eq!(count(&app, "SELECT count(*) FROM scrobble_outbox").await, 2);
    assert_eq!(
        count(&app, "SELECT min(played_at) FROM scrobble_outbox").await,
        1_700_000_000_000
    );

    // Now playing is not a play: nothing is queued.
    let v = call(
        &app,
        "scrobble",
        &ann(),
        &format!("id={}&submission=false", ids.track),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    assert_eq!(count(&app, "SELECT count(*) FROM scrobble_outbox").await, 2);
}
