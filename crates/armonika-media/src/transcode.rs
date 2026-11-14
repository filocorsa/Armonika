//! On-the-fly transcoding with an external `ffmpeg`.
//!
//! [`plan`] decides whether a track is served as it is or encoded; [`spawn`] runs ffmpeg
//! and hands its output back as a stream. The input file is only read, ffmpeg gets an
//! argument vector (never a shell), and the only values that reach it besides the path
//! are numbers and a codec picked from a fixed set.

use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::task::{Context, Poll};

use armonika_core::model::AudioFormat;
use bytes::Bytes;
use futures_core::Stream;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio_util::io::ReaderStream;

/// Lowest bit rate passed to ffmpeg, in kbit/s.
const MIN_KBPS: u32 = 8;

/// Bit rate used when the configured default is 0.
const FALLBACK_KBPS: u32 = 192;

/// A format ffmpeg can encode to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    Mp3,
    Opus,
    Aac,
}

impl TargetFormat {
    /// The format named `name` (`mp3`, `opus` or `aac`), ignoring case.
    pub fn parse(name: &str) -> Option<TargetFormat> {
        match name.trim().to_ascii_lowercase().as_str() {
            "mp3" => Some(TargetFormat::Mp3),
            "opus" => Some(TargetFormat::Opus),
            "aac" => Some(TargetFormat::Aac),
            _ => None,
        }
    }

    /// The content type of the encoded stream.
    pub fn content_type(self) -> &'static str {
        match self {
            TargetFormat::Mp3 => "audio/mpeg",
            TargetFormat::Opus => "audio/ogg",
            TargetFormat::Aac => "audio/aac",
        }
    }

    fn codec(self) -> &'static str {
        match self {
            TargetFormat::Mp3 => "libmp3lame",
            TargetFormat::Opus => "libopus",
            TargetFormat::Aac => "aac",
        }
    }

    fn container(self) -> &'static str {
        match self {
            TargetFormat::Mp3 => "mp3",
            TargetFormat::Opus => "ogg",
            TargetFormat::Aac => "adts",
        }
    }
}

/// The names a client may use for `format` when it means the format of the file itself.
fn names_of(source: AudioFormat) -> &'static [&'static str] {
    match source {
        AudioFormat::Mp3 => &["mp3"],
        AudioFormat::Flac => &["flac"],
        AudioFormat::Ogg => &["ogg", "oga"],
        AudioFormat::Opus => &["opus"],
        AudioFormat::M4a => &["m4a", "aac", "mp4"],
        AudioFormat::Wav => &["wav"],
        AudioFormat::Aiff => &["aif", "aiff"],
    }
}

/// What to do with a track that is streamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscodePlan {
    /// The file as it is.
    Raw,
    Encode {
        format: TargetFormat,
        kbps: u32,
    },
}

/// Decides how to serve a track. `requested_format` and `max_kbps` are the `format` and
/// `maxBitRate` of the request; `defaults` is the configured format and bit rate.
pub fn plan(
    source_format: AudioFormat,
    source_kbps: Option<u32>,
    requested_format: Option<&str>,
    max_kbps: Option<u32>,
    defaults: (&str, u32),
) -> TranscodePlan {
    let requested = requested_format.map(str::trim).filter(|f| !f.is_empty());
    if requested.is_some_and(|f| f.eq_ignore_ascii_case("raw")) {
        return TranscodePlan::Raw;
    }
    let max_kbps = max_kbps.filter(|k| *k > 0);
    let target = requested.and_then(TargetFormat::parse);
    // A format that is the one of the file is not a request to change it.
    let same_format = requested.is_none_or(|f| {
        names_of(source_format)
            .iter()
            .any(|name| f.eq_ignore_ascii_case(name))
    });
    if same_format {
        let fits = match (max_kbps, source_kbps) {
            (None, _) => true,
            (Some(max), Some(source)) => max >= source,
            (Some(_), None) => false,
        };
        if fits {
            return TranscodePlan::Raw;
        }
    }
    let default_kbps = if defaults.1 == 0 {
        FALLBACK_KBPS
    } else {
        defaults.1
    };
    let format = target
        .or_else(|| TargetFormat::parse(defaults.0))
        .unwrap_or(TargetFormat::Mp3);
    let kbps = max_kbps.map_or(default_kbps, |max| max.min(default_kbps));
    TranscodePlan::Encode {
        format,
        kbps: kbps.max(MIN_KBPS),
    }
}

/// How many bytes of the error output of ffmpeg are kept for the log.
const STDERR_TAIL_BYTES: usize = 4096;

type Finish = Pin<Box<dyn Future<Output = io::Result<()>> + Send>>;

enum State {
    Reading,
    /// The output ended: waiting for the exit status of the process.
    Finishing(Finish),
    Done,
}

/// The output of an ffmpeg process. Dropping it kills the process. If ffmpeg exits with
/// an error the stream ends with an error (never as a clean end) and the tail of its
/// error output is logged.
pub struct TranscodeStream {
    // Held so that dropping the stream kills the process (`kill_on_drop`).
    child: Option<Child>,
    stderr: Option<ChildStderr>,
    output: ReaderStream<ChildStdout>,
    prefetched: Option<Bytes>,
    state: State,
}

impl TranscodeStream {
    /// The process id, while the process is running.
    pub fn id(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    /// Waits for the first bytes of audio. An error means ffmpeg failed or ended without
    /// producing any, so nothing was sent yet and the caller can serve the file as it
    /// is. The bytes are kept and come out first.
    pub async fn prime(&mut self) -> io::Result<()> {
        match std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await {
            Some(Ok(first)) => {
                self.prefetched = Some(first);
                Ok(())
            }
            Some(Err(err)) => Err(err),
            None => Err(io::Error::other("ffmpeg produced no audio")),
        }
    }
}

async fn finish(mut child: Child, stderr: Option<ChildStderr>) -> io::Result<()> {
    let mut tail = Vec::new();
    if let Some(stderr) = stderr {
        // Bounded: more than this is not read, and the writer gets a broken pipe.
        let _ = stderr
            .take(STDERR_TAIL_BYTES as u64)
            .read_to_end(&mut tail)
            .await;
    }
    let status = child.wait().await?;
    if status.success() {
        return Ok(());
    }
    tracing::warn!(
        %status,
        stderr = %String::from_utf8_lossy(&tail).trim(),
        "ffmpeg failed"
    );
    Err(io::Error::other(format!("ffmpeg failed: {status}")))
}

impl Stream for TranscodeStream {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;
        if let Some(bytes) = this.prefetched.take() {
            return Poll::Ready(Some(Ok(bytes)));
        }
        loop {
            match &mut this.state {
                State::Reading => match Pin::new(&mut this.output).poll_next(cx) {
                    Poll::Ready(None) => match this.child.take() {
                        Some(child) => {
                            let stderr = this.stderr.take();
                            this.state = State::Finishing(Box::pin(finish(child, stderr)));
                        }
                        None => this.state = State::Done,
                    },
                    other => return other,
                },
                State::Finishing(done) => {
                    let result = std::task::ready!(done.as_mut().poll(cx));
                    this.state = State::Done;
                    return Poll::Ready(result.err().map(Err));
                }
                State::Done => return Poll::Ready(None),
            }
        }
    }
}

/// Starts ffmpeg on `input` for a plan that encodes (`Raw` is an error) and returns its
/// output. `time_offset_s` skips that many seconds of the track. Nothing is written to
/// disk and the input is never opened for writing.
pub fn spawn(
    ffmpeg: &Path,
    input: &Path,
    plan: TranscodePlan,
    time_offset_s: Option<u32>,
) -> io::Result<TranscodeStream> {
    let TranscodePlan::Encode { format, kbps } = plan else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a raw plan has nothing to encode",
        ));
    };
    let mut command = Command::new(ffmpeg);
    command.args(["-v", "error", "-nostdin"]);
    if let Some(offset) = time_offset_s.filter(|o| *o > 0) {
        command.arg("-ss").arg(offset.to_string());
    }
    command
        .arg("-i")
        .arg(input)
        .args(["-map", "0:a:0", "-vn", "-c:a", format.codec()])
        .arg("-b:a")
        .arg(format!("{kbps}k"))
        .args(["-f", format.container(), "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("ffmpeg has no output"))?;
    Ok(TranscodeStream {
        stderr: child.stderr.take(),
        child: Some(child),
        output: ReaderStream::new(stdout),
        prefetched: None,
        state: State::Reading,
    })
}
