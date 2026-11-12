# Messy library

A small library of real Creative Commons recordings whose metadata and layout are damaged
on purpose, the way real collections are: video style titles, missing tags, missing
covers, editions with slightly different album titles, the same recording in two formats,
compilations, wrong case. It is used to measure identification and duplicate detection
against a known ground truth.

Build it with:

```
cargo run -p armonika-testdata --bin gen-messy -- --manifest bench/messy/manifest.toml --sources bench/data/cc-sources --out bench/data/messy
```

Missing sources are downloaded from `source_url` and checked against `sha256`. The output
folder gets one file per entry of `ground_truth.json`. The manifest format is described in
`manifest.toml`.

## Provenance and license

| Source id | Artist and release | License | Download |
| --------- | ------------------ | ------- | -------- |
