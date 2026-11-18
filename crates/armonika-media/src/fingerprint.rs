//! Chromaprint fingerprint of the first 120 seconds of an audio file.
//!
//! The audio is decoded to 16-bit PCM (Symphonia; opus-pure for Opus; ffmpeg as a
//! fallback for codecs Symphonia does not know) and fed to rusty-chromaprint with the
//! same configuration as `fpcalc`. The string returned is what `fpcalc` prints: the
//! compressed fingerprint in URL-safe base64 without padding. Files are only read.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Seconds of audio fingerprinted, like `fpcalc`.
pub const FINGERPRINT_SECONDS: u32 = 120;

/// Less audio than this cannot give a useful fingerprint.
const MIN_SECONDS: u32 = 3;

/// How long the ffmpeg fallback may take.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(60);

/// How much of the ffmpeg error output is kept for the error message.
const FFMPEG_STDERR_BYTES: u64 = 4096;

/// The largest `moov` box the edit list reader will load.
const MAX_MOOV_BYTES: u64 = 64 * 1024 * 1024;

/// What decoded the audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoder {
    Symphonia,
    OpusPure,
    Ffmpeg,
}

impl Decoder {
    pub fn as_str(self) -> &'static str {
        match self {
            Decoder::Symphonia => "symphonia",
            Decoder::OpusPure => "opus-pure",
            Decoder::Ffmpeg => "ffmpeg",
        }
    }
}

/// The fingerprint of a file and the decoder that produced the audio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioFingerprint {
    pub fingerprint: String,
    pub decoder: Decoder,
}

#[derive(Debug, thiserror::Error)]
pub enum FingerprintError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not supported: {message}", path.display())]
    Unsupported { path: PathBuf, message: String },
    #[error("cannot decode {}: {message}", path.display())]
    Decode { path: PathBuf, message: String },
    #[error("{} has less than {MIN_SECONDS} seconds of audio", .0.display())]
    TooShort(PathBuf),
}

/// Interleaved 16-bit PCM.
pub(crate) struct Pcm {
    pub samples: Vec<i16>,
    pub rate: u32,
    pub channels: u32,
}

impl Pcm {
    /// Samples per channel.
    pub(crate) fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
}

/// Fingerprints the first [`FINGERPRINT_SECONDS`] of the file. `ffmpeg`, when given,
/// decodes the codecs Symphonia does not support.
pub fn fingerprint(
    path: &Path,
    ffmpeg: Option<&Path>,
) -> Result<AudioFingerprint, FingerprintError> {
    let (pcm, decoder) = match (decode_pcm(path), ffmpeg) {
        (Err(FingerprintError::Unsupported { .. }), Some(ffmpeg)) => (
            isolated(path, || decode_ffmpeg(ffmpeg, path))?,
            Decoder::Ffmpeg,
        ),
        (Err(e), _) => return Err(e),
        (Ok(decoded), _) => decoded,
    };
    if pcm.frames() < (pcm.rate * MIN_SECONDS) as usize {
        return Err(FingerprintError::TooShort(path.to_path_buf()));
    }
    let fingerprint = isolated(path, || chromaprint(path, &pcm))?;
    Ok(AudioFingerprint {
        fingerprint,
        decoder,
    })
}

/// Runs a decoder, turning a panic of a codec on a malformed file into an error of
/// that file only.
fn isolated<T>(
    path: &Path,
    work: impl FnOnce() -> Result<T, FingerprintError>,
) -> Result<T, FingerprintError> {
    match panic::catch_unwind(AssertUnwindSafe(work)) {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(decode_error(
                path,
                format!("the audio reader failed: {message}"),
            ))
        }
    }
}

fn chromaprint(path: &Path, pcm: &Pcm) -> Result<String, FingerprintError> {
    let config = Configuration::preset_test2();
    let mut printer = Fingerprinter::new(&config);
    printer
        .start(pcm.rate, pcm.channels)
        .map_err(|e| decode_error(path, e))?;
    printer.consume(&pcm.samples);
    printer.finish();
    let bytes = FingerprintCompressor::from(&config).compress(printer.fingerprint());
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn io_error(path: &Path, source: std::io::Error) -> FingerprintError {
    FingerprintError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn decode_error(path: &Path, message: impl std::fmt::Display) -> FingerprintError {
    FingerprintError::Decode {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

fn unsupported(path: &Path, message: impl std::fmt::Display) -> FingerprintError {
    FingerprintError::Unsupported {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

/// Decodes at most [`FINGERPRINT_SECONDS`] of audio, isolated against panics.
pub(crate) fn decode_pcm(path: &Path) -> Result<(Pcm, Decoder), FingerprintError> {
    if extension(path) == "opus" {
        let pcm = isolated(path, || decode_opus(path))?;
        return Ok((pcm, Decoder::OpusPure));
    }
    let pcm = isolated(path, || decode_symphonia(path))?;
    Ok((pcm, Decoder::Symphonia))
}

fn decode_symphonia(path: &Path) -> Result<Pcm, FingerprintError> {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(&extension(path));
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| match e {
            SymError::Unsupported(m) => unsupported(path, m),
            e => decode_error(path, format!("probe: {e}")),
        })?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| unsupported(path, "no audio track"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| unsupported(path, "no audio codec parameters"))?;
    let mut decoder = registry
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| unsupported(path, format!("decoder: {e}")))?;
    let edit = match extension(path).as_str() {
        "m4a" | "mp4" | "m4b" => mp4_edit_list_skip(path),
        _ => None,
    };
    let mut pcm = Pcm {
        samples: Vec::new(),
        rate: 0,
        channels: 0,
    };
    let mut buf: Vec<i16> = Vec::new();
    let mut skip_frames = 0usize;
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(decode_error(path, format!("read packet: {e}"))),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = audio.spec();
                pcm.rate = spec.rate();
                pcm.channels = spec.channels().count() as u32;
                if let Some((media_time, timescale)) = edit {
                    skip_frames = media_time
                        .saturating_mul(u64::from(pcm.rate))
                        .checked_div(u64::from(timescale))
                        .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
                }
                buf.resize(audio.samples_interleaved(), 0);
                audio.copy_to_slice_interleaved(&mut buf);
                pcm.samples.extend_from_slice(&buf);
                let wanted =
                    skip_frames.saturating_add(pcm.rate as usize * FINGERPRINT_SECONDS as usize);
                if pcm.frames() >= wanted {
                    break;
                }
            }
            Err(SymError::DecodeError(_)) => {}
            Err(SymError::Unsupported(m)) => return Err(unsupported(path, m)),
            Err(e) => return Err(decode_error(path, format!("decode: {e}"))),
        }
    }
    if pcm.samples.is_empty() {
        return Err(decode_error(path, "no samples decoded"));
    }
    let channels = pcm.channels.max(1) as usize;
    let skip = skip_frames.saturating_mul(channels).min(pcm.samples.len());
    pcm.samples.drain(..skip);
    pcm.samples
        .truncate(pcm.rate as usize * FINGERPRINT_SECONDS as usize * channels);
    Ok(pcm)
}

fn f32_to_i16(x: f32) -> i16 {
    (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

fn decode_opus(path: &Path) -> Result<Pcm, FingerprintError> {
    use opus_pure::{MAX_PACKET_SAMPLES, OggOpusReader, Trim};
    const RATE: i32 = 48_000;
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    let mut reader = OggOpusReader::new(BufReader::new(file)).map_err(|e| decode_error(path, e))?;
    let channels = reader.head().channel_count as usize;
    let mut decoder = reader
        .head()
        .decoder(RATE)
        .map_err(|e| decode_error(path, e))?;
    let mut trim = Trim::new(reader.head(), RATE, channels).map_err(|e| decode_error(path, e))?;
    let mut block = vec![0.0f32; MAX_PACKET_SAMPLES * channels];
    let mut pcm = Pcm {
        samples: Vec::new(),
        rate: RATE as u32,
        channels: channels as u32,
    };
    let limit = RATE as usize * FINGERPRINT_SECONDS as usize * channels;
    for packet in reader.packets() {
        let packet = packet.map_err(|e| decode_error(path, e))?;
        let n = decoder
            .decode(&packet.data, MAX_PACKET_SAMPLES, &mut block)
            .map_err(|e| decode_error(path, e))?;
        pcm.samples.extend(
            trim.keep(&packet, &block[..n * channels])
                .iter()
                .map(|&x| f32_to_i16(x)),
        );
        if pcm.samples.len() >= limit {
            break;
        }
    }
    if pcm.samples.is_empty() {
        return Err(decode_error(path, "no samples decoded"));
    }
    pcm.samples.truncate(limit);
    Ok(pcm)
}

/// Decodes with `ffmpeg` to 44.1 kHz stereo 16-bit PCM, reading only what is needed.
/// A stuck ffmpeg is killed after [`FFMPEG_TIMEOUT`]; the child is always reaped.
fn decode_ffmpeg(ffmpeg: &Path, path: &Path) -> Result<Pcm, FingerprintError> {
    const RATE: u32 = 44_100;
    let mut child = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(path)
        .args(["-t", &FINGERPRINT_SECONDS.to_string()])
        .args([
            "-map", "0:a:0", "-f", "s16le", "-ac", "2", "-ar", "44100", "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| io_error(ffmpeg, e))?;
    let limit = u64::from(RATE) * u64::from(FINGERPRINT_SECONDS) * 4;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(decode_error(path, "ffmpeg has no output"));
    };
    let (tx, rx) = mpsc::channel();
    let audio = thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = stdout.take(limit).read_to_end(&mut bytes);
        let _ = tx.send(());
        (bytes, read)
    });
    let errors = thread::spawn(move || {
        let mut stderr = stderr;
        let mut tail = Vec::new();
        let _ = (&mut stderr)
            .take(FFMPEG_STDERR_BYTES)
            .read_to_end(&mut tail);
        // Keep draining so that ffmpeg never blocks on a full pipe.
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        String::from_utf8_lossy(&tail).trim().to_owned()
    });
    let started = Instant::now();
    let timed_out = rx.recv_timeout(FFMPEG_TIMEOUT).is_err() && !audio.is_finished();
    if timed_out {
        let _ = child.kill();
    }
    let (bytes, read) = audio
        .join()
        .unwrap_or_else(|_| (Vec::new(), Err(std::io::Error::other("reader panicked"))));
    let reached_limit = bytes.len() as u64 >= limit;
    if reached_limit || read.is_err() {
        // Stopped on purpose, or the pipe broke: either way the child is done for.
        let _ = child.kill();
    }
    let status = wait_until(&mut child, started + FFMPEG_TIMEOUT);
    let message = errors.join().unwrap_or_default();
    if timed_out {
        return Err(decode_error(
            path,
            format!("ffmpeg took more than {} s", FFMPEG_TIMEOUT.as_secs()),
        ));
    }
    read.map_err(|e| io_error(path, e))?;
    if !reached_limit && !status.is_some_and(|s| s.success()) {
        return Err(decode_error(path, format!("ffmpeg failed: {message}")));
    }
    if bytes.is_empty() {
        return Err(decode_error(path, "ffmpeg produced no audio"));
    }
    let samples = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect();
    Ok(Pcm {
        samples,
        rate: RATE,
        channels: 2,
    })
}

/// Waits for the child until `deadline`, then kills it; always reaps it.
fn wait_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                return child.wait().ok();
            }
        }
    }
}

/// The first non-empty edit of the audio track's edit list (`moov/trak/edts/elst`):
/// its `media_time` and the track timescale. The `media_time` is the number of samples
/// to skip: the encoder delay for AAC, 0 for ALAC. Symphonia does not expose the box.
fn mp4_edit_list_skip(path: &Path) -> Option<(u64, u32)> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut pos = 0u64;
    while pos.checked_add(8)? <= len {
        let (kind, start, end) = box_header(&mut file, pos, len)?;
        if &kind == b"moov" {
            if end.checked_sub(start)? > MAX_MOOV_BYTES {
                return None;
            }
            file.seek(SeekFrom::Start(start)).ok()?;
            let mut moov = vec![0u8; (end - start) as usize];
            file.read_exact(&mut moov).ok()?;
            return skip_from_moov(&moov);
        }
        pos = end;
    }
    None
}

/// Kind, payload start and end of the box at `pos`. `end` is always after `pos`, so a
/// scan that continues from it makes progress.
fn box_header<F: Read + Seek>(file: &mut F, pos: u64, len: u64) -> Option<([u8; 4], u64, u64)> {
    file.seek(SeekFrom::Start(pos)).ok()?;
    let mut head = [0u8; 8];
    file.read_exact(&mut head).ok()?;
    let size = u64::from(u32::from_be_bytes([head[0], head[1], head[2], head[3]]));
    let kind = [head[4], head[5], head[6], head[7]];
    let (header, size) = match size {
        0 => (8, len.checked_sub(pos)?),
        1 => {
            let mut big = [0u8; 8];
            file.read_exact(&mut big).ok()?;
            (16, u64::from_be_bytes(big))
        }
        n => (8, n),
    };
    let end = pos.checked_add(size)?;
    if size < header || end > len {
        return None;
    }
    Some((kind, pos + header, end))
}

/// Child boxes of `data` as (kind, payload).
fn children(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
        let kind = [data[at + 4], data[at + 5], data[at + 6], data[at + 7]];
        let size = if size == 0 { data.len() - at } else { size };
        if size < 8 || at + size > data.len() {
            break;
        }
        out.push((kind, &data[at + 8..at + size]));
        at += size;
    }
    out
}

fn find<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(data)
        .into_iter()
        .find(|(k, _)| k == kind)
        .map(|(_, payload)| payload)
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn be64(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

fn skip_from_moov(moov: &[u8]) -> Option<(u64, u32)> {
    for (kind, trak) in children(moov) {
        if &kind != b"trak" {
            continue;
        }
        let mdia = find(trak, b"mdia")?;
        let hdlr = find(mdia, b"hdlr")?;
        if hdlr.get(8..12)? != b"soun" {
            continue;
        }
        let mdhd = find(mdia, b"mdhd")?;
        let timescale = if *mdhd.first()? == 1 {
            be32(mdhd, 20)?
        } else {
            be32(mdhd, 12)?
        };
        if timescale == 0 {
            return None;
        }
        let elst = find(find(trak, b"edts")?, b"elst")?;
        let version = *elst.first()?;
        let entries = be32(elst, 4)? as usize;
        let mut at = 8;
        for _ in 0..entries {
            let (media_time, step) = if version == 1 {
                (be64(elst, at + 8)? as i64, 20)
            } else {
                (i64::from(be32(elst, at + 4)? as i32), 12)
            };
            at += step;
            if media_time >= 0 {
                return Some((media_time as u64, timescale));
            }
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m4a_edit_list_priming_is_skipped() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.m4a");
        let (m4a, _) = decode_pcm(&path).unwrap();
        // The fixture is 4 s of mono audio at 44.1 kHz, like the WAV of the same tone.
        let expected = 4 * 44_100_i64;
        let got = m4a.frames() as i64;
        eprintln!(
            "m4a frames {got}, expected {expected}, edit {:?}",
            mp4_edit_list_skip(&path)
        );
        assert!((got - expected).abs() <= 1024, "{got} vs {expected}");
    }

    #[test]
    fn a_wrapping_box_size_ends_the_scan() {
        use std::io::Cursor;
        // A box with a 64-bit size that wraps around when added to its position.
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(b"free");
        data.extend_from_slice(&u64::MAX.to_be_bytes());
        let len = data.len() as u64;
        assert!(box_header(&mut Cursor::new(&data), 8, len).is_none());
        // And a size smaller than the header cannot stand still either.
        let mut tiny = Vec::new();
        tiny.extend_from_slice(&4u32.to_be_bytes());
        tiny.extend_from_slice(b"free");
        assert!(box_header(&mut Cursor::new(&tiny), 0, 8).is_none());
        // Every box a valid header returns ends after its start.
        let mut ok = Vec::new();
        ok.extend_from_slice(&16u32.to_be_bytes());
        ok.extend_from_slice(b"free");
        ok.extend_from_slice(&[0u8; 8]);
        let (_, start, end) = box_header(&mut Cursor::new(&ok), 0, 16).unwrap();
        assert!(end > 0 && start <= end);
    }

    fn ffmpeg_on_path() -> Option<PathBuf> {
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    }

    #[test]
    fn ffmpeg_fallback_decodes_a_fixture() {
        let Some(ffmpeg) = ffmpeg_on_path() else {
            eprintln!("skipped: ffmpeg is not on PATH");
            return;
        };
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.flac");
        let pcm = decode_ffmpeg(&ffmpeg, &path).unwrap();
        assert_eq!(pcm.channels, 2);
        assert!((pcm.frames() as i64 - 4 * 44_100).abs() <= 2048);
    }

    #[test]
    fn ffmpeg_fallback_failure_is_a_clean_error() {
        let Some(ffmpeg) = ffmpeg_on_path() else {
            eprintln!("skipped: ffmpeg is not on PATH");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-audio.m4a");
        std::fs::write(&path, vec![7u8; 4096]).unwrap();
        let err = fingerprint(&path, Some(&ffmpeg)).unwrap_err();
        assert!(!matches!(err, FingerprintError::Io { .. }), "{err}");
    }
}
