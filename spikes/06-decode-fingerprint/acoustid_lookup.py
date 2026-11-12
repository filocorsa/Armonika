#!/usr/bin/env python3
"""Spike 6, step 3: AcoustID lookup with fpcalc and rusty-chromaprint fingerprints.

Reads results.json written by the Rust spike, picks 20 files of the messy
library (different source per file, quota per format), looks each up on
AcoustID with the fpcalc fingerprint and with the rusty-chromaprint
fingerprint of every decoder variant that worked, and compares the top
recording with fpcalc's and with the ground truth.

The key comes from ARMONIKA_ACOUSTID_KEY and is never printed or written.
Responses are cached in bench/data/spike06/acoustid-cache/ (git-ignored).

Usage: python spikes/06-decode-fingerprint/acoustid_lookup.py \
    bench/data/spike06/results.json bench/data/messy/ground_truth.json \
    bench/data/spike06/acoustid.json
"""
import hashlib
import json
import os
import sys
import time
import urllib.parse
import urllib.request

USER_AGENT = "Armonika/0.0.0 ( https://github.com/filocorsa/Armonika )"
URL = "https://api.acoustid.org/v2/lookup"
QUOTA = [("opus", 6), ("m4a", 4), ("flac", 4), ("mp3", 3), ("ogg", 3)]
MIN_INTERVAL = 1.0 / 3.0

_last_request = 0.0


def lookup(key, cache_dir, fingerprint, duration):
    global _last_request
    ident = hashlib.sha256(f"{duration}:{fingerprint}".encode()).hexdigest()[:32]
    cache = os.path.join(cache_dir, ident + ".json")
    if os.path.exists(cache):
        with open(cache, encoding="utf-8") as f:
            return json.load(f)
    wait = _last_request + MIN_INTERVAL - time.monotonic()
    if wait > 0:
        time.sleep(wait)
    body = urllib.parse.urlencode(
        {
            "client": key,
            "duration": str(duration),
            "fingerprint": fingerprint,
            "meta": "recordingids",
            "format": "json",
        }
    ).encode()
    req = urllib.request.Request(URL, data=body, headers={"User-Agent": USER_AGENT})
    _last_request = time.monotonic()
    with urllib.request.urlopen(req, timeout=30) as resp:
        data = json.load(resp)
    if data.get("status") == "ok":
        with open(cache, "w", encoding="utf-8") as f:
            json.dump(data, f)
    return data


def top(data):
    """Top result: (score, first recording id, all recording ids)."""
    if data.get("status") != "ok":
        return (None, None, [], data.get("error", {}).get("message", "error"))
    results = sorted(data.get("results", []), key=lambda r: -r.get("score", 0))
    for r in results:
        recs = [x["id"] for x in r.get("recordings", [])]
        if recs:
            return (r["score"], recs[0], recs, None)
    if results:
        return (results[0]["score"], None, [], "no recordings")
    return (None, None, [], "no results")


def main():
    results_path, truth_path, out_path = sys.argv[1:4]
    key = os.environ.get("ARMONIKA_ACOUSTID_KEY", "")
    if not key:
        sys.exit("ARMONIKA_ACOUSTID_KEY is not set")
    cache_dir = os.path.join(os.path.dirname(out_path), "acoustid-cache")
    os.makedirs(cache_dir, exist_ok=True)

    with open(results_path, encoding="utf-8") as f:
        results = {r["file"]: r for r in json.load(f)}
    with open(truth_path, encoding="utf-8") as f:
        truth = {t["file"]: t for t in json.load(f)}

    chosen, used_sources = [], set()
    for fmt, n in QUOTA:
        picked = 0
        for name in sorted(truth):
            if picked == n:
                break
            if not name.endswith("." + fmt) or truth[name]["source_id"] in used_sources:
                continue
            chosen.append(name)
            used_sources.add(truth[name]["source_id"])
            picked += 1

    rows = []
    for name in chosen:
        r = results[name]
        gt = truth[name]["recording_mbid"]
        fp_dur = int(round(r["fpcalc_duration_s"]))
        f_score, f_rec, f_recs, f_err = top(lookup(key, cache_dir, r["fpcalc_compressed"], fp_dur))
        row = {
            "file": name,
            "ground_truth": gt,
            "fpcalc": {
                "duration": fp_dur,
                "score": f_score,
                "top_recording": f_rec,
                "gt_in_top_result": gt in f_recs,
                "error": f_err,
            },
            "variants": {},
        }
        for v in r["variants"]:
            if not v["ok"]:
                continue
            dur = int(round(v["duration_s"]))
            score, rec, recs, err = top(lookup(key, cache_dir, v["compressed"], dur))
            row["variants"][v["variant"]] = {
                "duration": dur,
                "score": score,
                "top_recording": rec,
                "same_as_fpcalc": rec is not None and rec == f_rec,
                "gt_in_top_result": gt in recs,
                "error": err,
            }
        rows.append(row)
        line = f"{name:<36} fpcalc score={f_score} rec={f_rec} gt={gt in f_recs}"
        for vn, vv in row["variants"].items():
            line += f" | {vn}: score={vv['score']} same={vv['same_as_fpcalc']} gt={vv['gt_in_top_result']}"
        print(line)

    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(rows, f, indent=2)
    print(f"wrote {out_path} ({len(rows)} files)")


if __name__ == "__main__":
    main()
