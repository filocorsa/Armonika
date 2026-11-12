//! Spike 4, step 1 (local part): for every file of the messy library, decode the audio
//! with the combination chosen in spike 6 (Symphonia, opus-pure for Opus), compute the
//! Chromaprint fingerprint of the first 120 s with rusty-chromaprint (preset_test2),
//! and read the current tags with lofty. Writes one JSON array; the network part
//! (AcoustID, MusicBrainz) and the scoring are in `recognize.py`.
//!
//! Usage: cargo run --release -- <messy dir> <out.json>

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use base64::Engine;
use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, ItemKey};
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};
use serde::Serialize;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// fpcalc fingerprints only the first 120 seconds by default.
const FP_SECONDS: u64 = 120;

struct Pcm {
    samples: Vec<i16>,
    rate: u32,
    channels: u32,
}

#[derive(Serialize, Default)]
struct Tags {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    track: Option<u32>,
    track_total: Option<u32>,
}

#[derive(Serialize)]
struct FileFeatures {
    file: String,
    duration_s: f64,
    fingerprint: String,
    decoder: String,
    tags: Tags,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: spike-recognition <messy dir> <out.json>");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| is_audio(p))
        .collect();
    paths.sort();
    let config = Configuration::preset_test2();
    let mut rows = Vec::new();
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (pcm, decoder) = if extension(path) == "opus" {
            (decode_opus_pure(path), "opus-pure")
        } else {
            (decode_symphonia(path), "symphonia")
        };
        let pcm = match pcm {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{name}: decode failed: {e}");
                continue;
            }
        };
        let duration_s =
            pcm.samples.len() as f64 / f64::from(pcm.channels) / f64::from(pcm.rate);
        let fp = fingerprint(&config, &pcm);
        let tags = read_tags(path);
        eprintln!("{name}: {duration_s:.1} s, {} items", fp.len());
        rows.push(FileFeatures {
            file: name,
            duration_s,
            fingerprint: compress(&config, &fp),
            decoder: decoder.to_string(),
            tags,
        });
    }
    std::fs::write(&out, serde_json::to_string_pretty(&rows).unwrap()).expect("write out");
    eprintln!("wrote {} ({} files)", out.display(), rows.len());
}

fn read_tags(path: &Path) -> Tags {
    let Ok(file) = lofty::read_from_path(path) else {
        return Tags::default();
    };
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return Tags::default();
    };
    Tags {
        title: tag.title().map(|s| s.into_owned()),
        artist: tag.artist().map(|s| s.into_owned()),
        album: tag.album().map(|s| s.into_owned()),
        album_artist: tag.get_string(ItemKey::AlbumArtist).map(str::to_string),
        track: tag.track(),
        track_total: tag.track_total(),
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_audio(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "mp3" | "flac" | "ogg" | "opus" | "m4a" | "wav" | "aiff"
    )
}

fn compress(config: &Configuration, fp: &[u32]) -> String {
    let bytes = FingerprintCompressor::from(config).compress(fp);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Fingerprint the first `FP_SECONDS` of audio, like fpcalc does by default.
fn fingerprint(config: &Configuration, pcm: &Pcm) -> Vec<u32> {
    let mut printer = Fingerprinter::new(config);
    printer.start(pcm.rate, pcm.channels).expect("fingerprinter start");
    let limit = (u64::from(pcm.rate) * FP_SECONDS * u64::from(pcm.channels)) as usize;
    let n = pcm.samples.len().min(limit);
    printer.consume(&pcm.samples[..n]);
    printer.finish();
    printer.fingerprint().to_vec()
}

fn decode_symphonia(path: &Path) -> Result<Pcm, String> {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(&extension(path));
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("probe: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("no audio codec params")?;
    let mut decoder = registry
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;
    let mut pcm = Pcm {
        samples: Vec::new(),
        rate: 0,
        channels: 0,
    };
    let mut buf: Vec<i16> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("read packet: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = audio.spec();
                pcm.rate = spec.rate();
                pcm.channels = spec.channels().count() as u32;
                buf.resize(audio.samples_interleaved(), 0);
                audio.copy_to_slice_interleaved(&mut buf);
                pcm.samples.extend_from_slice(&buf);
            }
            Err(SymError::DecodeError(_)) => {}
            Err(e) => return Err(format!("decode: {e}")),
        }
    }
    if pcm.samples.is_empty() {
        return Err("no samples decoded".to_string());
    }
    Ok(pcm)
}

fn f32_to_i16(x: f32) -> i16 {
    (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

fn decode_opus_pure(path: &Path) -> Result<Pcm, String> {
    use opus_pure::{MAX_PACKET_SAMPLES, OggOpusReader, Trim};
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = OggOpusReader::new(BufReader::new(file)).map_err(|e| e.to_string())?;
    let channels = reader.head().channel_count as usize;
    let mut decoder = reader.head().decoder(48_000).map_err(|e| e.to_string())?;
    let mut trim = Trim::new(reader.head(), 48_000, channels).map_err(|e| e.to_string())?;
    let mut block = vec![0.0f32; MAX_PACKET_SAMPLES * channels];
    let mut samples = Vec::new();
    for packet in reader.packets() {
        let packet = packet.map_err(|e| e.to_string())?;
        let n = decoder
            .decode(&packet.data, MAX_PACKET_SAMPLES, &mut block)
            .map_err(|e| e.to_string())?;
        samples.extend(
            trim.keep(&packet, &block[..n * channels])
                .iter()
                .map(|&x| f32_to_i16(x)),
        );
    }
    Ok(Pcm {
        samples,
        rate: 48_000,
        channels: channels as u32,
    })
}
