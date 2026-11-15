mod common;

use common::*;
use serde_json::Value;

fn ann() -> String {
    login("ann", ANN_PASSWORD)
}

fn admin() -> String {
    login("admin", ADMIN_PASSWORD)
}

async fn call(app: &App, method: &str, login: &str, extra: &str) -> Value {
    get(app, &format!("/rest/{method}?{login}&{extra}"))
        .await
        .json()
}

/// Ids of the three tracks of library A.
async fn songs(app: &App) -> Vec<String> {
    let v = call(app, "search3", &admin(), "query=&songCount=10").await;
    let mut ids: Vec<(String, String)> = v["searchResult3"]["song"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["title"].as_str().unwrap().to_owned(),
                s["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    ids.sort();
    ids.into_iter().map(|(_, id)| id).collect()
}

fn want(s: &[String], idx: &[usize]) -> Vec<String> {
    idx.iter().map(|&i| s[i].clone()).collect()
}

fn entry_ids(v: &Value) -> Vec<String> {
    v["playlist"]["entry"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| e["id"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn create_update_get_playlist_roundtrip() {
    let app = app().await;
    let s = songs(&app).await;
    let v = call(
        &app,
        "createPlaylist",
        &ann(),
        &format!("name=Mix&songId={}&songId={}", s[0], s[1]),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    let id = v["playlist"]["id"].as_str().unwrap().to_owned();
    assert_eq!(v["playlist"]["name"], "Mix");
    assert_eq!(v["playlist"]["songCount"], 2);
    assert_eq!(v["playlist"]["owner"], "ann");
    assert_eq!(entry_ids(&v), want(&s, &[0, 1]));

    // Add a track and the same one again, rename, comment, make public.
    let v = call(
        &app,
        "updatePlaylist",
        &ann(),
        &format!(
            "playlistId={id}&songIdToAdd={}&songIdToAdd={}&name=Renamed&comment=hello&public=true",
            s[2], s[0]
        ),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(&app, "getPlaylist", &ann(), &format!("id={id}")).await;
    assert_eq!(entry_ids(&v), want(&s, &[0, 1, 2, 0]));
    assert_eq!(v["playlist"]["name"], "Renamed");
    assert_eq!(v["playlist"]["comment"], "hello");
    assert_eq!(v["playlist"]["public"], true);

    // Remove by index (1 is the second track).
    call(
        &app,
        "updatePlaylist",
        &ann(),
        &format!("playlistId={id}&songIndexToRemove=1"),
    )
    .await;
    let v = call(&app, "getPlaylist", &ann(), &format!("id={id}")).await;
    assert_eq!(entry_ids(&v), want(&s, &[0, 2, 0]));

    // createPlaylist with playlistId replaces the whole list.
    let v = call(
        &app,
        "createPlaylist",
        &ann(),
        &format!("playlistId={id}&songId={}&songId={}", s[2], s[1]),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    assert_eq!(entry_ids(&v), want(&s, &[2, 1]));

    // The public playlist shows in the list of the admin, who cannot see it as theirs.
    let v = call(&app, "getPlaylists", &admin(), "").await;
    let list = v["playlists"]["playlist"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{v}");
    assert_eq!(list[0]["id"], id.as_str());
    assert_eq!(list[0]["owner"], "ann");

    let v = call(&app, "deletePlaylist", &ann(), &format!("id={id}")).await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(&app, "getPlaylist", &ann(), &format!("id={id}")).await;
    assert_eq!(v["error"]["code"], 70, "{v}");
}

#[tokio::test]
async fn update_by_other_user_is_50() {
    let app = app().await;
    let s = songs(&app).await;
    let v = call(
        &app,
        "createPlaylist",
        &ann(),
        &format!("name=Mine&songId={}", s[0]),
    )
    .await;
    let id = v["playlist"]["id"].as_str().unwrap().to_owned();

    // Private: the admin does not even see it.
    let v = call(
        &app,
        "updatePlaylist",
        &admin(),
        &format!("playlistId={id}&name=x"),
    )
    .await;
    assert_eq!(v["error"]["code"], 70, "{v}");
    let v = call(&app, "getPlaylist", &admin(), &format!("id={id}")).await;
    assert_eq!(v["error"]["code"], 70, "{v}");

    // Public: readable, not writable.
    call(
        &app,
        "updatePlaylist",
        &ann(),
        &format!("playlistId={id}&public=true"),
    )
    .await;
    let v = call(&app, "getPlaylist", &admin(), &format!("id={id}")).await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(
        &app,
        "updatePlaylist",
        &admin(),
        &format!("playlistId={id}&name=x"),
    )
    .await;
    assert_eq!(v["error"]["code"], 50, "{v}");
    let v = call(&app, "deletePlaylist", &admin(), &format!("id={id}")).await;
    assert_eq!(v["error"]["code"], 50, "{v}");
    let v = call(
        &app,
        "createPlaylist",
        &admin(),
        &format!("playlistId={id}&songId={}", s[1]),
    )
    .await;
    assert_eq!(v["error"]["code"], 50, "{v}");
    let v = call(&app, "getPlaylist", &ann(), &format!("id={id}")).await;
    assert_eq!(v["playlist"]["name"], "Mine");
}

#[tokio::test]
async fn tracks_ann_cannot_see_are_refused_and_playlists_need_a_name() {
    let app = app().await;
    let s = songs(&app).await;
    let v = call(&app, "createPlaylist", &ann(), &format!("songId={}", s[0])).await;
    assert_eq!(v["error"]["code"], 10, "{v}");
    let v = call(
        &app,
        "createPlaylist",
        &ann(),
        "name=Bad&songId=t-doesnotexist",
    )
    .await;
    assert_eq!(v["error"]["code"], 70, "{v}");
    let v = call(&app, "getPlaylists", &ann(), "").await;
    assert!(
        v["playlists"]["playlist"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{v}"
    );
}

#[tokio::test]
async fn save_and_get_play_queue_roundtrip() {
    let app = app().await;
    let s = songs(&app).await;
    let v = call(&app, "getPlayQueue", &ann(), "").await;
    assert_eq!(v["status"], "ok", "{v}");
    assert!(v["playQueue"].is_null(), "{v}");

    let v = call(
        &app,
        "savePlayQueue",
        &ann(),
        &format!(
            "id={}&id={}&id={}&current={}&position=1234",
            s[0], s[1], s[2], s[1]
        ),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(&app, "getPlayQueue", &ann(), "").await;
    let q = &v["playQueue"];
    assert_eq!(q["current"], s[1].as_str(), "{v}");
    assert_eq!(q["position"], 1234);
    assert_eq!(q["username"], "ann");
    // The player name is the `c` of the login.
    assert_eq!(q["changedBy"], "test");
    let ids: Vec<&str> = q["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, want(&s, &[0, 1, 2]));

    // A track the caller cannot see is left out and the rest is saved.
    let v = call(
        &app,
        "savePlayQueue",
        &ann(),
        &format!("id={}&id=t-doesnotexist&id={}&current={}", s[2], s[0], s[0]),
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    let v = call(&app, "getPlayQueue", &ann(), "").await;
    let ids: Vec<&str> = v["playQueue"]["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, want(&s, &[2, 0]), "{v}");
    assert_eq!(v["playQueue"]["current"], s[0].as_str(), "{v}");

    // Without ids the queue is cleared.
    call(&app, "savePlayQueue", &ann(), "").await;
    let v = call(&app, "getPlayQueue", &ann(), "").await;
    assert!(v["playQueue"].is_null(), "{v}");
}
