mod common;

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use armonika_subsonic::MediaConfig;
use armonika_testdata::fixture::FixtureTags;
use common::*;

fn ann() -> String {
    login("ann", ANN_PASSWORD)
}

fn tags() -> FixtureTags {
    FixtureTags {
        title: Some("One".to_owned()),
        album_title: Some("Album".to_owned()),
        artist_credit: Some("Artist".to_owned()),
        artists: vec!["Artist".to_owned()],
        ..FixtureTags::default()
    }
}

fn media(ffmpeg: Option<PathBuf>) -> MediaConfig {
    MediaConfig {
        ffmpeg,
        transcode_format: "mp3".to_owned(),
        transcode_bitrate_kbps: 128,
        ..MediaConfig::default()
    }
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

async fn one_track(app: &App) -> String {
    app.put(&app.music_a, "one.mp3", &tags());
    app.scan();
    let reply = get(
        app,
        &format!(
            "/rest/getRandomSongs?{}&size=1",
            login("admin", ADMIN_PASSWORD)
        ),
    )
    .await;
    reply.json()["randomSongs"]["song"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn stream_with_max_bitrate_is_transcoded() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg is not on the PATH");
        return;
    }
    let app = app_with_media(media(None)).await;
    let id = one_track(&app).await;
    let reply = get(
        &app,
        &format!("/rest/stream?{}&id={id}&format=opus&maxBitRate=64", ann()),
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.headers["content-type"], "audio/ogg");
    assert!(reply.headers.get("content-length").is_none());
    assert!(reply.bytes.starts_with(b"OggS"));
    let original = fs::read(app.music_a.join("one.mp3")).unwrap();
    assert_ne!(reply.bytes, original);
}

#[tokio::test]
async fn missing_ffmpeg_falls_back_to_raw() {
    let app = app_with_media(media(Some(PathBuf::from("/nonexistent/ffmpeg")))).await;
    let id = one_track(&app).await;
    let reply = get(
        &app,
        &format!("/rest/stream?{}&id={id}&format=opus&maxBitRate=64", ann()),
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.headers["content-type"], "audio/mpeg");
    assert_eq!(reply.bytes, fs::read(app.music_a.join("one.mp3")).unwrap());
}
