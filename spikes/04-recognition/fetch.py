#!/usr/bin/env python3
"""Spike 4, step 1 (network part): collect candidates for every file.

For every file in features.json (written by the Rust program): AcoustID lookup with
meta=recordings releases releasegroups, then MusicBrainz details for the recordings of
the AcoustID results (releases, release groups, media, artist credits, length).

Every response is cached on disk, keyed by the request (the AcoustID key is not part
of the cache key and is never printed or written). A rerun with a full cache makes no
network call. Rate limits: AcoustID 3 req/s, MusicBrainz 1 req/s.

Usage: python spikes/04-recognition/fetch.py bench/data/recognition/features.json \
    bench/data/recognition-cache
"""
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

USER_AGENT = "Armonika/0.0.0 ( https://github.com/filocorsa/Armonika )"
ACOUSTID_URL = "https://api.acoustid.org/v2/lookup"
MB_URL = "https://musicbrainz.org/ws/2/recording/"
MB_INC = "releases+release-groups+media+artist-credits"
# Recordings of AcoustID results below this score are not looked up on MusicBrainz.
MIN_RESULT_SCORE = 0.5

_last = {"acoustid": 0.0, "mb": 0.0}
_interval = {"acoustid": 1.0 / 3.0, "mb": 1.1}
stats = {"network": 0, "cache": 0}


def _wait(service):
    wait = _last[service] + _interval[service] - time.monotonic()
    if wait > 0:
        time.sleep(wait)
    _last[service] = time.monotonic()


def cached(cache_dir, kind, key_text, fetch):
    ident = hashlib.sha256(key_text.encode()).hexdigest()[:40]
    path = os.path.join(cache_dir, kind, ident + ".json")
    if os.path.exists(path):
        stats["cache"] += 1
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    data = fetch()
    stats["network"] += 1
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump({"request": key_text, "response": data}, f)
    return {"request": key_text, "response": data}


def acoustid(cache_dir, key, fingerprint, duration):
    params = {
        "duration": str(duration),
        "fingerprint": fingerprint,
        "meta": "recordings releases releasegroups",
        "format": "json",
    }
    key_text = "acoustid:" + urllib.parse.urlencode(sorted(params.items()))

    def fetch():
        if not key:
            sys.exit("ARMONIKA_ACOUSTID_KEY is not set and the response is not cached")
        body = urllib.parse.urlencode({**params, "client": key}).encode()
        req = urllib.request.Request(
            ACOUSTID_URL, data=body, headers={"User-Agent": USER_AGENT}
        )
        _wait("acoustid")
        with urllib.request.urlopen(req, timeout=60) as resp:
            data = json.load(resp)
        if data.get("status") != "ok":
            sys.exit(f"AcoustID error: {data.get('error')}")
        return data

    return cached(cache_dir, "acoustid", key_text, fetch)["response"]


def musicbrainz(cache_dir, recording_id):
    url = f"{MB_URL}{recording_id}?inc={MB_INC}&fmt=json"

    def fetch():
        for attempt in range(5):
            _wait("mb")
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            try:
                with urllib.request.urlopen(req, timeout=60) as resp:
                    return json.load(resp)
            except urllib.error.HTTPError as e:
                if e.code in (503, 429) and attempt < 4:
                    time.sleep(2 + attempt * 2)
                    continue
                if e.code == 404:
                    return {"error": "not found"}
                raise
        raise RuntimeError("unreachable")

    return cached(cache_dir, "musicbrainz", "mb:" + url, fetch)["response"]


def main():
    features_path, cache_dir = sys.argv[1:3]
    key = os.environ.get("ARMONIKA_ACOUSTID_KEY", "")
    with open(features_path, encoding="utf-8") as f:
        features = json.load(f)
    recordings = set()
    for row in features:
        duration = int(round(row["duration_s"]))
        data = acoustid(cache_dir, key, row["fingerprint"], duration)
        n = 0
        for result in data.get("results", []):
            if result.get("score", 0) < MIN_RESULT_SCORE:
                continue
            for rec in result.get("recordings", []):
                recordings.add(rec["id"])
                n += 1
        print(f"{row['file']}: {len(data.get('results', []))} results, {n} recordings")
    print(f"{len(recordings)} distinct recordings to look up on MusicBrainz")
    for i, rid in enumerate(sorted(recordings)):
        musicbrainz(cache_dir, rid)
        if (i + 1) % 20 == 0:
            print(f"  {i + 1}/{len(recordings)}")
    print(f"done: {stats['network']} network requests, {stats['cache']} from cache")


if __name__ == "__main__":
    main()
