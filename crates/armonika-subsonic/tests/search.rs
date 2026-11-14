mod common;

use armonika_core::model::{AudioFormat, FileFacts, TagFacts};
use armonika_core::track_files;
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

fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|a| {
            a.iter()
                .map(|s| s["id"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn titles(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|a| {
            a.iter()
                .map(|s| s["title"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn typo_query_finds_track() {
    let app = app_empty().await;
    app.put(
        &app.music_a,
        "a/one.mp3",
        &song("Bohemian Rhapsody", "Queen", "Opera"),
    );
    app.put(
        &app.music_a,
        "a/two.mp3",
        &song("Other Tune", "Someone", "Else"),
    );
    app.scan();
    app.index().await;
    for (method, node) in [("search3", "searchResult3"), ("search2", "searchResult2")] {
        let v = call(&app, method, &ann(), "query=Bohemian%20Rapsody").await;
        assert_eq!(v["status"], "ok", "{v}");
        assert_eq!(titles(&v[node]["song"]), ["Bohemian Rhapsody"], "{v}");
    }
}

#[tokio::test]
async fn empty_query_pages_whole_library() {
    let app = app_empty().await;
    let lib = app.lib_a;
    app.db
        .write(move |c| {
            let tx = c.transaction()?;
            for n in 0..1200 {
                let facts = FileFacts {
                    size: 1,
                    mtime_ms: 1,
                    format: AudioFormat::Flac,
                    duration_ms: Some(1000),
                    bitrate_kbps: None,
                    sample_rate: None,
                    channels: None,
                    bit_depth: None,
                    tags: TagFacts {
                        title: Some(format!("Song {n}")),
                        ..TagFacts::default()
                    },
                };
                track_files::insert_track(&tx, lib, &format!("t{n}.flac"), &facts, 1)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    let mut seen = std::collections::BTreeSet::new();
    let mut sizes = Vec::new();
    for offset in [0, 500, 1000, 1500] {
        let v = call(
            &app,
            "search3",
            &ann(),
            &format!("query=&songCount=500&songOffset={offset}&albumCount=0&artistCount=0"),
        )
        .await;
        let page = ids(&v["searchResult3"]["song"]);
        sizes.push(page.len());
        for id in page {
            assert!(seen.insert(id), "duplicate across pages");
        }
    }
    assert_eq!(sizes, [500, 500, 200, 0]);
    assert_eq!(seen.len(), 1200);
}

#[tokio::test]
async fn quoted_empty_query_is_empty_query() {
    let app = app().await;
    for query in ["%22%22", "%20%20", "", "%20%22%22%20"] {
        let v = call(&app, "search3", &ann(), &format!("query={query}")).await;
        assert_eq!(v["status"], "ok", "{v}");
        assert_eq!(ids(&v["searchResult3"]["song"]).len(), 3, "{query}: {v}");
    }
    // A huge offset or count is clamped, not an error.
    let v = call(
        &app,
        "search3",
        &ann(),
        "query=&songCount=99999999999999999999&songOffset=99999999999999999999",
    )
    .await;
    assert_eq!(v["status"], "ok", "{v}");
    assert!(ids(&v["searchResult3"]["song"]).is_empty());
}

#[tokio::test]
async fn search_respects_library_access() {
    let app = app_empty().await;
    app.put(
        &app.music_a,
        "open.mp3",
        &song("Shared Melody", "Open Artist", "Open Album"),
    );
    app.put(
        &app.music_b,
        "secret.mp3",
        &song("Secret Melody", "Hidden Artist", "Hidden Album"),
    );
    app.scan();
    app.index().await;

    let v = call(&app, "search3", &ann(), "query=Melody").await;
    assert_eq!(
        titles(&v["searchResult3"]["song"]),
        ["Shared Melody"],
        "{v}"
    );

    let v = call(&app, "search3", &admin(), "query=Melody").await;
    assert_eq!(titles(&v["searchResult3"]["song"]).len(), 2, "{v}");

    let b = app.lib_b.0.to_string();
    for query in ["Melody", ""] {
        let v = call(
            &app,
            "search3",
            &ann(),
            &format!("query={query}&musicFolderId={b}"),
        )
        .await;
        assert_eq!(v["status"], "ok", "{v}");
        for kind in ["song", "album", "artist"] {
            assert!(ids(&v["searchResult3"][kind]).is_empty(), "{kind}: {v}");
        }
    }
    let v = call(
        &app,
        "search3",
        &admin(),
        &format!("query=Melody&musicFolderId={b}"),
    )
    .await;
    assert_eq!(
        titles(&v["searchResult3"]["song"]),
        ["Secret Melody"],
        "{v}"
    );
}

#[tokio::test]
async fn own_star_boosts_result() {
    let app = app_empty().await;
    app.put(&app.music_a, "x.mp3", &song("Echo", "Xavier", "Alpha"));
    app.put(&app.music_a, "y.mp3", &song("Echo", "Yolanda", "Beta"));
    app.scan();
    app.index().await;
    for artist in ["Xavier", "Yolanda"] {
        app.exec("DELETE FROM track_annotations").await;
        let sql: &'static str = if artist == "Xavier" {
            "INSERT INTO track_annotations (user_id, track_id, starred_at)
             SELECT u.id, t.id, 1 FROM users u, tracks t
             WHERE u.name = 'ann' AND t.artist_credit = 'Xavier'"
        } else {
            "INSERT INTO track_annotations (user_id, track_id, starred_at)
             SELECT u.id, t.id, 1 FROM users u, tracks t
             WHERE u.name = 'ann' AND t.artist_credit = 'Yolanda'"
        };
        app.exec(sql).await;
        let v = call(&app, "search3", &ann(), "query=Echo").await;
        let songs = v["searchResult3"]["song"].as_array().unwrap();
        assert_eq!(songs.len(), 2, "{v}");
        assert_eq!(songs[0]["artist"], artist, "{v}");
    }
}
