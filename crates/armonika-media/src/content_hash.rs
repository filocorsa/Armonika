//! A fingerprint of the audio of a file that does not change when its tags do
//! (spec §8.7, spike 5 variant A).
//!
//! Every compressed packet of the first audio track, as the container demuxer hands
//! it out, goes into BLAKE3 preceded by its length as 4 bytes little-endian. Nothing
//! is decoded: for Ogg and Opus only the Ogg demuxer runs. Tags, covers and padding
//! are not packets, so they are not part of the hash. The file is only read.

use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Why the content hash of a file could not be computed.
#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not a readable audio container: {message}", path.display())]
    Container { path: PathBuf, message: String },
    #[error("{} has no audio track", .0.display())]
    NoAudioTrack(PathBuf),
    #[error("cannot read the audio of {}: {message}", path.display())]
    Packet { path: PathBuf, message: String },
}

/// The content hash of the audio at `path`: 32 bytes of BLAKE3 over the length-prefixed
/// packets of the first audio track.
pub fn content_hash(path: &Path) -> Result<[u8; 32], HashError> {
    let file = File::open(path).map_err(|source| HashError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| HashError::Container {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| HashError::NoAudioTrack(path.to_path_buf()))?
        .id;

    let mut hasher = blake3::Hasher::new();
    loop {
        let packet = format.next_packet().map_err(|e| HashError::Packet {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        let Some(packet) = packet else { break };
        if packet.track_id != track {
            continue;
        }
        // A packet is never 4 GiB; saturate rather than wrap if one ever claimed to be.
        let len = u32::try_from(packet.data.len()).unwrap_or(u32::MAX);
        hasher.update(&len.to_le_bytes());
        hasher.update(&packet.data);
    }
    Ok(*hasher.finalize().as_bytes())
}
