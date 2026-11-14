//! What a file says about itself (audio properties and tags, as read by the scanner)
//! and the events the scanner emits about a library.
//!
//! [`FileFacts`] and [`TagFacts`] carry no database identity: `armonika-media` fills
//! them from a file and the scanner copies them into the database. [`LibraryChange`]
//! names the tracks, albums and artists a scan wrote, by public identifier.

use crate::ids::{ArtistId, LibraryAlbumId, LibraryId, TrackId};

/// Audio formats accepted by the scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioFormat {
    Mp3,
    Flac,
    /// Ogg Vorbis.
    Ogg,
    Opus,
    M4a,
    Wav,
    Aiff,
}

impl AudioFormat {
    /// The format for a file extension (without the dot), ignoring case. `aif` and
    /// `aiff` are both AIFF.
    pub fn from_extension(ext: &str) -> Option<AudioFormat> {
        let format = match ext.to_ascii_lowercase().as_str() {
            "mp3" => AudioFormat::Mp3,
            "flac" => AudioFormat::Flac,
            "ogg" => AudioFormat::Ogg,
            "opus" => AudioFormat::Opus,
            "m4a" => AudioFormat::M4a,
            "wav" => AudioFormat::Wav,
            "aif" | "aiff" => AudioFormat::Aiff,
            _ => return None,
        };
        Some(format)
    }
}

/// A date that may be only a year, or a year and a month, as in MusicBrainz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartialDate {
    pub year: i32,
    pub month: Option<u8>,
    pub day: Option<u8>,
}

/// The tags of one file. Text values are trimmed; empty values are `None` (or left out
/// of the lists).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagFacts {
    pub title: Option<String>,
    pub title_sort: Option<String>,
    /// The artist as credited on the track (`ARTIST`), for example `"A feat. B"`.
    pub artist_credit: Option<String>,
    pub artist_sort: Option<String>,
    /// The single artists of the track (`ARTISTS`).
    pub artists: Vec<String>,
    pub album_title: Option<String>,
    pub album_sort: Option<String>,
    pub album_artist_credit: Option<String>,
    pub album_artist_sort: Option<String>,
    /// The single artists of the album (`ALBUMARTISTS`).
    pub album_artists: Vec<String>,
    /// The track number, only when the tag holds a plain integer.
    pub track_no: Option<u32>,
    /// The track number as written, without the total: `"3"`, or `"A1"` on a vinyl side.
    pub track_number_text: Option<String>,
    pub track_total: Option<u32>,
    pub disc_no: Option<u32>,
    pub disc_total: Option<u32>,
    pub disc_subtitle: Option<String>,
    /// Release date of this edition.
    pub date: Option<PartialDate>,
    /// First release date of the recording (for remasters and reissues).
    pub original_date: Option<PartialDate>,
    pub genres: Vec<String>,
    pub compilation: bool,
    pub has_embedded_cover: bool,
    pub mb_recording_id: Option<String>,
    pub mb_release_track_id: Option<String>,
    pub mb_release_id: Option<String>,
    pub mb_release_group_id: Option<String>,
    pub mb_work_id: Option<String>,
    pub mb_artist_ids: Vec<String>,
    pub mb_album_artist_ids: Vec<String>,
}

/// Everything the scanner learns from one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    pub size: u64,
    /// Modification time in milliseconds since the Unix epoch.
    pub mtime_ms: i64,
    pub format: AudioFormat,
    pub duration_ms: Option<u64>,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub bit_depth: Option<u8>,
    pub tags: TagFacts,
}

/// What one committed batch of a scan changed in a library, for the clients that
/// keep a view of it (search index, live updates). Every list is without duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryChange {
    pub library: LibraryId,
    /// Tracks created, or whose file, path, tags or state changed and that are present.
    pub tracks_upserted: Vec<TrackId>,
    /// Tracks that became missing: they are no longer shown but keep their data.
    pub tracks_hidden: Vec<TrackId>,
    pub albums_touched: Vec<LibraryAlbumId>,
    pub artists_touched: Vec<ArtistId>,
}

impl LibraryChange {
    /// An event for `library` that changes nothing yet.
    pub fn new(library: LibraryId) -> LibraryChange {
        LibraryChange {
            library,
            tracks_upserted: Vec::new(),
            tracks_hidden: Vec::new(),
            albums_touched: Vec::new(),
            artists_touched: Vec::new(),
        }
    }

    /// Whether the event names nothing.
    pub fn is_empty(&self) -> bool {
        self.tracks_upserted.is_empty()
            && self.tracks_hidden.is_empty()
            && self.albums_touched.is_empty()
            && self.artists_touched.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_extension_ignores_case_and_knows_aif() {
        assert_eq!(AudioFormat::from_extension("MP3"), Some(AudioFormat::Mp3));
        assert_eq!(AudioFormat::from_extension("Flac"), Some(AudioFormat::Flac));
        assert_eq!(AudioFormat::from_extension("ogg"), Some(AudioFormat::Ogg));
        assert_eq!(AudioFormat::from_extension("opus"), Some(AudioFormat::Opus));
        assert_eq!(AudioFormat::from_extension("m4a"), Some(AudioFormat::M4a));
        assert_eq!(AudioFormat::from_extension("WAV"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::from_extension("aif"), Some(AudioFormat::Aiff));
        assert_eq!(AudioFormat::from_extension("AIFF"), Some(AudioFormat::Aiff));
        assert_eq!(AudioFormat::from_extension("txt"), None);
        assert_eq!(AudioFormat::from_extension(""), None);
    }
}
