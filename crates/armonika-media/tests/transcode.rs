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
fn plan_raw_when_requested_format_is_the_source() {
    let plan = transcode::plan(
        AudioFormat::Flac,
        Some(900),
        Some("flac"),
        Some(9999),
        DEFAULTS,
    );
    assert_eq!(plan, TranscodePlan::Raw);
    let plan = transcode::plan(
        AudioFormat::M4a,
        Some(256),
        Some("aac"),
        Some(9999),
        DEFAULTS,
    );
    assert_eq!(plan, TranscodePlan::Raw);
    let plan = transcode::plan(AudioFormat::M4a, Some(256), Some("M4A"), None, DEFAULTS);
    assert_eq!(plan, TranscodePlan::Raw);
    // Below the source rate it is still encoded, to the default format.
    let plan = transcode::plan(
        AudioFormat::Flac,
        Some(900),
        Some("flac"),
        Some(128),
        DEFAULTS,
    );
    assert_eq!(
        plan,
        TranscodePlan::Encode {
            format: TargetFormat::Mp3,
            kbps: 128
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

/// A stand-in for ffmpeg: a script that runs `unix` (sh) or `windows` (cmd) and ignores
/// its arguments.
fn fake_ffmpeg(dir: &Path, unix: &str, windows: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        let path = dir.join("fake-ffmpeg.cmd");
        fs::write(&path, format!("@echo off\r\n{windows}\r\n")).unwrap();
        path
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("fake-ffmpeg.sh");
            fs::write(&path, format!("#!/bin/sh\n{unix}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            return path;
        }
        #[allow(unreachable_code)]
        {
            let _ = unix;
            unreachable!()
        }
    }
}

/// Starts the fake ffmpeg at `fake`. On Linux, a test running at the same time can
/// fork while this test still has the script open for writing; the child keeps that
/// file open until it runs its own program, and running the script meanwhile fails
/// with "Text file busy". That only lasts a moment, so try again.
fn spawn_fake(fake: &Path) -> transcode::TranscodeStream {
    let mut tries = 0;
    loop {
        match transcode::spawn(fake, Path::new("in.wav"), ENCODE, None) {
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 100 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result.unwrap(),
        }
    }
}

const ENCODE: TranscodePlan = TranscodePlan::Encode {
    format: TargetFormat::Mp3,
    kbps: 128,
};

#[tokio::test]
async fn client_disconnect_kills_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    // Runs until killed and writes nothing, ignoring a closed output: only the kill on
    // drop can end it. On Windows the loop stays inside cmd: a child process such as
    // `ping` would outlive the kill and hold the pipes open, so the test runtime would
    // wait for it at exit.
    let fake = fake_ffmpeg(dir.path(), "trap '' PIPE\nsleep 60", ":loop\r\ngoto loop");
    let stream = spawn_fake(&fake);
    let pid = stream.id().expect("a running process has a pid");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(alive(pid), "the fake ffmpeg should be running");
    drop(stream);
    for _ in 0..50 {
        if !alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the process is still running after the stream was dropped");
}

#[tokio::test]
async fn failing_ffmpeg_ends_with_an_error_before_any_audio() {
    let dir = tempfile::tempdir().unwrap();
    let fake = fake_ffmpeg(
        dir.path(),
        "echo boom >&2\nexit 3",
        "echo boom 1>&2\r\nexit /b 3",
    );
    let mut stream = spawn_fake(&fake);
    let err = stream.prime().await.unwrap_err();
    assert!(err.to_string().contains("ffmpeg failed"), "{err}");
    // A failure is an error, then the end.
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn ffmpeg_that_exits_cleanly_without_output_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let fake = fake_ffmpeg(dir.path(), "exit 0", "exit /b 0");
    let mut stream = spawn_fake(&fake);
    assert!(stream.prime().await.is_err());
}

#[tokio::test]
async fn failure_after_some_audio_is_an_error_not_a_clean_end() {
    let dir = tempfile::tempdir().unwrap();
    let fake = fake_ffmpeg(dir.path(), "echo audio\nexit 1", "echo audio\r\nexit /b 1");
    let mut stream = spawn_fake(&fake);
    stream.prime().await.unwrap();
    let mut saw_error = false;
    while let Some(item) = stream.next().await {
        saw_error |= item.is_err();
    }
    assert!(saw_error);
}

#[tokio::test]
async fn heavy_error_output_does_not_block_the_audio() {
    let dir = tempfile::tempdir().unwrap();
    let line = "e".repeat(200);
    // More than 1 MiB of error output before any audio.
    let fake = fake_ffmpeg(
        dir.path(),
        &format!("i=0\nwhile [ $i -lt 6000 ]; do echo {line} >&2; i=$((i+1)); done\necho audio"),
        &format!("for /L %%i in (1,1,6000) do @echo {line} 1>&2\r\necho audio"),
    );
    let mut stream = spawn_fake(&fake);
    tokio::time::timeout(Duration::from_secs(60), async {
        stream.prime().await.unwrap();
        let mut total = 0;
        while let Some(item) = stream.next().await {
            total += item.unwrap().len();
        }
        assert!(total >= 5, "the audio was delivered");
    })
    .await
    .expect("the stream must not hang on a full error pipe");
}
