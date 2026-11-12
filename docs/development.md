# Development notes

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

Measured on the development machine (Windows 11, release build, 7 formats in rotation):

- 100,000 files generated in 534 s (about 9 minutes), mostly file system overhead.
- 1.7 GB on disk (WAV and AIFF templates are about 35 KB each; the rest are small).
- `manifest.json` has 100,000 entries.
