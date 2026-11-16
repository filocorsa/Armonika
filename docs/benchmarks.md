# Benchmarks

`armonika-bench` (crate `bench/harness`) measures the performance thresholds of section 3
of the design against a real `armonika serve` process. It starts the server on an empty
data directory with the given library and measures, over HTTP:

1. **First scan**: time until the scan ends, and the number of tracks, which must match
   the manifest of the library (folders that differ only by case are compared as one
   folder, as NTFS and APFS store them).
2. **Second scan** with no changes (`startScan`): time and `files_read`, read from the
   `scan finished` log line. Hard threshold: 0 files read.
3. **Search**: 1,000 `search3` queries generated from the manifest with the scheme and
   seed of spike 1 (`prefix`, `typo`, `diacritics`, `multiword`, 250 each), 10 artists,
   10 albums and 10 songs per query, after 50 warm-up queries. p50 and p95 per type, from
   the `Server-Timing: app;dur=<ms>` header of the server and, apart, as seen by the
   client. Hard threshold: server p95 < 50 ms for every type.
4. **First byte** of a `stream` without transcoding on 200 random tracks. Hard
   threshold: p95 < 200 ms.
5. **Idle memory** of the process after 60 s without requests (working set on Windows,
   resident set on Linux), with the size on disk of the search index and the database.

First scan and memory are indicative and only reported. A missed hard threshold ends the
run with exit code 1.

## Commands

```
cargo build --release -p armonika-server -p armonika-bench -p armonika-testdata
target/release/gen-big --tracks 100000 --seed 1 --out bench/data/big-100k
target/release/armonika-bench run \
  --library bench/data/big-100k \
  --manifest bench/data/big-100k/manifest.json \
  --binary target/release/armonika \
  --quiet-timeout-secs 0 \
  --out bench/data/results.json
```

Other options: `--queries` (default 1000), `--streams` (default 200), `--idle-secs`
(default 60) and `--quiet-timeout-secs` (default 3600): how long to wait, before the
search is measured, for the server to stop using CPU. After a scan the content hash runs
in the background (about 27 minutes for 100,000 tracks on the machine below); 0 measures
at once, while the hash is running, which is the worse case and what a user sees right
after adding a library.

CI runs the job `bench-smoke` on Linux on every push that touches code: 5,000 tracks from
`gen-big`, hard thresholds on, idle wait of 5 s, wait for a quiet server capped at 10
minutes. The results are kept as the artifact `bench-smoke-results`.

## Machine

AMD Ryzen 5 5600H (6 cores, 12 threads), 27.9 GB RAM, SSD, Windows 11, rustc
1.99.1, release build. Library and data directory on the same SSD.

## Results, 100,000 tracks

`bench/data/big-100k` from `gen-big --tracks 100000 --seed 1` (7 formats in rotation,
120,000-word vocabulary with accents and CJK words). `--quiet-timeout-secs 0`: search,
first byte and memory were measured while the content hash was running.

| Measure | With CJK fields | Without CJK fields | Threshold |
|---|---|---|---|
| First scan | 72.0 s | 87.4 s | indicative: 50,000 files in < 5 min on HDD |
| Second scan, files read | 0 (3.1 s) | 0 (3.0 s) | 0 |
| Search p95, prefix | 20.5 ms | 17.8 ms | < 50 ms |
| Search p95, typo | 15.0 ms | 13.8 ms | < 50 ms |
| Search p95, diacritics | 16.3 ms | 15.0 ms | < 50 ms |
| Search p95, multiword | 13.9 ms | 12.5 ms | < 50 ms |
| Search p50, all types | 7.5 ms | 7.2 ms | |
| First byte p95 | 2.8 ms | 2.9 ms | < 200 ms |
| Idle memory | 74.1 MB | 69.7 MB | indicative: < 100 MB |
| Search index on disk | 12.6 MB | 12.9 MB | |
| Database on disk | 76.0 MB | 75.6 MB | |

All hard thresholds pass. Search times are server times (`Server-Timing`); the client
adds about 1 ms.

The first scan time varies between runs (62 to 181 s in the runs of this task, the same
binary and library): it depends on the state of the file system cache. It reads 100,000
files, so it is well inside the indicative goal on SSD; the goal is for an HDD and is
measured on real hardware at the end of phase 1.

### CJK fields (design section 11, point 12)

Each text field of the index has a twin field with the CJK bigram tokenizer. The
"without" column is a build where those twin fields are left empty. On this library
(8% of the titles and 20% of the tracks have CJK text in a title, artist or album) the
CJK fields cost nothing measurable: index sizes over seven runs were 12.2 to 12.9 MB with or without them, a spread caused by the
segment layout at the moment of measuring, and the search p95 differs by 1 to 3 ms,
less than the spread between two runs of the same build. The CJK fields stay.

### Search before the fix

The first runs failed the search threshold: server p95 62 to 70 ms for every type, the
same with and without CJK fields and after waiting an hour for the server to go quiet.
Timing each stage of `search3` on the scanned 100,000-track data showed Tantivy and the
re-ranking at 3 to 6 ms p95 (as in spike 1), and the lookup of the found tracks in the
database at 40 ms p50: SQLite has no table statistics, so for
`WHERE t.state = 'present' AND t.library_id IN (...) AND t.public_id IN (...)` it chose
the two-column index `tracks_state` over the unique `public_id`, and read every track
of the library. The same happened for albums. Those lookups now keep the scope columns
out of the choice of index (`+t.library_id`), and tests check the query plans.

| Stage of `search3` (100,000 tracks) | p95 before | p95 after |
|---|---|---|
| Tantivy query and re-ranking | 5 to 7 ms | same |
| Popularity boost of the candidates | up to 5 ms | same |
| Tracks by identifier | 47 to 55 ms | 1.2 to 1.6 ms |
| Albums by identifier | 8 to 10 ms | 1.3 to 1.6 ms |
| Artists by identifier | under 1.1 ms | same |

## Results, 5,000 tracks (the CI smoke size)

Same machine, `bench/data/big-5k`, `--idle-secs 5 --quiet-timeout-secs 0`: first scan
3.4 s, second scan 0 files read, search server p95 6.9 to 8.6 ms by type, first byte p95
2.3 ms, idle memory 29.9 MB.

## Limits

- **Synthetic data.** Tags come from a generator: name lengths, word frequencies and
  the share of accents and CJK text are not those of a real collection, and the audio
  files are tiny silent templates, so scans read little data. Search recall is not
  measured here (see spike 1).
- **SSD.** Every number above is on an SSD. Scans and the first byte will be
  slower on an HDD or a network share; phase 1 ends with a run on the home server
  (plan, Task 25).
- **One machine, one client.** Queries are sent one at a time from the same machine;
  concurrent clients are not measured.
- **Background work.** With `--quiet-timeout-secs 0` the content hash runs during the
  measurements. Before the fix, a run that waited an hour for a quiet server gave the
  same search times as one that did not; after the fix only runs without the wait were
  made.
