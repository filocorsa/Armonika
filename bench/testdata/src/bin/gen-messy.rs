//! `gen-messy --manifest <FILE> --sources <DIR> --out <DIR>`: build the messy library.
//!
//! Missing sources are downloaded from their `source_url`; every source is checked against
//! its `sha256` before use. Then the damages are applied and `ground_truth.json` is written.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use armonika_testdata::messy::{
    Source, build_messy_library, find_source_file, parse_manifest, source_extension, verify_sha256,
};
use sha2::{Digest, Sha256};

const USAGE: &str = "usage: gen-messy --manifest <FILE> --sources <DIR> --out <DIR>";

fn download(source: &Source, sources_dir: &std::path::Path) -> Result<PathBuf> {
    let ext = source_extension(&source.source_url)
        .with_context(|| format!("cannot find a file extension in {}", source.source_url))?;
    let ua = format!(
        "Armonika/{} ( https://github.com/filocorsa/Armonika )",
        env!("CARGO_PKG_VERSION")
    );
    let client = reqwest::blocking::Client::builder()
        .user_agent(ua)
        .timeout(Duration::from_secs(600))
        .build()?;
    println!("downloading {}", source.source_url);
    let bytes = client
        .get(&source.source_url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("downloading {}", source.source_url))?
        .bytes()
        .with_context(|| format!("reading {}", source.source_url))?;
    let digest = Sha256::digest(&bytes);
    let actual: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if !actual.eq_ignore_ascii_case(source.sha256.trim()) {
        bail!(
            "sha256 mismatch for source {:?} ({}): expected {}, got {}",
            source.id,
            source.source_url,
            source.sha256.trim(),
            actual
        );
    }
    let path = sources_dir.join(format!("{}.{ext}", source.id));
    let part = sources_dir.join(format!("{}.{ext}.part", source.id));
    let mut f = fs::File::create(&part)?;
    f.write_all(&bytes)?;
    drop(f);
    fs::rename(&part, &path)?;
    Ok(path)
}

fn main() -> Result<()> {
    let mut manifest: Option<PathBuf> = None;
    let mut sources: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match a.as_str() {
            "--manifest" => manifest = Some(PathBuf::from(value("--manifest")?)),
            "--sources" => sources = Some(PathBuf::from(value("--sources")?)),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    let (Some(manifest_path), Some(sources_dir), Some(out)) = (manifest, sources, out) else {
        bail!("{USAGE}");
    };
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = parse_manifest(&text)?;
    fs::create_dir_all(&sources_dir)?;
    for source in &manifest.sources {
        match find_source_file(&sources_dir, &source.id) {
            Some(path) => verify_sha256(&path, &source.sha256)
                .with_context(|| format!("source {:?}", source.id))?,
            None => {
                download(source, &sources_dir)?;
            }
        }
    }
    let truth = build_messy_library(&manifest, &sources_dir, &out)?;
    println!(
        "generated {} files from {} sources under {}",
        truth.len(),
        manifest.sources.len(),
        out.display()
    );
    Ok(())
}
