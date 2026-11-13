//! `read_file` on real files of all seven formats, built from the silent templates.

use std::fs;
use std::path::{Path, PathBuf};

use armonika_core::model::{AudioFormat, PartialDate, TagFacts};
use armonika_media::probe::{TAG_SCHEMA_VERSION, read_file};
use armonika_testdata::AudioFormat as TemplateFormat;
use armonika_testdata::fixture::{FixtureTags, cover_jpeg, tagged_file};

/// Every format: the template to copy, the format `read_file` must report, the extension.
const FORMATS: [(TemplateFormat, AudioFormat, &str); 7] = [
    (TemplateFormat::Mp3, AudioFormat::Mp3, "mp3"),
    (TemplateFormat::Flac, AudioFormat::Flac, "flac"),
    (TemplateFormat::OggVorbis, AudioFormat::Ogg, "ogg"),
    (TemplateFormat::Opus, AudioFormat::Opus, "opus"),
    (TemplateFormat::M4a, AudioFormat::M4a, "m4a"),
    (TemplateFormat::Wav, AudioFormat::Wav, "wav"),
    (TemplateFormat::Aiff, AudioFormat::Aiff, "aiff"),
];

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

fn v(values: &[&str]) -> Vec<String> {
    values.iter().map(|x| x.to_string()).collect()
}

fn year(y: i32) -> Option<PartialDate> {
    Some(PartialDate {
        year: y,
        month: None,
        day: None,
    })
}

/// Write `tags` into a file of every format and read it back.
fn each_format(tags: &FixtureTags, mut check: impl FnMut(&str, AudioFormat, &TagFacts)) {
    let dir = tempfile::tempdir().unwrap();
    for (template, format, ext) in FORMATS {
        let path = tagged_file(dir.path(), &format!("song.{ext}"), template, tags);
        let facts = read_file(&path).unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!(facts.format, format, "{ext}");
        check(ext, format, &facts.tags);
    }
}

#[test]
fn tag_schema_version_is_one() {
    assert_eq!(TAG_SCHEMA_VERSION, 1);
}

#[test]
fn reads_basic_tags() {
    let tags = FixtureTags {
        title: s("  So What  "),
        artist_credit: s("Miles Davis"),
        album_title: s("Kind of Blue"),
        album_artist_credit: s("Miles Davis"),
        track_no: Some(1),
        track_total: Some(5),
        disc_no: Some(1),
        disc_total: Some(2),
        date: s("1959"),
        genres: v(&["Jazz", "Modal"]),
        compilation: true,
        ..Default::default()
    };
    let dir = tempfile::tempdir().unwrap();
    for (template, format, ext) in FORMATS {
        let path = tagged_file(dir.path(), &format!("song.{ext}"), template, &tags);
        let facts = read_file(&path).unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!(facts.format, format, "{ext}");
        assert_eq!(facts.size, fs::metadata(&path).unwrap().len(), "{ext}");
        assert!(facts.mtime_ms > 0, "{ext}");
        let duration = facts
            .duration_ms
            .unwrap_or_else(|| panic!("{ext}: no duration"));
        assert!(
            (150..=300).contains(&duration),
            "{ext}: duration {duration}"
        );
        assert_eq!(facts.channels, Some(2), "{ext}");
        let expected_rate = if format == AudioFormat::Opus {
            48000
        } else {
            44100
        };
        assert_eq!(facts.sample_rate, Some(expected_rate), "{ext}");
        let t = &facts.tags;
        assert_eq!(t.title, s("So What"), "{ext}: trimmed title");
        assert_eq!(t.artist_credit, s("Miles Davis"), "{ext}");
        assert_eq!(t.album_title, s("Kind of Blue"), "{ext}");
        assert_eq!(t.album_artist_credit, s("Miles Davis"), "{ext}");
        assert_eq!(t.track_no, Some(1), "{ext}");
        assert_eq!(t.track_number_text, s("1"), "{ext}");
        assert_eq!(t.track_total, Some(5), "{ext}");
        assert_eq!(t.disc_no, Some(1), "{ext}");
        assert_eq!(t.disc_total, Some(2), "{ext}");
        assert_eq!(t.date, year(1959), "{ext}");
        assert_eq!(t.genres, v(&["Jazz", "Modal"]), "{ext}");
        assert!(t.compilation, "{ext}");
        assert!(!t.has_embedded_cover, "{ext}");
    }
}

#[test]
fn genres_split_on_semicolon_not_comma() {
    let tags = FixtureTags {
        genres: v(&["Rock; Pop, Funk;;"]),
        ..Default::default()
    };
    each_format(&tags, |ext, _, t| {
        assert_eq!(t.genres, v(&["Rock", "Pop, Funk"]), "{ext}");
    });
}

#[test]
fn reads_multi_valued_artists_and_mb_ids() {
    let tags = FixtureTags {
        artist_credit: s("Simon & Garfunkel"),
        artists: v(&["Paul Simon", "Art Garfunkel"]),
        album_artist_credit: s("Simon & Garfunkel"),
        album_artists: v(&["Paul Simon", "Art Garfunkel"]),
        mb_recording_id: s("11111111-1111-1111-1111-111111111111"),
        mb_release_track_id: s("22222222-2222-2222-2222-222222222222"),
        mb_release_id: s("33333333-3333-3333-3333-333333333333"),
        mb_release_group_id: s("44444444-4444-4444-4444-444444444444"),
        mb_work_id: s("55555555-5555-5555-5555-555555555555"),
        mb_artist_ids: v(&[
            "66666666-6666-6666-6666-666666666666",
            "77777777-7777-7777-7777-777777777777",
        ]),
        mb_album_artist_ids: v(&[
            "88888888-8888-8888-8888-888888888888",
            "99999999-9999-9999-9999-999999999999",
        ]),
        ..Default::default()
    };
    // The brief requires mp3, flac, ogg, opus and m4a. WAV and AIFF carry the same
    // ID3v2 tag in a chunk, and lofty reads it back in full there too, so they are
    // checked with the same expectations.
    each_format(&tags, |ext, _, t| {
        assert_eq!(t.artist_credit, s("Simon & Garfunkel"), "{ext}");
        assert_eq!(t.artists, v(&["Paul Simon", "Art Garfunkel"]), "{ext}");
        assert_eq!(
            t.album_artists,
            v(&["Paul Simon", "Art Garfunkel"]),
            "{ext}"
        );
        assert_eq!(t.mb_recording_id, tags.mb_recording_id, "{ext}");
        assert_eq!(t.mb_release_track_id, tags.mb_release_track_id, "{ext}");
        assert_eq!(t.mb_release_id, tags.mb_release_id, "{ext}");
        assert_eq!(t.mb_release_group_id, tags.mb_release_group_id, "{ext}");
        assert_eq!(t.mb_work_id, tags.mb_work_id, "{ext}");
        assert_eq!(t.mb_artist_ids, tags.mb_artist_ids, "{ext}");
        assert_eq!(t.mb_album_artist_ids, tags.mb_album_artist_ids, "{ext}");
    });
}

#[test]
fn partial_dates_parsed() {
    let cases = [
        ("1975", year(1975)),
        (
            "1975-10",
            Some(PartialDate {
                year: 1975,
                month: Some(10),
                day: None,
            }),
        ),
        (
            "1975-10-31",
            Some(PartialDate {
                year: 1975,
                month: Some(10),
                day: Some(31),
            }),
        ),
        (
            "1975-03-01T00:00",
            Some(PartialDate {
                year: 1975,
                month: Some(3),
                day: Some(1),
            }),
        ),
        ("1975?", year(1975)),
    ];
    for (text, expected) in cases {
        let tags = FixtureTags {
            date: s(text),
            ..Default::default()
        };
        each_format(&tags, |ext, _, t| {
            assert_eq!(t.date, expected, "{ext}: {text:?}");
        });
    }
}

#[test]
fn original_date_separate_from_release_date() {
    let tags = FixtureTags {
        date: s("2011"),
        original_date: s("1975"),
        ..Default::default()
    };
    each_format(&tags, |ext, _, t| {
        assert_eq!(t.date, year(2011), "{ext}");
        assert_eq!(t.original_date, year(1975), "{ext}");
    });
}

#[test]
fn vinyl_track_number_kept_as_text() {
    let vinyl = FixtureTags {
        track_number_text: s("A1"),
        ..Default::default()
    };
    each_format(&vinyl, |ext, format, t| {
        assert_eq!(t.track_no, None, "{ext}");
        if format == AudioFormat::M4a {
            // Limit of the container: MP4 stores the track number only as a binary
            // integer pair (`trkn`), so a side like "A1" cannot be written at all.
            assert_eq!(t.track_number_text, None, "{ext}");
        } else {
            assert_eq!(t.track_number_text, s("A1"), "{ext}");
        }
    });

    let side_with_total = FixtureTags {
        track_number_text: s("A1"),
        track_total: Some(12),
        ..Default::default()
    };
    each_format(&side_with_total, |ext, format, t| {
        assert_eq!(t.track_no, None, "{ext}");
        assert_eq!(t.track_total, Some(12), "{ext}");
        if format == AudioFormat::M4a {
            assert_eq!(t.track_number_text, None, "{ext}: MP4 limit, see above");
        } else {
            assert_eq!(t.track_number_text, s("A1"), "{ext}");
        }
    });

    let with_total = FixtureTags {
        track_number_text: s("3/12"),
        ..Default::default()
    };
    each_format(&with_total, |ext, _, t| {
        assert_eq!(t.track_number_text, s("3"), "{ext}");
        assert_eq!(t.track_no, Some(3), "{ext}");
        assert_eq!(t.track_total, Some(12), "{ext}");
    });
}

#[test]
fn reads_sort_tags() {
    let tags = FixtureTags {
        title: s("The Wall"),
        title_sort: s("Wall, The"),
        artist_credit: s("The Beatles"),
        artist_sort: s("Beatles, The"),
        album_title: s("The White Album"),
        album_sort: s("White Album, The"),
        album_artist_credit: s("The Beatles"),
        album_artist_sort: s("Beatles, The"),
        ..Default::default()
    };
    each_format(&tags, |ext, _, t| {
        assert_eq!(t.title_sort, s("Wall, The"), "{ext}");
        assert_eq!(t.artist_sort, s("Beatles, The"), "{ext}");
        assert_eq!(t.album_sort, s("White Album, The"), "{ext}");
        assert_eq!(t.album_artist_sort, s("Beatles, The"), "{ext}");
    });
}

#[test]
fn reads_disc_subtitle() {
    let tags = FixtureTags {
        disc_no: Some(2),
        disc_subtitle: s("Live at the BBC"),
        ..Default::default()
    };
    each_format(&tags, |ext, _, t| {
        assert_eq!(t.disc_no, Some(2), "{ext}");
        assert_eq!(t.disc_subtitle, s("Live at the BBC"), "{ext}");
    });
}

#[test]
fn detects_embedded_cover() {
    let tags = FixtureTags {
        title: s("With cover"),
        cover: Some(cover_jpeg()),
        ..Default::default()
    };
    each_format(&tags, |ext, _, t| {
        assert!(t.has_embedded_cover, "{ext}");
        assert_eq!(t.title, s("With cover"), "{ext}");
    });
}

#[test]
fn missing_tags_are_none() {
    let tags = FixtureTags {
        title: s("   "),
        ..Default::default()
    };
    each_format(&tags, |ext, _, t| {
        assert_eq!(t, &TagFacts::default(), "{ext}");
    });
}

#[test]
fn truncated_file_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let tags = FixtureTags {
        title: s("Cut"),
        cover: Some(cover_jpeg()),
        ..Default::default()
    };
    for (template, format, ext) in FORMATS {
        let path = tagged_file(dir.path(), &format!("cut.{ext}"), template, &tags);
        let full = read_file(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        let cut = read_file(&path);
        if format == AudioFormat::Mp3 {
            // Limit of the format: an MP3 is a plain sequence of audio frames and the
            // template has no Xing/LAME header with the frame count, so a file cut
            // after its ID3v2 tag is still a valid, shorter MP3 for lofty. It reads
            // without error, with a shorter duration; the next scan reads it again
            // if the size changes.
            let cut = cut.unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert!(cut.duration_ms < full.duration_ms, "{ext}");
            // Cut inside the ID3v2 tag it is an error.
            fs::write(&path, &bytes[..40]).unwrap();
            assert!(read_file(&path).is_err(), "{ext}: cut inside the tag");
        } else {
            assert!(cut.is_err(), "{ext}: a half file must be an error");
        }
    }
}

/// `ORIGINALYEAR` is the fallback for the original date. lofty maps it in Vorbis
/// comments only; in ID3v2 (`TXXX`) and MP4 (freeform atom) it is read natively.
#[test]
fn original_year_used_when_original_date_missing() {
    use lofty::config::{ParseOptions, WriteOptions};
    use lofty::file::AudioFile;
    use lofty::flac::FlacFile;
    use lofty::mp4::{Atom, AtomData, AtomIdent, Mp4File};
    use lofty::mpeg::MpegFile;
    use lofty::ogg::tag::VorbisComments;
    use lofty::tag::TagExt;

    let dir = tempfile::tempdir().unwrap();
    let base = FixtureTags {
        date: s("2011"),
        ..Default::default()
    };

    let mp3 = tagged_file(dir.path(), "song.mp3", TemplateFormat::Mp3, &base);
    let mut file = fs::File::open(&mp3).unwrap();
    let mut id3 = MpegFile::read_from(&mut file, ParseOptions::new())
        .unwrap()
        .id3v2()
        .cloned()
        .unwrap();
    drop(file);
    // Picard writes the description in lower case.
    id3.insert_user_text("originalyear".to_string(), "1975".to_string());
    id3.save_to_path(&mp3, WriteOptions::default()).unwrap();

    let m4a = tagged_file(dir.path(), "song.m4a", TemplateFormat::M4a, &base);
    let mut file = fs::File::open(&m4a).unwrap();
    let mut ilst = Mp4File::read_from(&mut file, ParseOptions::new())
        .unwrap()
        .ilst()
        .cloned()
        .unwrap();
    drop(file);
    ilst.insert(Atom::new(
        AtomIdent::Freeform {
            mean: "com.apple.iTunes".into(),
            name: "ORIGINALYEAR".into(),
        },
        AtomData::UTF8("1975".to_string()),
    ));
    ilst.save_to_path(&m4a, WriteOptions::default()).unwrap();

    let flac = tagged_file(dir.path(), "song.flac", TemplateFormat::Flac, &base);
    let mut file = fs::File::open(&flac).unwrap();
    let mut flac_file = FlacFile::read_from(&mut file, ParseOptions::new()).unwrap();
    drop(file);
    let mut comments: VorbisComments = flac_file.vorbis_comments().cloned().unwrap();
    comments.push("ORIGINALYEAR".to_string(), "1975".to_string());
    flac_file.set_vorbis_comments(comments);
    flac_file
        .save_to_path(&flac, WriteOptions::default())
        .unwrap();

    for path in [mp3, m4a, flac] {
        let t = read_file(&path).unwrap().tags;
        assert_eq!(t.date, year(2011), "{}", path.display());
        assert_eq!(t.original_date, year(1975), "{}", path.display());
    }
}

#[test]
fn empty_and_missing_files_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.mp3");
    fs::write(&empty, b"").unwrap();
    assert!(read_file(&empty).is_err());
    assert!(read_file(&dir.path().join("missing.flac")).is_err());
}

#[test]
fn unsupported_extension_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = tagged_file(
        dir.path(),
        "song.txt",
        TemplateFormat::Mp3,
        &FixtureTags::default(),
    );
    assert!(read_file(&path).is_err());
}

#[test]
fn extension_case_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let tags = FixtureTags {
        title: s("Loud"),
        ..Default::default()
    };
    for (template, format, ext) in FORMATS {
        let name = format!("SONG.{}", ext.to_ascii_uppercase());
        let path = tagged_file(dir.path(), &name, template, &tags);
        let facts = read_file(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(facts.format, format, "{name}");
        assert_eq!(facts.tags.title, s("Loud"), "{name}");
    }
    let aif = tagged_file(dir.path(), "SONG.AIF", TemplateFormat::Aiff, &tags);
    assert_eq!(read_file(&aif).unwrap().format, AudioFormat::Aiff);
}

/// A WAV file with only a RIFF INFO list (no ID3v2 chunk): the primary tag type of WAV
/// is ID3v2, so the title must come from the fallback tag.
#[test]
fn falls_back_to_other_tags() {
    use lofty::config::{ParseOptions, WriteOptions};
    use lofty::file::AudioFile;
    use lofty::iff::wav::{RiffInfoList, WavFile};
    use lofty::tag::{ItemKey, MergeTag, SplitTag, TagExt};

    let dir = tempfile::tempdir().unwrap();
    let path: PathBuf = dir.path().join("info.wav");
    fs::copy(TemplateFormat::Wav.template_path(), &path).unwrap();
    let mut file = fs::File::open(&path).unwrap();
    let wav = WavFile::read_from(&mut file, ParseOptions::new()).unwrap();
    drop(file);
    let info: RiffInfoList = wav.riff_info().cloned().unwrap_or_default();
    let (remainder, mut tag) = info.split_tag();
    tag.insert_text(ItemKey::TrackTitle, "From INFO".to_string());
    tag.insert_text(ItemKey::TrackArtist, "Someone".to_string());
    remainder
        .merge_tag(tag)
        .save_to_path(&path, WriteOptions::default())
        .unwrap();

    let facts = read_file(Path::new(&path)).unwrap();
    assert_eq!(facts.tags.title, s("From INFO"));
    assert_eq!(facts.tags.artist_credit, s("Someone"));
}
