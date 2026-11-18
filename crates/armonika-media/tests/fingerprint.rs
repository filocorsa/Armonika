use std::path::{Path, PathBuf};

use armonika_media::fingerprint::{Decoder, FingerprintError, fingerprint};

fn templates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/templates")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn every_format_gives_a_fingerprint() {
    for ext in ["mp3", "flac", "ogg", "opus", "m4a"] {
        let path = fixture(&format!("tone.{ext}"));
        let fp = fingerprint(&path, None).unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert!(!fp.fingerprint.is_empty(), "{ext}: empty fingerprint");
        let expected = if ext == "opus" {
            Decoder::OpusPure
        } else {
            Decoder::Symphonia
        };
        assert_eq!(fp.decoder, expected, "{ext}");
    }
}

#[test]
fn templates_are_too_short() {
    for ext in ["mp3", "flac", "ogg", "opus", "m4a", "wav", "aiff"] {
        let path = templates_dir().join(format!("silence.{ext}"));
        assert!(
            matches!(fingerprint(&path, None), Err(FingerprintError::TooShort(_))),
            "{ext}"
        );
    }
}

/// 30 s of a three-sine chord, 16-bit stereo at 44.1 kHz, interleaved.
fn chord() -> Vec<i16> {
    let rate = 44_100.0_f64;
    let mut out = Vec::new();
    for i in 0..(44_100 * 30) {
        let t = f64::from(i) / rate;
        let mut v = 0.0;
        for (k, f) in [261.63_f64, 329.63, 392.0].iter().enumerate() {
            v += (std::f64::consts::TAU * f * t * (1.0 + k as f64 * 0.003 * t)).sin();
        }
        let left = (v / 3.0 * 20_000.0) as i16;
        let right = (v / 3.0 * 12_000.0 * (t * 0.7).cos()) as i16;
        out.push(left);
        out.push(right);
    }
    out
}

fn wav_bytes(pcm: &[i16]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&44_100u32.to_le_bytes());
    b.extend_from_slice(&(44_100u32 * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

fn aiff_bytes(pcm: &[i16]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let frames = (pcm.len() / 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"FORM");
    b.extend_from_slice(&(4 + 8 + 18 + 8 + 8 + data_len).to_be_bytes());
    b.extend_from_slice(b"AIFFCOMM");
    b.extend_from_slice(&18u32.to_be_bytes());
    b.extend_from_slice(&2u16.to_be_bytes());
    b.extend_from_slice(&frames.to_be_bytes());
    b.extend_from_slice(&16u16.to_be_bytes());
    // 44100 Hz as an 80-bit extended float.
    b.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
    b.extend_from_slice(b"SSND");
    b.extend_from_slice(&(8 + data_len).to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_be_bytes());
    }
    b
}

#[test]
fn same_pcm_gives_same_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let pcm = chord();
    let wav = dir.path().join("chord.wav");
    let aiff = dir.path().join("chord.aiff");
    std::fs::write(&wav, wav_bytes(&pcm)).unwrap();
    std::fs::write(&aiff, aiff_bytes(&pcm)).unwrap();
    let a = fingerprint(&wav, None).unwrap();
    let b = fingerprint(&aiff, None).unwrap();
    assert!(!a.fingerprint.is_empty());
    assert_eq!(a.fingerprint, b.fingerprint);
}

#[test]
fn unsupported_codec_without_ffmpeg_is_unsupported() {
    let mut bytes = std::fs::read(fixture("tone.m4a")).unwrap();
    let at = bytes
        .windows(4)
        .position(|w| w == b"mp4a")
        .expect("the m4a template has an mp4a sample entry");
    bytes[at..at + 4].copy_from_slice(b"zzzz");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("odd.m4a");
    std::fs::write(&path, bytes).unwrap();
    assert!(matches!(
        fingerprint(&path, None),
        Err(FingerprintError::Unsupported { .. })
    ));
}

#[test]
fn truncated_file_is_an_error_not_a_panic() {
    let bytes = std::fs::read(fixture("tone.flac")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cut.flac");
    std::fs::write(&path, &bytes[..2048.min(bytes.len())]).unwrap();
    assert!(fingerprint(&path, None).is_err());
}

/// Items of a compressed fingerprint (the format `fpcalc` prints).
fn decompress(encoded: &str) -> Vec<u32> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .unwrap();
    let count = u32::from_be_bytes([0, bytes[1], bytes[2], bytes[3]]) as usize;
    let body = &bytes[4..];
    let bit_at = |data: &[u8], index: usize| (data[index / 8] >> (index % 8)) & 1;
    let read = |data: &[u8], at: usize, width: usize| {
        (0..width).fold(0u32, |v, i| v | (u32::from(bit_at(data, at + i)) << i))
    };
    let mut symbols = Vec::new();
    let mut zeros = 0;
    let mut at = 0;
    while zeros < count {
        let s = read(body, at, 3);
        at += 3;
        zeros += usize::from(s == 0);
        symbols.push(s);
    }
    let exceptional = &body[at.div_ceil(8)..];
    let mut ex_at = 0;
    let mut items = Vec::new();
    let mut previous = 0u32;
    let mut bit = 0u32;
    let mut value = 0u32;
    for s in symbols {
        if s == 0 {
            previous ^= value;
            items.push(previous);
            value = 0;
            bit = 0;
            continue;
        }
        bit += s;
        if s == 7 {
            bit += read(exceptional, ex_at, 5);
            ex_at += 5;
        }
        value |= 1 << (bit - 1);
    }
    items
}

/// Manual check against the spike 4 fingerprints; not run in CI.
#[test]
#[ignore]
fn matches_spike_fingerprints() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/data");
    let features = root.join("recognition/features.json");
    let messy = root.join("messy");
    let Ok(text) = std::fs::read_to_string(&features) else {
        eprintln!("skipped: {} is absent", features.display());
        return;
    };
    let rows: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    let mut worst = 100.0_f64;
    for row in &rows {
        let name = row["file"].as_str().unwrap();
        let expected = decompress(row["fingerprint"].as_str().unwrap());
        let got = match fingerprint(&messy.join(name), None) {
            Ok(fp) => decompress(&fp.fingerprint),
            Err(e) => panic!("{name}: {e}"),
        };
        let n = expected.len().min(got.len());
        let same: u32 = (0..n)
            .map(|i| 32 - (expected[i] ^ got[i]).count_ones())
            .sum();
        let pct = f64::from(same) / (n as f64 * 32.0) * 100.0;
        eprintln!(
            "{name}: {pct:.2}% ({} vs {} items)",
            expected.len(),
            got.len()
        );
        worst = worst.min(pct);
        // The spike did not apply the M4A edit list, so for M4A files this measures
        // the effect of skipping the encoder delay (the spike reports 96.8% as the
        // minimum there); every other format must be identical.
        let minimum = if name.ends_with(".m4a") { 96.0 } else { 98.0 };
        assert!(pct >= minimum, "{name}: only {pct:.2}% equal bits");
    }
    eprintln!("{} files, worst {worst:.2}%", rows.len());
}

#[test]
fn a_bad_first_audio_frame_does_not_abort_the_file() {
    let mut bytes = std::fs::read(fixture("tone.mp3")).unwrap();
    // The first real audio frame starts at 202 (frame 20 is the Info frame).
    for b in &mut bytes[206..358] {
        *b = 0xAA;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad-first-frame.mp3");
    std::fs::write(&path, bytes).unwrap();
    let fp = fingerprint(&path, None).unwrap();
    assert!(!fp.fingerprint.is_empty());
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
fn unsupported_codec_falls_back_to_ffmpeg() {
    let Some(ffmpeg) = ffmpeg_on_path() else {
        eprintln!("skipped: ffmpeg is not on PATH");
        return;
    };
    let mut bytes = std::fs::read(fixture("tone.m4a")).unwrap();
    let at = bytes.windows(4).position(|w| w == b"mp4a").unwrap();
    bytes[at..at + 4].copy_from_slice(b"zzzz");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("odd.m4a");
    std::fs::write(&path, bytes).unwrap();
    // Symphonia refuses the sample entry; ffmpeg reads the codec from the esds box.
    let fp = fingerprint(&path, Some(&ffmpeg)).unwrap();
    assert_eq!(fp.decoder, Decoder::Ffmpeg);
    assert!(!fp.fingerprint.is_empty());
}
