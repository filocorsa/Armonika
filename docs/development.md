# Development notes

## Requirements

- Rust, at the version pinned in `rust-toolchain.toml` (rustup installs it on first use).
- ffmpeg, with ffprobe: used by the test data generators and by tests.
- `fpcalc` from Chromaprint: computes audio fingerprints.
- Docker: used by the spikes and later by the server image.
- Python 3: runs `scripts/check_text_style.py`.

Fingerprint lookups need an AcoustID application key. Register an application at
https://acoustid.org/new-application and put the key in the `ARMONIKA_ACOUSTID_KEY`
environment variable. Never commit the key or write it to a file that is tracked.

Before every commit run:

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
python scripts/check_text_style.py
```

## Large fake library (benchmark data)

The search benchmark and later tests use a deterministic fake library made of silent
template files with generated tags. Generated data goes in `bench/data/`, which is
git-ignored.

```
cargo run --release -p armonika-testdata --bin gen-big -- --tracks 100000 --seed 1 --out bench/data/big-100k
```

The same seed always gives a byte-identical `manifest.json`. Paths in the manifest are
relative to the output folder and always use forward slashes, so manifests compare equal
across platforms.

The generator caps relative paths at 200 characters, so on Windows keep the output root under about 55 characters: the documented `bench/data/big-100k` inside a deep checkout may exceed the 260-character path limit for tools like ffmpeg, fpcalc or Python.

Measured on the development machine (Windows 11, release build, 7 formats in rotation):

- 100,000 files generated in 534 s (about 9 minutes), mostly file system overhead.
- 1.7 GB on disk (WAV and AIFF templates are about 35 KB each; the rest are small).
- `manifest.json` has 100,000 entries.

## Messy library (identification test data)

`gen-messy` builds a library of real Creative Commons recordings with deliberately damaged
metadata and a `ground_truth.json`. See `bench/messy/README.md`. It needs ffmpeg on `PATH`
for the format duplicates.
