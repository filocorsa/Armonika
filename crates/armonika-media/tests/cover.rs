//! Cover art: embedded pictures, folder images and resizing.

use std::fs;
use std::io::Cursor;

use armonika_media::cover;
use armonika_testdata::AudioFormat;
use armonika_testdata::fixture::{FixtureTags, cover_jpeg, tagged_file};
use image::{ImageFormat, RgbImage};
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::TagExt;

fn tags() -> FixtureTags {
    FixtureTags {
        title: Some("T".to_owned()),
        ..FixtureTags::default()
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    RgbImage::new(width, height)
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

fn dimensions(bytes: &[u8]) -> (u32, u32) {
    let img = image::load_from_memory(bytes).unwrap();
    (img.width(), img.height())
}

#[test]
fn embedded_front_cover_preferred() {
    let dir = tempfile::tempdir().unwrap();
    let path = tagged_file(dir.path(), "a.mp3", AudioFormat::Mp3, &tags());
    // A PNG of another type first, then the JPEG front cover.
    let mut tagged = Probe::open(&path).unwrap().read().unwrap();
    let tag = tagged.primary_tag_mut().unwrap();
    tag.push_picture(
        Picture::unchecked(png(4, 4))
            .pic_type(PictureType::Other)
            .mime_type(MimeType::Png)
            .build(),
    );
    tag.push_picture(
        Picture::unchecked(cover_jpeg())
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Jpeg)
            .build(),
    );
    tag.save_to_path(&path, WriteOptions::default()).unwrap();

    let image = cover::embedded(&path).unwrap().unwrap();
    assert_eq!(image.mime, "image/jpeg");
    assert_eq!(image.bytes, cover_jpeg());
}

#[test]
fn embedded_without_front_takes_the_first_picture() {
    let dir = tempfile::tempdir().unwrap();
    let path = tagged_file(dir.path(), "a.mp3", AudioFormat::Mp3, &tags());
    assert!(cover::embedded(&path).unwrap().is_none());
    let mut tagged = Probe::open(&path).unwrap().read().unwrap();
    let tag = tagged.primary_tag_mut().unwrap();
    tag.push_picture(
        Picture::unchecked(png(4, 4))
            .pic_type(PictureType::Artist)
            .mime_type(MimeType::Png)
            .build(),
    );
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    assert_eq!(cover::embedded(&path).unwrap().unwrap().mime, "image/png");
}

#[test]
fn folder_cover_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(cover::folder(dir.path()), None);
    fs::write(dir.path().join("notes.jpg"), b"x").unwrap();
    assert_eq!(cover::folder(dir.path()), None);
    fs::write(dir.path().join("Front.PNG"), b"x").unwrap();
    assert_eq!(
        cover::folder(dir.path()),
        Some(dir.path().join("Front.PNG"))
    );
    fs::write(dir.path().join("COVER.Jpg"), b"x").unwrap();
    assert_eq!(
        cover::folder(dir.path()),
        Some(dir.path().join("COVER.Jpg"))
    );
}

#[test]
fn resize_never_upscales() {
    let small = png(8, 8);
    let out = cover::resize(&small, 100).unwrap();
    assert_eq!(dimensions(&out), (8, 8));
    let big = png(200, 100);
    let out = cover::resize(&big, 50).unwrap();
    assert_eq!(dimensions(&out), (50, 25));
    assert_eq!(image::guess_format(&out).unwrap(), ImageFormat::Jpeg);
}
