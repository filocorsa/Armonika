//! Spike 6: audio decoding and Chromaprint fingerprinting.
//!
//! For every file in the given directories, decode it with every applicable
//! decoder variant, fingerprint the first 120 seconds with rusty-chromaprint
//! (fpcalc-equivalent preset) and compare with `fpcalc -raw`.
//!
//! Usage: spike-decode-fingerprint <out.json> <dir or file> [<dir or file> ...]

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use base64::Engine;
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
/// Offsets (in fingerprint items) tried when aligning two fingerprints.
const MAX_OFFSET: i64 = 20;

struct Pcm {
    samples: Vec<i16>,
    rate: u32,
    channels: u32,
    decode_errors: u32,
}

#[derive(Serialize)]
struct VariantResult {
    variant: String,
    ok: bool,
    error: Option<String>,
    decode_errors: u32,
    rate: u32,
    channels: u32,
    duration_s: f64,
    decode_ms: f64,
    fingerprint_ms: f64,
    items: usize,
    similarity_pct: f64,
    best_offset: i64,
    overlap_items: usize,
    exact_equal: bool,
    compressed: String,
}

#[derive(Serialize)]
struct FileResult {
    file: String,
    path: String,
    format: String,
    fpcalc_ms: f64,
    fpcalc_duration_s: f64,
    fpcalc_items: usize,
    fpcalc_compressed: String,
    fpcalc_recompressed_equal: bool,
    variants: Vec<VariantResult>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: {} <out.json> <dir> [<dir> ...]", args[0]);
        std::process::exit(2);
    }
    if args[1] == "count-packets" {
        count_packets(Path::new(&args[2]));
        return;
    }
    let out = PathBuf::from(&args[1]);
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in &args[2..] {
        if Path::new(dir).is_file() {
            files.push(PathBuf::from(dir));
            continue;
        }
        for entry in std::fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if is_audio(&path) {
                files.push(path);
            }
        }
    }
    files.sort();

    let config = Configuration::preset_test2();
    let mut results = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let ext = extension(path);
        let (fp_raw, fp_compressed, fp_duration, fpcalc_ms) = run_fpcalc(path);
        let recompressed = compress(&config, &fp_raw);

        let mut variants: Vec<&str> = vec!["symphonia"];
        if ext == "opus" {
            variants.extend(["symphonia+libopus", "opus-decoder", "opus-pure"]);
        }
        if ext == "m4a" {
            // Experiment: Symphonia ignores the MP4 edit list (elst media time 1024,
            // the AAC encoder priming written by ffmpeg); drop those frames by hand.
            variants.push("symphonia+skip1024");
        }
        variants.push("ffmpeg");

        let mut vresults = Vec::new();
        for variant in variants {
            let start = Instant::now();
            let decoded = match variant {
                "symphonia" => decode_symphonia(path, false),
                "symphonia+libopus" => decode_symphonia(path, true),
                "symphonia+skip1024" => decode_symphonia(path, false).map(|mut p| {
                    let n = (1024 * p.channels as usize).min(p.samples.len());
                    p.samples.drain(..n);
                    p
                }),
                "opus-decoder" => decode_opus_decoder(path),
                "opus-pure" => decode_opus_pure(path),
                "ffmpeg" => decode_ffmpeg(path),
                _ => unreachable!(),
            };
            let decode_ms = start.elapsed().as_secs_f64() * 1000.0;
            let vr = match decoded {
                Err(e) => VariantResult {
                    variant: variant.to_string(),
                    ok: false,
                    error: Some(e),
                    decode_errors: 0,
                    rate: 0,
                    channels: 0,
                    duration_s: 0.0,
                    decode_ms,
                    fingerprint_ms: 0.0,
                    items: 0,
                    similarity_pct: 0.0,
                    best_offset: 0,
                    overlap_items: 0,
                    exact_equal: false,
                    compressed: String::new(),
                },
                Ok(pcm) => {
                    if let Ok(dir) = std::env::var("SPIKE_DUMP_DIR") {
                        // Diagnostic only: raw interleaved s16le of the decoded audio.
                        let bytes: Vec<u8> = pcm.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                        let dump = Path::new(&dir).join(format!("{name}.{variant}.s16"));
                        std::fs::write(dump, bytes).expect("write dump");
                    }
                    let start = Instant::now();
                    let fp = fingerprint(&config, &pcm);
                    let fingerprint_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let (sim, offset, overlap) = similarity(&fp, &fp_raw);
                    let frames = pcm.samples.len() as f64 / f64::from(pcm.channels.max(1));
                    VariantResult {
                        variant: variant.to_string(),
                        ok: !fp.is_empty(),
                        error: if fp.is_empty() {
                            Some("empty fingerprint".to_string())
                        } else {
                            None
                        },
                        decode_errors: pcm.decode_errors,
                        rate: pcm.rate,
                        channels: pcm.channels,
                        duration_s: frames / f64::from(pcm.rate.max(1)),
                        decode_ms,
                        fingerprint_ms,
                        items: fp.len(),
                        similarity_pct: sim,
                        best_offset: offset,
                        overlap_items: overlap,
                        exact_equal: fp == fp_raw,
                        compressed: compress(&config, &fp),
                    }
                }
            };
            println!(
                "{:<40} {:<18} ok={:<5} sim={:>7.3}% off={:>3} items={:>5}/{:<5} dec={:>8.1}ms fp={:>7.1}ms {}",
                name,
                vr.variant,
                vr.ok,
                vr.similarity_pct,
                vr.best_offset,
                vr.items,
                fp_raw.len(),
                vr.decode_ms,
                vr.fingerprint_ms,
                vr.error.clone().unwrap_or_default()
            );
            vresults.push(vr);
        }
        results.push(FileResult {
            file: name,
            path: path.to_string_lossy().to_string(),
            format: ext,
            fpcalc_ms,
            fpcalc_duration_s: fp_duration,
            fpcalc_items: fp_raw.len(),
            fpcalc_recompressed_equal: recompressed == fp_compressed,
            fpcalc_compressed: fp_compressed,
            variants: vresults,
        });
    }
    std::fs::write(&out, serde_json::to_string_pretty(&results).unwrap()).expect("write out");
    println!("wrote {}", out.display());
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

/// Best percentage of equal bits over the overlapping part, trying small offsets.
/// Returns (percentage, offset of `a` relative to `b`, overlapping items).
fn similarity(a: &[u32], b: &[u32]) -> (f64, i64, usize) {
    let mut best = (0.0, 0, 0);
    for offset in -MAX_OFFSET..=MAX_OFFSET {
        let (mut equal, mut total) = (0u64, 0usize);
        for (i, &x) in a.iter().enumerate() {
            let j = i as i64 + offset;
            if j < 0 || j as usize >= b.len() {
                continue;
            }
            let y = b[j as usize];
            equal += u64::from(32 - (x ^ y).count_ones());
            total += 1;
        }
        if total == 0 {
            continue;
        }
        let pct = equal as f64 * 100.0 / (total as f64 * 32.0);
        if pct > best.0 {
            best = (pct, offset, total);
        }
    }
    best
}

fn run_fpcalc(path: &Path) -> (Vec<u32>, String, f64, f64) {
    let start = Instant::now();
    let raw = Command::new("fpcalc")
        .arg("-raw")
        .arg("-json")
        .arg(path)
        .output()
        .expect("run fpcalc");
    let fpcalc_ms = start.elapsed().as_secs_f64() * 1000.0;
    let raw: serde_json::Value = serde_json::from_slice(&raw.stdout).expect("fpcalc raw json");
    let fp: Vec<u32> = raw["fingerprint"]
        .as_array()
        .expect("fingerprint array")
        .iter()
        .map(|v| v.as_i64().expect("int") as u32)
        .collect();
    let duration = raw["duration"].as_f64().unwrap_or(0.0);
    let comp = Command::new("fpcalc").arg("-json").arg(path).output().expect("run fpcalc");
    let comp: serde_json::Value = serde_json::from_slice(&comp.stdout).expect("fpcalc json");
    let compressed = comp["fingerprint"].as_str().unwrap_or_default().to_string();
    (fp, compressed, duration, fpcalc_ms)
}

fn decode_symphonia(path: &Path, with_libopus: bool) -> Result<Pcm, String> {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    if with_libopus {
        #[cfg(feature = "libopus")]
        registry.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
        #[cfg(not(feature = "libopus"))]
        return Err("built without libopus feature".to_string());
    }
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
        decode_errors: 0,
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
            Err(SymError::DecodeError(_)) => pcm.decode_errors += 1,
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
        samples.extend(trim.keep(&packet, &block[..n * channels]).iter().map(|&x| f32_to_i16(x)));
    }
    Ok(Pcm {
        samples,
        rate: 48_000,
        channels: channels as u32,
        decode_errors: 0,
    })
}

/// opus-decoder has no Ogg demuxer: packets come from opus-pure's Ogg reader,
/// the pre-skip is dropped by hand.
fn decode_opus_decoder(path: &Path) -> Result<Pcm, String> {
    use opus_pure::OggOpusReader;
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = OggOpusReader::new(BufReader::new(file)).map_err(|e| e.to_string())?;
    let channels = reader.head().channel_count as usize;
    let pre_skip = reader.head().pre_skip as usize * channels;
    let mut decoder =
        opus_decoder::OpusDecoder::new(48_000, channels).map_err(|e| format!("{e:?}"))?;
    let mut block = vec![0i16; 5760 * channels];
    let mut samples = Vec::new();
    for packet in reader.packets() {
        let packet = packet.map_err(|e| e.to_string())?;
        let n = decoder
            .decode(&packet.data, &mut block, false)
            .map_err(|e| format!("{e:?}"))?;
        samples.extend_from_slice(&block[..n * channels]);
    }
    let samples = samples.split_off(pre_skip.min(samples.len()));
    Ok(Pcm {
        samples,
        rate: 48_000,
        channels: channels as u32,
        decode_errors: 0,
    })
}

fn decode_ffmpeg(path: &Path) -> Result<Pcm, String> {
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries"])
        .args(["stream=sample_rate,channels", "-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    let probe: serde_json::Value =
        serde_json::from_slice(&probe.stdout).map_err(|e| format!("ffprobe json: {e}"))?;
    let stream = &probe["streams"][0];
    let rate: u32 = stream["sample_rate"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or("ffprobe: no sample rate")?;
    let channels = stream["channels"].as_u64().ok_or("ffprobe: no channels")? as u32;
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "s16le", "-acodec", "pcm_s16le", "-"])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        return Err(format!("ffmpeg exit {}", out.status));
    }
    let samples = out
        .stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(Pcm {
        samples,
        rate,
        channels,
        decode_errors: 0,
    })
}

/// Diagnostic: packets and their total size as seen by Symphonia's Ogg reader
/// and by opus-pure's Ogg reader.
fn count_packets(path: &Path) {
    let file = File::open(path).expect("open");
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(&Hint::new(), mss, FormatOptions::default(), MetadataOptions::default())
        .expect("probe");
    let (mut n, mut bytes, mut dur) = (0u64, 0usize, 0u64);
    while let Ok(Some(p)) = format.next_packet() {
        n += 1;
        bytes += p.data.len();
        dur += p.dur.get();
    }
    println!("symphonia ogg: packets={n} bytes={bytes} dur={dur}");
    let file = File::open(path).expect("open");
    let mut reader = opus_pure::OggOpusReader::new(BufReader::new(file)).expect("ogg");
    let (mut n, mut bytes) = (0u64, 0usize);
    for p in reader.packets() {
        let p = p.expect("packet");
        n += 1;
        bytes += p.data.len();
    }
    println!("opus-pure ogg: packets={n} bytes={bytes}");
}
