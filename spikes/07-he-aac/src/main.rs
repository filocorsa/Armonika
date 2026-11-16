//! Spike 7: what Armonika does with an HE-AAC m4a file.
//!
//! Usage: `cargo run --release -- <file.m4a>`. Prints what `probe::read_file` reads,
//! the `content_hash`, and what Symphonia decodes (sample rate, channels, frames,
//! duration, decode errors).

use std::fs::File;
use std::path::PathBuf;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

fn main() {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .expect("usage: spike-he-aac <file.m4a>")
        .into();

    println!("== probe::read_file");
    match armonika_media::probe::read_file(&path) {
        Ok(facts) => println!("{facts:#?}"),
        Err(e) => println!("error: {e}"),
    }

    println!("== content_hash");
    match armonika_media::content_hash::content_hash(&path) {
        Ok(hash) => {
            let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
            println!("{hex}");
        }
        Err(e) => println!("error: {e}"),
    }

    println!("== symphonia decode");
    match decode(&path) {
        Ok(()) => {}
        Err(e) => println!("error: {e}"),
    }
}

fn decode(path: &PathBuf) -> Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("probe: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("no audio codec params")?
        .clone();
    println!(
        "container: sample_rate {:?}, channels {:?}",
        params.sample_rate, params.channels
    );
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder: {e}"))?;

    let (mut rate, mut channels, mut frames, mut errors) = (0u32, 0usize, 0u64, 0u32);
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
                rate = spec.rate();
                channels = spec.channels().count();
                frames += (audio.samples_interleaved() / channels.max(1)) as u64;
            }
            Err(SymError::DecodeError(_)) => errors += 1,
            Err(e) => return Err(format!("decode: {e}")),
        }
    }
    let seconds = frames as f64 / f64::from(rate.max(1));
    println!(
        "decoded: {rate} Hz, {channels} channels, {frames} frames, {seconds:.2} s, {errors} decode errors"
    );
    Ok(())
}
