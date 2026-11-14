//! `content_hash` on files built from the silent templates: tags and covers do not
//! change it, different audio does. WAV and AIFF are covered too: the ID3v2 chunk the
//! fixture writes sits outside the data chunk, so their hash is stable as well.

use std::fs;
use std::path::Path;

use armonika_media::content_hash::content_hash;
use armonika_testdata::AudioFormat as TemplateFormat;
use armonika_testdata::fixture::{FixtureTags, cover_jpeg, tagged_file};

const FORMATS: [(TemplateFormat, &str); 7] = [
    (TemplateFormat::Mp3, "mp3"),
    (TemplateFormat::Flac, "flac"),
    (TemplateFormat::OggVorbis, "ogg"),
    (TemplateFormat::Opus, "opus"),
    (TemplateFormat::M4a, "m4a"),
    (TemplateFormat::Wav, "wav"),
    (TemplateFormat::Aiff, "aiff"),
];

fn titled(title: &str) -> FixtureTags {
    FixtureTags {
        title: Some(title.to_owned()),
        artist_credit: Some("Queen".to_owned()),
        ..FixtureTags::default()
    }
}

fn hash_of(path: &Path) -> [u8; 32] {
    content_hash(path).unwrap_or_else(|e| panic!("hashing {}: {e}", path.display()))
}

#[test]
fn hash_ignores_tag_changes() {
    let dir = tempfile::tempdir().unwrap();
    for (format, ext) in FORMATS {
        let a = tagged_file(dir.path(), &format!("a.{ext}"), format, &titled("One"));
        let b = tagged_file(
            dir.path(),
            &format!("b.{ext}"),
            format,
            &titled("A much longer title, changed"),
        );
        assert_eq!(hash_of(&a), hash_of(&b), "{ext}");
    }
}

#[test]
fn hash_ignores_cover_change() {
    let dir = tempfile::tempdir().unwrap();
    for (format, ext) in FORMATS {
        let plain = tagged_file(dir.path(), &format!("p.{ext}"), format, &titled("One"));
        let covered = tagged_file(
            dir.path(),
            &format!("c.{ext}"),
            format,
            &FixtureTags {
                cover: Some(cover_jpeg()),
                ..titled("One")
            },
        );
        assert_eq!(hash_of(&plain), hash_of(&covered), "{ext}");
    }
}

#[test]
fn hash_differs_for_different_audio() {
    let dir = tempfile::tempdir().unwrap();
    let mp3 = tagged_file(dir.path(), "a.mp3", TemplateFormat::Mp3, &titled("One"));
    let flac = tagged_file(dir.path(), "a.flac", TemplateFormat::Flac, &titled("One"));
    assert_ne!(hash_of(&mp3), hash_of(&flac));

    // The same format with one byte of audio changed.
    let mut bytes = fs::read(&mp3).unwrap();
    let at = bytes.len() - 200;
    bytes[at] ^= 0xFF;
    let altered = dir.path().join("altered.mp3");
    fs::write(&altered, bytes).unwrap();
    assert_ne!(hash_of(&mp3), hash_of(&altered));
}

#[test]
fn hash_fails_on_a_file_that_is_not_audio() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.mp3");
    fs::write(&path, b"this is not audio at all, just text").unwrap();
    assert!(content_hash(&path).is_err());
    assert!(content_hash(&dir.path().join("missing.mp3")).is_err());
}
