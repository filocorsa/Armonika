//! Small tagged audio files for tests: copy a silent template and write chosen tags.
//!
//! Tags are written only through lofty's native API for the concrete file type
//! (`split_tag`, edit the generic `Tag`, `merge_tag`, save), as the tag spike requires:
//! the generic `TaggedFile` API silently drops tags it has no key for. FLAC is saved as
//! a whole file so that its `PICTURE` blocks are kept. WAV and AIFF get an ID3v2 chunk,
//! lofty's default for those formats.
//!
//! The test crates use this as a dev-dependency only.

use std::borrow::Cow;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use lofty::TextEncoding;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::flac::FlacFile;
use lofty::id3::v2::{Frame, FrameId, Id3v2Tag, TextInformationFrame};
use lofty::iff::aiff::AiffFile;
use lofty::iff::wav::WavFile;
use lofty::mp4::{Ilst, Mp4File};
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OpusFile, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{ItemKey, ItemValue, MergeTag, SplitTag, Tag, TagExt, TagItem};

use crate::AudioFormat;

/// The tags to write. Same fields as `armonika_core::model::TagFacts`, plus `cover`.
///
/// To let tests check how text is interpreted, a few fields are raw text written as is:
/// `date` and `original_date` (for example `"1975-10"`), and `track_number_text`
/// (for example `"A1"` or `"3/12"`), which takes the place of `track_no` when both are
/// set. `None` and empty lists write nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixtureTags {
    pub title: Option<String>,
    pub title_sort: Option<String>,
    pub artist_credit: Option<String>,
    pub artist_sort: Option<String>,
    pub artists: Vec<String>,
    pub album_title: Option<String>,
    pub album_sort: Option<String>,
    pub album_artist_credit: Option<String>,
    pub album_artist_sort: Option<String>,
    pub album_artists: Vec<String>,
    pub track_no: Option<u32>,
    pub track_number_text: Option<String>,
    pub track_total: Option<u32>,
    pub disc_no: Option<u32>,
    pub disc_total: Option<u32>,
    pub disc_subtitle: Option<String>,
    pub date: Option<String>,
    pub original_date: Option<String>,
    pub genres: Vec<String>,
    pub compilation: bool,
    pub mb_recording_id: Option<String>,
    pub mb_release_track_id: Option<String>,
    pub mb_release_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    pub mb_work_id: Option<String>,
    pub mb_artist_ids: Vec<String>,
    pub mb_album_artist_ids: Vec<String>,
    /// Embedded front cover (JPEG bytes).
    pub cover: Option<Vec<u8>>,
}

/// A valid 8x8 JPEG (`bench/templates/cover.jpg`) for `FixtureTags::cover`. Some
/// containers (Vorbis comments) drop a picture that does not parse as an image.
pub fn cover_jpeg() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("templates")
        .join("cover.jpg");
    fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Copy the silent template of `format` to `dir/name` and write `tags` into it.
/// `name` is used as given, extension included, so tests can pick `SONG.MP3`.
///
/// Panics on any error: this is a test helper and a failure means a broken test setup.
pub fn tagged_file(dir: &Path, name: &str, format: AudioFormat, tags: &FixtureTags) -> PathBuf {
    let path = dir.join(name);
    fs::copy(format.template_path(), &path)
        .unwrap_or_else(|e| panic!("copying the {format:?} template to {}: {e}", path.display()));
    write_tags(&path, format, tags)
        .unwrap_or_else(|e| panic!("writing tags to {}: {e}", path.display()));
    path
}

fn write_tags(path: &Path, format: AudioFormat, tags: &FixtureTags) -> anyhow::Result<()> {
    let options = ParseOptions::new();
    let mut file = File::open(path)?;
    match format {
        AudioFormat::Mp3 => {
            let native = lofty::mpeg::MpegFile::read_from(&mut file, options)?
                .id3v2()
                .cloned()
                .unwrap_or_default();
            drop(file);
            save_id3v2(native, path, tags)
        }
        AudioFormat::Wav => {
            let native = WavFile::read_from(&mut file, options)?
                .id3v2()
                .cloned()
                .unwrap_or_default();
            drop(file);
            save_id3v2(native, path, tags)
        }
        AudioFormat::Aiff => {
            let native = AiffFile::read_from(&mut file, options)?
                .id3v2()
                .cloned()
                .unwrap_or_default();
            drop(file);
            save_id3v2(native, path, tags)
        }
        AudioFormat::Flac => {
            let mut flac = FlacFile::read_from(&mut file, options)?;
            drop(file);
            let native: VorbisComments = flac.vorbis_comments().cloned().unwrap_or_default();
            let (remainder, mut tag) = native.split_tag();
            fill(&mut tag, tags, TrackText::Verbatim);
            flac.set_vorbis_comments(remainder.merge_tag(tag));
            flac.save_to_path(path, WriteOptions::default())?;
            Ok(())
        }
        AudioFormat::OggVorbis => {
            let native = VorbisFile::read_from(&mut file, options)?
                .vorbis_comments()
                .clone();
            drop(file);
            let (remainder, mut tag) = native.split_tag();
            fill(&mut tag, tags, TrackText::Verbatim);
            remainder
                .merge_tag(tag)
                .save_to_path(path, WriteOptions::default())?;
            Ok(())
        }
        AudioFormat::Opus => {
            let native = OpusFile::read_from(&mut file, options)?
                .vorbis_comments()
                .clone();
            drop(file);
            let (remainder, mut tag) = native.split_tag();
            fill(&mut tag, tags, TrackText::Verbatim);
            remainder
                .merge_tag(tag)
                .save_to_path(path, WriteOptions::default())?;
            Ok(())
        }
        AudioFormat::M4a => {
            let native: Ilst = Mp4File::read_from(&mut file, options)?
                .ilst()
                .cloned()
                .unwrap_or_default();
            drop(file);
            let (remainder, mut tag) = native.split_tag();
            fill(&mut tag, tags, TrackText::Numbers);
            remainder
                .merge_tag(tag)
                .save_to_path(path, WriteOptions::default())?;
            Ok(())
        }
    }
}

/// ID3v2 keeps only integer track numbers when merging a generic `Tag`, so a raw
/// `track_number_text` is written afterwards as the `TRCK` frame itself.
fn save_id3v2(native: Id3v2Tag, path: &Path, tags: &FixtureTags) -> anyhow::Result<()> {
    let (remainder, mut tag) = native.split_tag();
    fill(&mut tag, tags, TrackText::Skip);
    let mut merged = remainder.merge_tag(tag);
    if let Some(text) = &tags.track_number_text {
        let value = match tags.track_total {
            Some(total) => format!("{text}/{total}"),
            None => text.clone(),
        };
        merged.insert(Frame::Text(TextInformationFrame::new(
            FrameId::Valid(Cow::Borrowed("TRCK")),
            TextEncoding::UTF8,
            value,
        )));
    }
    merged.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

/// How a container stores `track_number_text`.
#[derive(Clone, Copy)]
enum TrackText {
    /// As the text of the track number item (Vorbis comments).
    Verbatim,
    /// Only as numbers: `"3"` or `"3/12"`; other text cannot be stored (MP4 `trkn`).
    Numbers,
    /// Not through the generic tag; the caller writes it natively (ID3v2).
    Skip,
}

fn push(tag: &mut Tag, key: ItemKey, value: &str) {
    tag.push(TagItem::new(key, ItemValue::Text(value.to_string())));
}

fn push_opt(tag: &mut Tag, key: ItemKey, value: &Option<String>) {
    if let Some(value) = value {
        push(tag, key, value);
    }
}

fn push_all(tag: &mut Tag, key: ItemKey, values: &[String]) {
    for value in values {
        push(tag, key, value);
    }
}

fn fill(tag: &mut Tag, t: &FixtureTags, track_text: TrackText) {
    push_opt(tag, ItemKey::TrackTitle, &t.title);
    push_opt(tag, ItemKey::TrackTitleSortOrder, &t.title_sort);
    push_opt(tag, ItemKey::TrackArtist, &t.artist_credit);
    push_opt(tag, ItemKey::TrackArtistSortOrder, &t.artist_sort);
    push_all(tag, ItemKey::TrackArtists, &t.artists);
    push_opt(tag, ItemKey::AlbumTitle, &t.album_title);
    push_opt(tag, ItemKey::AlbumTitleSortOrder, &t.album_sort);
    push_opt(tag, ItemKey::AlbumArtist, &t.album_artist_credit);
    push_opt(tag, ItemKey::AlbumArtistSortOrder, &t.album_artist_sort);
    push_all(tag, ItemKey::AlbumArtists, &t.album_artists);

    let mut track_total = t.track_total;
    match (&t.track_number_text, track_text) {
        (Some(text), TrackText::Verbatim) => push(tag, ItemKey::TrackNumber, text),
        (Some(text), TrackText::Numbers) => {
            let mut parts = text.splitn(2, '/');
            if let Some(n) = parts.next().and_then(|n| n.trim().parse::<u32>().ok()) {
                push(tag, ItemKey::TrackNumber, &n.to_string());
            }
            if track_total.is_none() {
                track_total = parts.next().and_then(|n| n.trim().parse::<u32>().ok());
            }
        }
        (Some(_), TrackText::Skip) => track_total = None,
        (None, _) => {
            if let Some(n) = t.track_no {
                push(tag, ItemKey::TrackNumber, &n.to_string());
            }
        }
    }
    if let Some(total) = track_total {
        push(tag, ItemKey::TrackTotal, &total.to_string());
    }
    if let Some(n) = t.disc_no {
        push(tag, ItemKey::DiscNumber, &n.to_string());
    }
    if let Some(n) = t.disc_total {
        push(tag, ItemKey::DiscTotal, &n.to_string());
    }
    push_opt(tag, ItemKey::SetSubtitle, &t.disc_subtitle);
    push_opt(tag, ItemKey::RecordingDate, &t.date);
    push_opt(tag, ItemKey::OriginalReleaseDate, &t.original_date);
    push_all(tag, ItemKey::Genre, &t.genres);
    if t.compilation {
        push(tag, ItemKey::FlagCompilation, "1");
    }
    push_opt(tag, ItemKey::MusicBrainzRecordingId, &t.mb_recording_id);
    push_opt(tag, ItemKey::MusicBrainzTrackId, &t.mb_release_track_id);
    push_opt(tag, ItemKey::MusicBrainzReleaseId, &t.mb_release_id);
    push_opt(
        tag,
        ItemKey::MusicBrainzReleaseGroupId,
        &t.mb_release_group_id,
    );
    push_opt(tag, ItemKey::MusicBrainzWorkId, &t.mb_work_id);
    push_all(tag, ItemKey::MusicBrainzArtistId, &t.mb_artist_ids);
    push_all(
        tag,
        ItemKey::MusicBrainzReleaseArtistId,
        &t.mb_album_artist_ids,
    );
    if let Some(cover) = &t.cover {
        tag.push_picture(
            Picture::unchecked(cover.clone())
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Jpeg)
                .build(),
        );
    }
}
