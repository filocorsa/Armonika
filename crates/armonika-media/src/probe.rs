//! Reading one audio file: size, modification time, audio properties and tags.
//!
//! The file is only read (lofty's `Probe`, never a save). Tags come from the primary
//! tag of the format (ID3v2 for mp3, wav and aiff; Vorbis comments for flac, ogg and
//! opus; `ilst` for m4a); a field missing there is taken from the other tags in the
//! file, if any (for example ID3v1, APE or RIFF INFO).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use armonika_core::model::{AudioFormat, FileFacts, PartialDate, TagFacts};
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::id3::v2::{Frame, Id3v2Tag};
use lofty::mp4::{AtomData, AtomIdent, Ilst};
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagType};

/// Version of the set of tags read and of how they are interpreted. Increment it on
/// every change to either: the scanner reads again every track read with an older
/// version, so a tag added later does not stay empty on files whose size and date did
/// not change.
pub const TAG_SCHEMA_VERSION: u32 = 1;

/// Why a file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} has an extension that is not an accepted audio format", .0.display())]
    UnsupportedExtension(PathBuf),
    #[error("{} is not in an accepted audio format", .0.display())]
    UnsupportedFormat(PathBuf),
    #[error("cannot parse {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: lofty::error::FileParseError,
    },
}

/// Read the facts of the audio file at `path`. Only reads the file.
///
/// The extension must be one of the accepted ones (case is ignored); the reported
/// format is the one found in the content.
pub fn read_file(path: &Path) -> Result<FileFacts, ProbeError> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(AudioFormat::from_extension)
        .ok_or_else(|| ProbeError::UnsupportedExtension(path.to_path_buf()))?;

    let io_err = |source| ProbeError::Io {
        path: path.to_path_buf(),
        source,
    };
    let parse_err = |source| ProbeError::Parse {
        path: path.to_path_buf(),
        source,
    };
    let meta = fs::metadata(path).map_err(io_err)?;
    let mtime_ms = meta
        .modified()
        .map_err(io_err)
        .map(|t| match t.duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
            Err(before) => -i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX),
        })?;

    let tagged = Probe::open(path)
        .map_err(parse_err)?
        .guess_file_type()
        .map_err(io_err)?
        .read()
        .map_err(parse_err)?;
    let format = format_of(tagged.file_type())
        .ok_or_else(|| ProbeError::UnsupportedFormat(path.to_path_buf()))?;

    let props = tagged.properties();
    let duration_ms = u64::try_from(props.duration().as_millis())
        .ok()
        .filter(|ms| *ms > 0);
    Ok(FileFacts {
        size: meta.len(),
        mtime_ms,
        format,
        duration_ms,
        bitrate_kbps: props
            .audio_bitrate()
            .or(props.overall_bitrate())
            .filter(|b| *b > 0),
        sample_rate: props.sample_rate().filter(|r| *r > 0),
        channels: props.channels().filter(|c| *c > 0),
        bit_depth: props.bit_depth().filter(|b| *b > 0),
        tags: read_tags(&tagged),
    })
}

fn format_of(file_type: FileType) -> Option<AudioFormat> {
    Some(match file_type {
        FileType::Mpeg => AudioFormat::Mp3,
        FileType::Flac => AudioFormat::Flac,
        FileType::Vorbis => AudioFormat::Ogg,
        FileType::Opus => AudioFormat::Opus,
        FileType::Mp4 => AudioFormat::M4a,
        FileType::Wav => AudioFormat::Wav,
        FileType::Aiff => AudioFormat::Aiff,
        _ => return None,
    })
}

/// The tags of a file, primary first.
struct Tags<'a>(Vec<&'a Tag>);

impl<'a> Tags<'a> {
    fn of(tagged: &'a TaggedFile) -> Self {
        let primary = tagged.primary_tag_type();
        let mut tags: Vec<&Tag> = tagged.primary_tag().into_iter().collect();
        tags.extend(tagged.tags().iter().filter(|t| t.tag_type() != primary));
        Tags(tags)
    }

    /// The trimmed, non-empty values of `key` in the first tag that has any.
    fn texts(&self, key: ItemKey) -> Vec<String> {
        self.0
            .iter()
            .map(|tag| clean(tag.get_strings(key)))
            .find(|values| !values.is_empty())
            .unwrap_or_default()
    }

    fn text(&self, key: ItemKey) -> Option<String> {
        self.texts(key).into_iter().next()
    }

    /// Values of a list field, also split on NUL (the ID3v2.4 separator, in case a
    /// container stores it inside one value).
    fn list(&self, key: ItemKey) -> Vec<String> {
        split_values(self.texts(key), &['\0'])
    }

    /// A track or disc number with its total, from the first tag that has either.
    ///
    /// lofty quirk (0.25): an ID3v2 `TRCK` or `TPOS` that does not start with an
    /// integer (`"A1"`, `"A1/12"`) is not split into number and total; the whole text
    /// ends up under the total key and the number key is missing. In that case the
    /// total holds the frame as written and is read as the number.
    fn number(&self, number_key: ItemKey, total_key: ItemKey) -> NumberPair {
        for tag in &self.0 {
            let number = clean(tag.get_strings(number_key)).into_iter().next();
            let total = clean(tag.get_strings(total_key)).into_iter().next();
            match (number, total) {
                (None, None) => continue,
                (None, Some(raw))
                    if tag.tag_type() == TagType::Id3v2 && raw.parse::<u32>().is_err() =>
                {
                    return number_pair(Some(raw), None);
                }
                (number, total) => return number_pair(number, total),
            }
        }
        number_pair(None, None)
    }

    /// First value that parses as a date, among `keys` in order.
    fn date(&self, keys: &[ItemKey]) -> Option<PartialDate> {
        keys.iter()
            .flat_map(|key| self.texts(*key))
            .find_map(|value| parse_partial_date(&value))
    }
}

fn clean<'s>(values: impl Iterator<Item = &'s str>) -> Vec<String> {
    values
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

fn split_values(values: Vec<String>, separators: &[char]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| v.split(separators))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

fn read_tags(tagged: &TaggedFile) -> TagFacts {
    let tags = Tags::of(tagged);

    // Several values of the credit field (an ID3v2.4 list, a repeated Vorbis comment)
    // are kept together in the credit and, when there is no `ARTISTS` tag, also used as
    // the list of artists.
    let credit = |single: ItemKey, list: ItemKey| {
        let credits = tags.list(single);
        let mut artists = tags.list(list);
        if artists.is_empty() && credits.len() > 1 {
            artists = credits.clone();
        }
        let credit = (!credits.is_empty()).then(|| credits.join("; "));
        (credit, artists)
    };
    let (artist_credit, artists) = credit(ItemKey::TrackArtist, ItemKey::TrackArtists);
    let (album_artist_credit, album_artists) = credit(ItemKey::AlbumArtist, ItemKey::AlbumArtists);

    let track = tags.number(ItemKey::TrackNumber, ItemKey::TrackTotal);
    let disc = tags.number(ItemKey::DiscNumber, ItemKey::DiscTotal);

    let original_date = tags
        .texts(ItemKey::OriginalReleaseDate)
        .iter()
        .filter_map(|v| parse_partial_date(v))
        // Vorbis maps both ORIGINALDATE and ORIGINALYEAR here: keep the most precise.
        .max_by_key(|d| (d.month.is_some(), d.day.is_some()))
        .or_else(|| native_original_year(&tags).and_then(|v| parse_partial_date(&v)));

    // MusicBrainz identifiers never contain `/` or `;`, which some taggers use to join
    // several of them in ID3v2.3.
    let mb_ids = |key| split_values(tags.texts(key), &['\0', '/', ';']);

    TagFacts {
        title: tags.text(ItemKey::TrackTitle),
        title_sort: tags.text(ItemKey::TrackTitleSortOrder),
        artist_credit,
        artist_sort: tags.text(ItemKey::TrackArtistSortOrder),
        artists,
        album_title: tags.text(ItemKey::AlbumTitle),
        album_sort: tags.text(ItemKey::AlbumTitleSortOrder),
        album_artist_credit,
        album_artist_sort: tags.text(ItemKey::AlbumArtistSortOrder),
        album_artists,
        track_no: track.number,
        track_number_text: track.text,
        track_total: track.total,
        disc_no: disc.number,
        disc_total: disc.total,
        disc_subtitle: tags.text(ItemKey::SetSubtitle),
        date: tags.date(&[ItemKey::RecordingDate, ItemKey::Year]),
        original_date,
        genres: split_values(tags.texts(ItemKey::Genre), &[';', '\0']),
        compilation: tags.text(ItemKey::FlagCompilation).as_deref() == Some("1"),
        has_embedded_cover: tags.0.iter().any(|t| t.picture_count() > 0),
        mb_recording_id: tags.text(ItemKey::MusicBrainzRecordingId),
        mb_release_track_id: tags.text(ItemKey::MusicBrainzTrackId),
        mb_release_id: tags.text(ItemKey::MusicBrainzReleaseId),
        mb_release_group_id: tags.text(ItemKey::MusicBrainzReleaseGroupId),
        mb_work_id: tags.text(ItemKey::MusicBrainzWorkId),
        mb_artist_ids: mb_ids(ItemKey::MusicBrainzArtistId),
        mb_album_artist_ids: mb_ids(ItemKey::MusicBrainzReleaseArtistId),
    }
}

struct NumberPair {
    text: Option<String>,
    number: Option<u32>,
    total: Option<u32>,
}

/// A track or disc number as written (`"3"`, `"3/12"`, `"A1"`) plus the separate
/// total field. The text keeps what comes before `/`; the number is set only when that
/// text is a plain integer.
fn number_pair(value: Option<String>, total: Option<String>) -> NumberPair {
    let (text, inline_total) = match &value {
        Some(v) => match v.split_once('/') {
            Some((n, t)) => (n.trim(), Some(t.trim())),
            None => (v.trim(), None),
        },
        None => ("", None),
    };
    let text = (!text.is_empty()).then(|| text.to_string());
    NumberPair {
        number: text.as_deref().and_then(|t| t.parse::<u32>().ok()),
        total: total
            .and_then(|t| t.trim().parse::<u32>().ok())
            .or_else(|| inline_total.and_then(|t| t.parse::<u32>().ok())),
        text,
    }
}

/// `ORIGINALYEAR` written as a user text frame (ID3v2) or a freeform atom (MP4), which
/// lofty does not map to a generic key (in Vorbis comments it does).
fn native_original_year(tags: &Tags<'_>) -> Option<String> {
    const NAME: &str = "ORIGINALYEAR";
    let non_empty = |s: &str| {
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_string())
    };
    tags.0
        .iter()
        .filter(|t| t.has_format_specific_items())
        .find_map(|t| match t.tag_type() {
            TagType::Id3v2 => {
                let id3 = Id3v2Tag::from((*t).clone());
                id3.into_iter().find_map(|frame| match frame {
                    Frame::UserText(f) if f.description.eq_ignore_ascii_case(NAME) => {
                        non_empty(&f.content)
                    }
                    _ => None,
                })
            }
            TagType::Mp4Ilst => {
                let ilst = Ilst::from((*t).clone());
                ilst.into_iter().find_map(|atom| match atom.ident() {
                    AtomIdent::Freeform { name, .. } if name.eq_ignore_ascii_case(NAME) => {
                        atom.data().find_map(|d| match d {
                            AtomData::UTF8(s) => non_empty(s),
                            _ => None,
                        })
                    }
                    _ => None,
                })
            }
            _ => None,
        })
}

/// Parse a date as MusicBrainz writes it: `YYYY`, `YYYY-MM` or `YYYY-MM-DD`. Any text
/// that starts with four digits gives at least the year (`"1975-03-01T00:00"` gives
/// 1975-03-01, `"1975?"` gives 1975). A month or day out of range is dropped, with
/// everything after it.
pub fn parse_partial_date(text: &str) -> Option<PartialDate> {
    let text = text.trim();
    let bytes = text.as_bytes();
    if bytes.len() < 4 || !bytes[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year: i32 = text[..4].parse().ok()?;
    // Two digits after a `-` at `at`, or nothing.
    let part = |at: usize| -> Option<u8> {
        if bytes.get(at) != Some(&b'-') {
            return None;
        }
        let digits = bytes.get(at + 1..at + 3)?;
        if !digits.iter().all(u8::is_ascii_digit)
            || bytes.get(at + 3).is_some_and(u8::is_ascii_digit)
        {
            return None;
        }
        text[at + 1..at + 3].parse().ok()
    };
    let month = part(4).filter(|m| (1..=12).contains(m));
    let day = month.and(part(7)).filter(|d| (1..=31).contains(d));
    Some(PartialDate { year, month, day })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: Option<u8>, day: Option<u8>) -> Option<PartialDate> {
        Some(PartialDate { year, month, day })
    }

    #[test]
    fn partial_dates() {
        assert_eq!(parse_partial_date("1975"), date(1975, None, None));
        assert_eq!(parse_partial_date(" 1975-10 "), date(1975, Some(10), None));
        assert_eq!(
            parse_partial_date("1975-10-31"),
            date(1975, Some(10), Some(31))
        );
        assert_eq!(
            parse_partial_date("1975-03-01T00:00"),
            date(1975, Some(3), Some(1))
        );
        assert_eq!(parse_partial_date("1975?"), date(1975, None, None));
        assert_eq!(parse_partial_date("1975-13-01"), date(1975, None, None));
        assert_eq!(parse_partial_date("1975-02-32"), date(1975, Some(2), None));
        assert_eq!(parse_partial_date("1975-1-5"), date(1975, None, None));
        assert_eq!(parse_partial_date("1975-100"), date(1975, None, None));
        assert_eq!(parse_partial_date("19751031"), date(1975, None, None));
        assert_eq!(parse_partial_date("75"), None);
        assert_eq!(parse_partial_date("c. 1975"), None);
        assert_eq!(parse_partial_date(""), None);
    }

    #[test]
    fn number_pairs() {
        let p = number_pair(Some("3/12".into()), None);
        assert_eq!(
            (p.text.as_deref(), p.number, p.total),
            (Some("3"), Some(3), Some(12))
        );
        let p = number_pair(Some(" A1 ".into()), None);
        assert_eq!(
            (p.text.as_deref(), p.number, p.total),
            (Some("A1"), None, None)
        );
        let p = number_pair(Some("4".into()), Some("10".into()));
        assert_eq!(
            (p.text.as_deref(), p.number, p.total),
            (Some("4"), Some(4), Some(10))
        );
        // The separate total field wins over the inline one.
        let p = number_pair(Some("4/9".into()), Some("10".into()));
        assert_eq!(p.total, Some(10));
        let p = number_pair(Some("/12".into()), None);
        assert_eq!((p.text, p.number, p.total), (None, None, Some(12)));
        let p = number_pair(None, None);
        assert_eq!((p.text, p.number, p.total), (None, None, None));
    }

    #[test]
    fn values_split_and_trimmed() {
        let values = vec!["Rock; Pop, Funk;;".to_string(), "A\0B".to_string()];
        assert_eq!(
            split_values(values, &[';', '\0']),
            vec!["Rock", "Pop, Funk", "A", "B"]
        );
    }
}
