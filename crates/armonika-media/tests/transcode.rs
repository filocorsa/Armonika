//! Transcode planning, and ffmpeg itself when it is installed.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use armonika_core::model::AudioFormat;
use armonika_media::probe::read_file;
use armonika_media::transcode::{self, TargetFormat, TranscodePlan};
use futures_util::StreamExt;

const DEFAULTS: (&str, u32) = ("mp3", 192);

#[test]
fn plan_raw_when_no_params() {
    let plan = transcode::plan(AudioFormat::Flac, Some(900), None, None, DEFAULTS);
    assert_eq!(plan, TranscodePlan::Raw);
    let plan = transcode::plan(AudioFormat::Flac, Some(900), None, Some(0), DEFAULTS);
    assert_eq!(plan, TranscodePlan::Raw);
    let plan = transcode::plan(
        AudioFormat::Flac,
        Some(900),
        Some("raw"),
        Some(64),
        DEFAULTS,
    );
    assert_eq!(plan, TranscodePlan::Raw);
}

#[test]
fn plan_raw_when_max_above_source() {
    let plan = transcode::plan(AudioFormat::Mp3, Some(128), None, Some(320), DEFAULTS);
    assert_eq!(plan, TranscodePlan::Raw);
    let plan = transcode::plan(
        AudioFormat::Mp3,
        Some(128),
        Some("mp3"),
        Some(128),
        DEFAULTS,
    );
    assert_eq!(plan, TranscodePlan::Raw);
}

#[test]
fn plan_encode_with_max_below_source() {
    let plan = transcode::plan(AudioFormat::Mp3, Some(320), None, Some(128), DEFAULTS);
    assert_eq!(
        plan,
        TranscodePlan::Encode {
            format: TargetFormat::Mp3,
            kbps: 128
        }
    );
    // A different format is encoded even without a limit, at the default rate.
    let plan = transcode::plan(AudioFormat::Flac, Some(900), Some("opus"), None, DEFAULTS);
    assert_eq!(
        plan,
        TranscodePlan::Encode {
            format: TargetFormat::Opus,
            kbps: 192
        }
    );
    // The limit never raises the default.
    let plan = transcode::plan(
        AudioFormat::Flac,
        Some(900),
        Some("aac"),
        Some(500),
        DEFAULTS,
    );
    assert_eq!(
        plan,
        TranscodePlan::Encode {
            format: TargetFormat::Aac,
            kbps: 192
        }
    );
}

#[test]
fn plan_unknown_format_uses_default() {
    let plan = transcode::plan(
        AudioFormat::Flac,
        Some(900),
        Some("wma"),
        Some(96),
        ("opus", 128),
    );
    assert_eq!(
        plan,
        TranscodePlan::Encode {
            format: TargetFormat::Opus,
            kbps: 96
        }
    );
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A silent 16-bit stereo WAV file of `seconds` seconds.
fn write_wav(path: &Path, seconds: u32) {
    let rate = 44_100u32;
    let data_len = rate * 4 * seconds;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.resize(44 + data_len as usize, 0);
    fs::write(path, bytes).unwrap();
}

#[tokio::test]
async fn mp3_transcode_produces_decodable_audio() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg is not on the PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.wav");
    write_wav(&input, 3);
    let plan = TranscodePlan::Encode {
        format: TargetFormat::Mp3,
        kbps: 128,
    };
    let mut stream = transcode::spawn(Path::new("ffmpeg"), &input, plan, None).unwrap();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }
    let output = dir.path().join("out.mp3");
    fs::write(&output, out).unwrap();
    let facts = read_file(&output).unwrap();
    assert_eq!(facts.format, AudioFormat::Mp3);
    let ms = facts.duration_ms.unwrap();
    assert!((2_500..=3_500).contains(&ms), "duration {ms} ms");
}

/// Whether the process `pid` is still running.
fn alive(pid: u32) -> bool {
    if cfg!(windows) {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    } else {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

#[tokio::test]
async fn client_disconnect_kills_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg is not on the PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.wav");
    // Long enough for the output to fill the pipe, so ffmpeg waits for a reader.
    write_wav(&input, 120);
    let plan = TranscodePlan::Encode {
        format: TargetFormat::Mp3,
        kbps: 320,
    };
    let mut stream = transcode::spawn(Path::new("ffmpeg"), &input, plan, None).unwrap();
    let pid = stream.id().expect("a running process has a pid");
    assert!(stream.next().await.unwrap().is_ok());
    assert!(alive(pid));
    drop(stream);
    for _ in 0..50 {
        if !alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("ffmpeg is still running after the stream was dropped");
}
