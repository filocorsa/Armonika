mod common;

use std::fs;

use armonika_testdata::fixture::{FixtureTags, cover_jpeg};
use common::*;

fn ann() -> String {
    login("ann", ANN_PASSWORD)
}

fn admin() -> String {
    login("admin", ADMIN_PASSWORD)
}

fn tags(title: &str) -> FixtureTags {
    FixtureTags {
        title: Some(title.to_owned()),
        album_title: Some("Album".to_owned()),
        artist_credit: Some("Artist".to_owned()),
        artists: vec!["Artist".to_owned()],
        ..FixtureTags::default()
    }
}

/// The first track the caller can see: its track and album identifiers.
async fn first_song(app: &App, login: &str) -> (String, String) {
    let reply = get(app, &format!("/rest/getRandomSongs?{login}&size=1")).await;
    let song = reply.json()["randomSongs"]["song"][0].clone();
    (
        song["id"].as_str().unwrap().to_owned(),
        song["albumId"].as_str().unwrap_or_default().to_owned(),
    )
}

fn error_code(reply: &Reply) -> u64 {
    reply.json()["error"]["code"].as_u64().unwrap()
}

#[tokio::test]
async fn stream_full_file_matches_bytes() {
    let app = app_empty().await;
    app.put(&app.music_a, "x/one.mp3", &tags("One"));
    app.scan();
    let (id, _) = first_song(&app, &admin()).await;
    let reply = get(&app, &format!("/rest/stream?{}&id={id}", ann())).await;
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.bytes,
        fs::read(app.music_a.join("x").join("one.mp3")).unwrap()
    );
    assert_eq!(reply.headers["content-type"], "audio/mpeg");
    assert_eq!(reply.headers["accept-ranges"], "bytes");
}

#[tokio::test]
async fn stream_range_returns_206_and_slice() {
    let app = app_empty().await;
    app.put(&app.music_a, "one.mp3", &tags("One"));
    app.scan();
    let (id, _) = first_song(&app, &admin()).await;
    let reply = get_with(
        &app,
        &format!("/rest/stream?{}&id={id}", ann()),
        &[("range", "bytes=100-199")],
    )
    .await;
    assert_eq!(reply.status, 206);
    let all = fs::read(app.music_a.join("one.mp3")).unwrap();
    assert_eq!(reply.bytes, all[100..200]);
    assert!(
        reply.headers["content-range"]
            .to_str()
            .unwrap()
            .starts_with("bytes 100-199/")
    );
}

#[tokio::test]
async fn stream_other_library_is_70() {
    let app = app_empty().await;
    app.put(&app.music_b, "one.mp3", &tags("One"));
    app.scan();
    let (id, _) = first_song(&app, &admin()).await;
    for method in ["stream", "download", "getCoverArt"] {
        let reply = get(&app, &format!("/rest/{method}?{}&id={id}", ann())).await;
        assert_eq!(reply.status, 200);
        assert_eq!(reply.json()["status"], "failed", "{method}");
        assert_eq!(error_code(&reply), 70, "{method}");
    }
}

#[tokio::test]
async fn download_requires_download_role() {
    let app = app_empty().await;
    app.put(&app.music_a, "one.mp3", &tags("One"));
    app.scan();
    let (id, _) = first_song(&app, &admin()).await;
    let denied = get(&app, &format!("/rest/download?{}&id={id}", ann())).await;
    assert_eq!(error_code(&denied), 50);

    let reply = get(&app, &format!("/rest/download?{}&id={id}", admin())).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.bytes, fs::read(app.music_a.join("one.mp3")).unwrap());
    let disposition = reply.headers["content-disposition"].to_str().unwrap();
    assert!(disposition.starts_with("attachment"), "{disposition}");
    assert!(disposition.contains("one.mp3"), "{disposition}");
}

#[tokio::test]
async fn cover_for_album_uses_folder_image() {
    let app = app_empty().await;
    app.put(&app.music_a, "Album/one.mp3", &tags("One"));
    fs::write(app.music_a.join("Album").join("Cover.JPG"), cover_jpeg()).unwrap();
    app.scan();
    let (track, album) = first_song(&app, &admin()).await;
    for id in [album, track] {
        let reply = get(&app, &format!("/rest/getCoverArt?{}&id={id}", ann())).await;
        assert_eq!(reply.status, 200, "{id}");
        assert_eq!(reply.bytes, cover_jpeg(), "{id}");
        assert_eq!(reply.headers["content-type"], "image/jpeg");
    }
}

#[tokio::test]
async fn embedded_cover_beats_folder_for_a_track() {
    let app = app_empty().await;
    let mut with_cover = tags("One");
    with_cover.cover = Some(cover_jpeg());
    app.put(&app.music_a, "Album/one.mp3", &with_cover);
    app.scan();
    let (track, _) = first_song(&app, &admin()).await;
    let reply = get(&app, &format!("/rest/getCoverArt?{}&id={track}", ann())).await;
    assert_eq!(reply.bytes, cover_jpeg());
}

/// A small jpeg that differs from `cover_jpeg()`.
fn other_jpeg() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbImage::new(2, 2)
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    assert_ne!(out, cover_jpeg());
    out
}

#[tokio::test]
async fn album_with_embedded_and_folder_images_prefers_the_folder() {
    let app = app_empty().await;
    let mut with_cover = tags("One");
    with_cover.cover = Some(cover_jpeg());
    app.put(&app.music_a, "Album/one.mp3", &with_cover);
    fs::write(app.music_a.join("Album").join("cover.jpg"), other_jpeg()).unwrap();
    app.scan();
    let (track, album) = first_song(&app, &admin()).await;
    let reply = get(&app, &format!("/rest/getCoverArt?{}&id={album}", ann())).await;
    assert_eq!(reply.bytes, other_jpeg());
    let reply = get(&app, &format!("/rest/getCoverArt?{}&id={track}", ann())).await;
    assert_eq!(reply.bytes, cover_jpeg());
    // Again, from the cache: the same answers.
    let reply = get(&app, &format!("/rest/getCoverArt?{}&id={album}", ann())).await;
    assert_eq!(reply.bytes, other_jpeg());
    let reply = get(&app, &format!("/rest/getCoverArt?{}&id={track}", ann())).await;
    assert_eq!(reply.bytes, cover_jpeg());
}

#[tokio::test]
async fn a_folder_image_added_later_beats_a_cached_embedded_cover() {
    let app = app_empty().await;
    let mut with_cover = tags("One");
    with_cover.cover = Some(cover_jpeg());
    app.put(&app.music_a, "Album/one.mp3", &with_cover);
    app.scan();
    let (_, album) = first_song(&app, &admin()).await;
    let uri = format!("/rest/getCoverArt?{}&id={album}", ann());
    assert_eq!(get(&app, &uri).await.bytes, cover_jpeg());
    fs::write(app.music_a.join("Album").join("cover.jpg"), other_jpeg()).unwrap();
    assert_eq!(get(&app, &uri).await.bytes, other_jpeg());
}

#[tokio::test]
async fn an_empty_or_unrecognised_folder_image_is_no_picture() {
    let app = app_empty().await;
    let mut with_cover = tags("One");
    with_cover.cover = Some(cover_jpeg());
    app.put(&app.music_a, "Album/one.mp3", &with_cover);
    app.scan();
    let (_, album) = first_song(&app, &admin()).await;
    let uri = format!("/rest/getCoverArt?{}&id={album}", ann());
    let folder = app.music_a.join("Album").join("cover.jpg");
    fs::write(&folder, b"").unwrap();
    assert_eq!(get(&app, &uri).await.bytes, cover_jpeg());
    fs::write(&folder, b"this is not an image").unwrap();
    assert_eq!(get(&app, &uri).await.bytes, cover_jpeg());
}

#[tokio::test]
async fn no_cover_is_70() {
    let app = app_empty().await;
    app.put(&app.music_a, "Album/one.mp3", &tags("One"));
    app.scan();
    let (track, _) = first_song(&app, &admin()).await;
    let reply = get(&app, &format!("/rest/getCoverArt?{}&id={track}", ann())).await;
    assert_eq!(error_code(&reply), 70);
}

#[tokio::test]
async fn cover_resized_and_cached() {
    let app = app_empty().await;
    app.put(&app.music_a, "Album/one.mp3", &tags("One"));
    fs::write(app.music_a.join("Album").join("cover.jpg"), cover_jpeg()).unwrap();
    app.scan();
    let (_, album) = first_song(&app, &admin()).await;
    let uri = format!("/rest/getCoverArt?{}&id={album}&size=4", ann());
    let first = get(&app, &uri).await;
    assert_eq!(first.status, 200);
    let img = image::load_from_memory(&first.bytes).unwrap();
    assert_eq!((img.width(), img.height()), (4, 4));

    let cached: Vec<_> = fs::read_dir(&app.cache_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(cached.len(), 1);
    let name = cached[0].file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with(&format!("{album}-folder-4-")), "{name}");
    assert!(name.ends_with(".jpg"), "{name}");
    assert_eq!(fs::read(&cached[0]).unwrap(), first.bytes);

    // A second request is served from the cache: a changed cache file shows.
    let marker = {
        let mut out = Vec::new();
        image::RgbImage::new(2, 2)
            .write_to(
                &mut std::io::Cursor::new(&mut out),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        out
    };
    fs::write(&cached[0], &marker).unwrap();
    let second = get(&app, &uri).await;
    assert_eq!(second.bytes, marker);
}

#[tokio::test]
async fn stream_of_vanished_file_requests_rescan() {
    let app = app_empty().await;
    app.put(&app.music_a, "one.mp3", &tags("One"));
    app.scan();
    let (id, _) = first_song(&app, &admin()).await;
    assert!(app.scans.status().libraries.is_empty());
    fs::remove_file(app.music_a.join("one.mp3")).unwrap();
    let reply = get(&app, &format!("/rest/stream?{}&id={id}", ann())).await;
    assert_eq!(error_code(&reply), 70);
    let status = app.scans.status();
    assert!(status.libraries.iter().any(|(lib, _)| *lib == app.lib_a));
}
