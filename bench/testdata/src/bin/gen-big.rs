//! `gen-big --tracks <N> --seed <S> --out <DIR>`: generate the large fake library.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use armonika_testdata::{AudioFormat, BigLibrarySpec, generate_big_library};

const USAGE: &str = "usage: gen-big --tracks <N> --seed <S> --out <DIR>";

fn main() -> Result<()> {
    let mut tracks: Option<usize> = None;
    let mut seed: Option<u64> = None;
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match a.as_str() {
            "--tracks" => tracks = Some(value("--tracks")?.parse().context("bad --tracks")?),
            "--seed" => seed = Some(value("--seed")?.parse().context("bad --seed")?),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    let (Some(tracks), Some(seed), Some(out)) = (tracks, seed, out) else {
        bail!("{USAGE}");
    };
    let spec = BigLibrarySpec {
        tracks,
        seed,
        formats: vec![
            AudioFormat::Mp3,
            AudioFormat::Flac,
            AudioFormat::OggVorbis,
            AudioFormat::Opus,
            AudioFormat::M4a,
            AudioFormat::Wav,
            AudioFormat::Aiff,
        ],
    };
    let start = Instant::now();
    let manifest = generate_big_library(&spec, &out)?;
    println!(
        "generated {} files in {:.1?} under {}",
        manifest.len(),
        start.elapsed(),
        out.display()
    );
    Ok(())
}
