#!/usr/bin/env python3
"""Spike 4, steps 2 and 3: clues, confidence score, grid search, report numbers.

Reads features.json (Rust program), the AcoustID and MusicBrainz responses cached by
fetch.py (no network access here), ground_truth.json and the messy manifest. Writes
a JSON with every number used in docs/spikes/04-recognition.md and prints a summary.

Usage: python spikes/04-recognition/recognize.py bench/data/recognition/features.json \
    bench/data/recognition-cache bench/data/messy/ground_truth.json \
    bench/messy/manifest.toml bench/data/recognition/results.json
"""
import hashlib
import itertools
import json
import os
import re
import sys
import tomllib
import unicodedata
import urllib.parse

import numpy as np

MIN_RESULT_SCORE = 0.5
MB_URL = "https://musicbrainz.org/ws/2/recording/"
MB_INC = "releases+release-groups+media+artist-credits"

# ---------------------------------------------------------------------------------------
# Text
# ---------------------------------------------------------------------------------------

NOISE_WORDS = r"official|video|audio|lyrics?|hd|hq|4k|visuali[sz]er|music video|mv"
NOISE_BRACKET = re.compile(r"[\(\[][^\)\]]*\b(" + NOISE_WORDS + r")\b[^\)\]]*[\)\]]", re.I)
FEAT = re.compile(r"\s+(feat\.?|ft\.?|featuring)\s+[^\(\)\[\]]*", re.I)


def base_norm(s):
    """Lowercase, strip accents, punctuation to spaces, collapse spaces."""
    s = unicodedata.normalize("NFKD", s)
    s = "".join(c for c in s if not unicodedata.combining(c)).lower()
    s = re.sub(r"[^\w\s]|_", " ", s)
    return " ".join(s.split())


def clean_title(title, artist):
    """Remove video-site noise: bracketed official/video/HD, feat., 'Artist - ' prefix."""
    t = NOISE_BRACKET.sub(" ", title)
    t = FEAT.sub(" ", t)
    if " - " in t and artist:
        head, tail = t.split(" - ", 1)
        if base_norm(head) == base_norm(artist):
            t = tail
    return t


def jaro_winkler(a, b):
    if a == b:
        return 1.0 if a else 0.0
    la, lb = len(a), len(b)
    if not la or not lb:
        return 0.0
    window = max(la, lb) // 2 - 1
    ma, mb = [False] * la, [False] * lb
    matches = 0
    for i, ca in enumerate(a):
        lo, hi = max(0, i - window), min(lb, i + window + 1)
        for j in range(lo, hi):
            if not mb[j] and b[j] == ca:
                ma[i] = mb[j] = True
                matches += 1
                break
    if not matches:
        return 0.0
    k = trans = 0
    for i in range(la):
        if ma[i]:
            while not mb[k]:
                k += 1
            if a[i] != b[k]:
                trans += 1
            k += 1
    m = matches
    jaro = (m / la + m / lb + (m - trans / 2) / m) / 3
    prefix = 0
    for ca, cb in zip(a[:4], b[:4]):
        if ca != cb:
            break
        prefix += 1
    return jaro + prefix * 0.1 * (1 - jaro)


# ---------------------------------------------------------------------------------------
# Cache
# ---------------------------------------------------------------------------------------


def load_cached(cache_dir, kind, key_text):
    ident = hashlib.sha256(key_text.encode()).hexdigest()[:40]
    path = os.path.join(cache_dir, kind, ident + ".json")
    if not os.path.exists(path):
        sys.exit(f"missing cache entry for {key_text[:80]}: run fetch.py first")
    with open(path, encoding="utf-8") as f:
        return json.load(f)["response"]


def acoustid_response(cache_dir, fingerprint, duration):
    params = {
        "duration": str(duration),
        "fingerprint": fingerprint,
        "meta": "recordings releases releasegroups",
        "format": "json",
    }
    key_text = "acoustid:" + urllib.parse.urlencode(sorted(params.items()))
    return load_cached(cache_dir, "acoustid", key_text)


def mb_recording(cache_dir, rid):
    url = f"{MB_URL}{rid}?inc={MB_INC}&fmt=json"
    return load_cached(cache_dir, "musicbrainz", "mb:" + url)


def credit_string(credits):
    return "".join(c["name"] + c.get("joinphrase", "") for c in credits)


def date_key(d):
    if not d:
        return "9999-99-99"
    parts = d.split("-")
    parts += ["99"] * (3 - len(parts))
    return "-".join(parts)


# ---------------------------------------------------------------------------------------
# Data
# ---------------------------------------------------------------------------------------


def damages_per_file(manifest_path, truth):
    with open(manifest_path, "rb") as f:
        manifest = tomllib.load(f)
    mutating = {}
    for d in manifest.get("damage", []):
        if d["kind"] not in ("duplicate_other_format", "compilation_copy"):
            mutating.setdefault(d["source"], []).append(d["kind"])
    out = {}
    for t in truth:
        name = t["file"]
        stem = name.rsplit(".", 1)[0]
        if stem.endswith("-copy"):
            out[name] = mutating.get(t["source_id"], []) or ["clean"]
        elif stem.endswith("-compilation"):
            out[name] = ["compilation_copy"]
        else:
            out[name] = ["duplicate_other_format"]
    return out


def build(features, cache_dir, truth):
    """Per file: tags and candidate (recording, release) pairs with file-level clues."""
    files = []
    for row in features:
        name = row["file"]
        tags = row["tags"]
        data = acoustid_response(cache_dir, row["fingerprint"], int(round(row["duration_s"])))
        rec_score, rec_result = {}, {}
        track_counts = {}
        for ri, res in enumerate(data.get("results", [])):
            for rec in res.get("recordings", []):
                for rg in rec.get("releasegroups", []):
                    for rel in rg.get("releases", []):
                        if "track_count" in rel:
                            track_counts[rel["id"]] = rel["track_count"]
                if res["score"] < MIN_RESULT_SCORE:
                    continue
                if res["score"] > rec_score.get(rec["id"], -1):
                    rec_score[rec["id"]] = res["score"]
                    rec_result[rec["id"]] = ri
        recordings = {}
        for rid, a in rec_score.items():
            mb = mb_recording(cache_dir, rid)
            if "error" in mb:
                continue
            length = (mb.get("length") or 0) / 1000.0
            releases = []
            for rel in mb.get("releases", []):
                rg = rel.get("release-group", {})
                tc = track_counts.get(rel["id"])
                if tc is None and len(rel.get("media", [])) == 1:
                    tc = rel["media"][0].get("track-count")
                releases.append(
                    {
                        "id": rel["id"],
                        "title": rel.get("title", ""),
                        "date": rel.get("date", ""),
                        "status": rel.get("status"),
                        "rg": rg.get("id"),
                        "rg_title": rg.get("title", ""),
                        "primary": rg.get("primary-type"),
                        "secondary": rg.get("secondary-types", []),
                        "track_count": tc,
                    }
                )
            recordings[rid] = {
                "id": rid,
                "title": mb.get("title", ""),
                "artist": credit_string(mb.get("artist-credit", [])),
                "length": length,
                "acoustid": a,
                "result_index": rec_result[rid],
                "releases": releases,
            }
        files.append(
            {
                "file": name,
                "duration": row["duration_s"],
                "tags": tags,
                "recordings": recordings,
                "truth": truth[name],
            }
        )
    return files


def release_of_gt(files, rel_id):
    for f in files:
        for r in f["recordings"].values():
            for rel in r["releases"]:
                if rel["id"] == rel_id:
                    return rel
    return None


def is_original(rel):
    return (
        rel["status"] == "Official" and rel["primary"] == "Album" and not rel["secondary"]
    )


def groups_before_recognition(files):
    """Design 8.7 point 15: same folder (all one folder here) and same album title and
    album artist tags; a file without album title is a group by itself."""
    groups, index = [], {}
    for i, f in enumerate(files):
        album = f["tags"].get("album")
        if not album:
            groups.append([i])
            continue
        key = (album, f["tags"].get("album_artist"))
        if key not in index:
            index[key] = len(groups)
            groups.append([])
        groups[index[key]].append(i)
    return groups


# ---------------------------------------------------------------------------------------
# Clues
# ---------------------------------------------------------------------------------------

CLUES = ["acoustid", "duration", "title", "artist", "album", "original", "cover", "count"]


def missing_value(missing):
    return float("nan") if missing is None else missing


def text_clues(f, rec, rel, cleanup, missing):
    tags = f["tags"]
    title, artist, album = tags.get("title"), tags.get("artist"), tags.get("album")
    if title:
        t_in = clean_title(title, artist) if cleanup else title
        t = jaro_winkler(base_norm(t_in), base_norm(rec["title"]))
    else:
        t = missing_value(missing)
    r = jaro_winkler(base_norm(artist), base_norm(rec["artist"])) if artist else missing_value(missing)
    l = jaro_winkler(base_norm(album), base_norm(rel["title"])) if album else missing_value(missing)
    return t, r, l


def rows_for_context(files, groups, missing, dur_scale, cleanup=True):
    """One row per (group, file, recording, release). Clue vector in CLUES order."""
    rows = []
    for gi, g in enumerate(groups):
        cover_count = {}
        for i in g:
            seen = set()
            for rec in files[i]["recordings"].values():
                for rel in rec["releases"]:
                    seen.add(rel["id"])
            for rid in seen:
                cover_count[rid] = cover_count.get(rid, 0) + 1
        tag_totals = {files[i]["tags"].get("track_total") for i in g} - {None}
        for i in g:
            f = files[i]
            for rec in f["recordings"].values():
                d = abs(f["duration"] - rec["length"]) if rec["length"] else 1e9
                sd = max(0.0, 1.0 - d / dur_scale)
                for rel in rec["releases"]:
                    t, r, l = text_clues(f, rec, rel, cleanup, missing)
                    tc = rel["track_count"]
                    count = 1.0 if tc and (tc == len(g) or tc in tag_totals) else 0.0
                    rows.append(
                        {
                            "group": gi,
                            "file": i,
                            "rec": rec["id"],
                            "rel": rel["id"],
                            "rg": rel["rg"],
                            "date": date_key(rel["date"]),
                            "v": [
                                rec["acoustid"],
                                sd,
                                t,
                                r,
                                l,
                                1.0 if is_original(rel) else 0.0,
                                cover_count[rel["id"]] / len(g),
                                count,
                            ],
                        }
                    )
    return rows


# ---------------------------------------------------------------------------------------
# Proposal for many weight vectors at once
# ---------------------------------------------------------------------------------------


def propose(files, groups, rows, weights):
    """weights: (n_cfg, 8). Returns per config, per file: chosen row index, score, and
    whether the file stays on the group edition; and the group score per group."""
    n_cfg = weights.shape[0]
    V = np.array([r["v"] for r in rows], dtype=float)  # (n_rows, 8), NaN = missing tag
    present = (~np.isnan(V)).astype(float)
    den = present @ weights.T
    S = 100.0 * (np.nan_to_num(V) @ weights.T) / np.where(den > 0, den, 1.0)
    by_group = {}
    for k, r in enumerate(rows):
        by_group.setdefault(r["group"], {}).setdefault(r["rel"], {}).setdefault(
            r["file"], []
        ).append(k)
    chosen = np.full((len(files), n_cfg), -1, dtype=np.int64)
    on_group = np.zeros((len(files), n_cfg), dtype=bool)
    group_score = np.zeros((len(groups), n_cfg))
    group_rel = {}
    for gi, g in enumerate(groups):
        rels = by_group.get(gi, {})
        if not rels:
            continue
        rel_ids = sorted(rels, key=lambda rid: (rows[next(iter(rels[rid].values()))[0]]["date"], rid))
        gs = np.zeros((len(rel_ids), n_cfg))
        best_row = {}
        for ri, rid in enumerate(rel_ids):
            for fi, ks in rels[rid].items():
                sub = S[ks]  # (len(ks), n_cfg)
                am = sub.argmax(axis=0)
                best_row[(rid, fi)] = np.array(ks)[am]
                gs[ri] += sub.max(axis=0)
        gs /= len(g)
        pick = gs.argmax(axis=0)  # first max: earliest date wins ties
        group_score[gi] = gs.max(axis=0)
        group_rel[gi] = (rel_ids, pick, gs)
        for fi in g:
            for ri, rid in enumerate(rel_ids):
                if (rid, fi) not in best_row:
                    continue
                mask = pick == ri
                chosen[fi, mask] = best_row[(rid, fi)][mask]
                on_group[fi, mask] = True
    return S, chosen, on_group, group_score, group_rel


def evaluate_config_arrays(
    files, groups, ctx_rows, single_rows, weights, comp_files, lenient=True
):
    """Combine group and singleton contexts: a file not on its group's edition falls
    back to its own best proposal (as if the group were split)."""
    S_g, ch_g, on_g, gscore, grel = propose(files, groups, ctx_rows, weights)
    singles = [[i] for i in range(len(files))]
    S_s, ch_s, _, _, _ = propose(files, singles, single_rows, weights)
    n, n_cfg = len(files), weights.shape[0]
    cols = np.arange(n_cfg)
    score = np.zeros((n, n_cfg))
    rec_ok = np.zeros((n, n_cfg), dtype=bool)
    near = np.zeros((n, n_cfg), dtype=bool)
    rg_ok = np.zeros((n, n_cfg), dtype=bool)
    rel_ok = np.zeros((n, n_cfg), dtype=bool)
    wrong_max = np.full((n, n_cfg), -1.0)
    rec_arr_g = np.array([r["rec"] for r in ctx_rows])
    rec_arr_s = np.array([r["rec"] for r in single_rows])
    rel_arr_g = np.array([r["rel"] for r in ctx_rows])
    rel_arr_s = np.array([r["rel"] for r in single_rows])
    rg_arr_g = np.array([str(r["rg"]) for r in ctx_rows])
    rg_arr_s = np.array([str(r["rg"]) for r in single_rows])
    file_g = np.array([r["file"] for r in ctx_rows])
    file_s = np.array([r["file"] for r in single_rows])
    for i, f in enumerate(files):
        gt = f["truth"]
        gt_rg = f["gt_rg"]
        near_ids = f["near_miss_recs"]
        excused = near_ids if lenient else set()
        use_g = on_g[i]
        row = np.where(use_g, ch_g[i], ch_s[i])
        sc = np.where(use_g, S_g[np.maximum(ch_g[i], 0), cols], S_s[np.maximum(ch_s[i], 0), cols])
        sc = np.where(row < 0, 0.0, sc)
        score[i] = sc
        rec_g = rec_arr_g[np.maximum(ch_g[i], 0)]
        rec_s = rec_arr_s[np.maximum(ch_s[i], 0)]
        rec = np.where(use_g, rec_g, rec_s)
        rel = np.where(use_g, rel_arr_g[np.maximum(ch_g[i], 0)], rel_arr_s[np.maximum(ch_s[i], 0)])
        rg = np.where(use_g, rg_arr_g[np.maximum(ch_g[i], 0)], rg_arr_s[np.maximum(ch_s[i], 0)])
        rec_ok[i] = rec == gt["recording_mbid"]
        near[i] = np.isin(rec, list(near_ids)) if near_ids else False
        rg_ok[i] = rg == gt_rg
        rel_ok[i] = rel == gt["release_mbid"]
        # Best wrong candidate in each context. Wrong = other recording (near misses
        # excluded, see doc) or, except for compilation copies, other release group.
        comp = f["file"] in comp_files
        for S, rec_arr, rg_arr, file_arr, sel in (
            (S_g, rec_arr_g, rg_arr_g, file_g, use_g),
            (S_s, rec_arr_s, rg_arr_s, file_s, ~use_g),
        ):
            ks = np.where(file_arr == i)[0]
            if not len(ks):
                continue
            bad = rec_arr[ks] != gt["recording_mbid"]
            bad &= ~np.isin(rec_arr[ks], list(excused)) if excused else True
            if not comp:
                bad |= rg_arr[ks] != gt_rg
            if not bad.any():
                continue
            wm = S[ks[bad]].max(axis=0)
            wrong_max[i] = np.where(sel, np.maximum(wrong_max[i], wm), wrong_max[i])
    return {
        "score": score,
        "rec_ok": rec_ok,
        "near": near,
        "rg_ok": rg_ok,
        "rel_ok": rel_ok,
        "wrong_max": wrong_max,
        "on_group": on_g,
        "group_score": gscore,
        "group_rel": grel,
        "chosen_g": ch_g,
        "chosen_s": ch_s,
    }


# ---------------------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------------------

GRID = {
    "acoustid": [1, 2, 3],
    "duration": [0, 1, 2],
    "title": [0, 1, 2],
    "artist": [0, 1],
    "album": [0, 1],
    "original": [0, 1, 2],
    "cover": [0, 1, 2],
    "count": [0, 1],
}
DUR_SCALES = [10.0, 30.0]
# Value of a text clue when the tag is missing; None = leave the clue out of the score
# (its weight is removed from the denominator too).
MISSING = [0.0, 0.5, None]


def objective(r, idx, comp_mask):
    """Lexicographic: recording right, release group right (no compilation copies),
    coverage (right proposals strictly above the best wrong candidate), then gap
    (lowest right proposal minus best wrong candidate)."""
    rec_ok = r["rec_ok"][idx]
    good = rec_ok & (r["rg_ok"][idx] | comp_mask[idx][:, None])
    t0 = r["wrong_max"][idx].max(axis=0)
    cover = (good & (r["score"][idx] > t0[None, :])).sum(axis=0)
    rg = (r["rg_ok"][idx] & ~comp_mask[idx][:, None]).sum(axis=0)
    gap = np.where(good, r["score"][idx], 1e9).min(axis=0) - t0
    return rec_ok.sum(axis=0), rg, cover, t0, gap


def run_grid(files, groups, comp_files, idx_train, require_text=False):
    """require_text: only weight vectors with title >= 1 and artist >= 1."""
    combos = np.array(list(itertools.product(*[GRID[c] for c in CLUES])), dtype=float)
    combos = combos[combos.sum(axis=1) > 0]
    if require_text:
        combos = combos[(combos[:, 2] >= 1) & (combos[:, 3] >= 1)]
    comp_mask = np.array([f["file"] in comp_files for f in files])
    best = None
    all_results = {}
    for dur in DUR_SCALES:
        for missing in MISSING:
            ctx = rows_for_context(files, groups, missing, dur)
            single = rows_for_context(files, [[i] for i in range(len(files))], missing, dur)
            r = evaluate_config_arrays(files, groups, ctx, single, combos, comp_files)
            all_results[(dur, missing)] = (r, combos)
            rec, rg, cover, t0, gap = objective(r, idx_train, comp_mask)
            wsum = combos.sum(axis=1)
            for c in range(len(combos)):
                key = (int(rec[c]), int(rg[c]), int(cover[c]), round(float(gap[c]), 1), -float(wsum[c]))
                if best is None or key > best[0]:
                    best = (key, dur, missing, c)
    return best, all_results


def main():
    features_path, cache_dir, truth_path, manifest_path, out_path = sys.argv[1:6]
    with open(features_path, encoding="utf-8") as f:
        features = json.load(f)
    with open(truth_path, encoding="utf-8") as f:
        truth_list = json.load(f)
    truth = {t["file"]: t for t in truth_list}
    damages = damages_per_file(manifest_path, truth_list)
    files = build(features, cache_dir, truth)
    for f in files:
        gt_rel = release_of_gt(files, f["truth"]["release_mbid"])
        f["gt_rg"] = gt_rel["rg"] if gt_rel else None
        # Near miss: another recording in the same AcoustID result as the ground truth
        # recording, with the same normalized title (Josh Woodward 2004 vs 2008 remix).
        gt_rec = f["recordings"].get(f["truth"]["recording_mbid"])
        f["near_miss_recs"] = set()
        if gt_rec:
            for r in f["recordings"].values():
                if (
                    r["id"] != gt_rec["id"]
                    and r["result_index"] == gt_rec["result_index"]
                    and base_norm(r["title"]) == base_norm(gt_rec["title"])
                ):
                    f["near_miss_recs"].add(r["id"])
        f["damages"] = damages[f["file"]]
    comp_files = {f["file"] for f in files if "compilation_copy" in f["damages"]}
    groups = groups_before_recognition(files)
    report = {"groups": [[files[i]["file"] for i in g] for g in groups]}

    # ---- Candidate statistics and text similarity (step 2) ----
    cand = []
    for f in files:
        gt = f["truth"]["recording_mbid"]
        recs = sorted(f["recordings"].values(), key=lambda r: -r["acoustid"])
        top_score = recs[0]["acoustid"] if recs else None
        top_recs = [r["id"] for r in recs if r["acoustid"] == top_score]
        gt_rec = f["recordings"].get(gt)
        tags = f["tags"]
        row = {
            "file": f["file"],
            "damages": f["damages"],
            "n_recordings": len(recs),
            "n_releases": len({rel["id"] for r in recs for rel in r["releases"]}),
            "top_acoustid": top_score,
            "gt_in_candidates": gt_rec is not None,
            "gt_acoustid": gt_rec["acoustid"] if gt_rec else None,
            "tied_top_recordings": len(top_recs),
            "gt_alone_at_top": top_recs == [gt],
            "duration_diff_gt": abs(f["duration"] - gt_rec["length"]) if gt_rec else None,
        }
        if gt_rec and tags.get("title"):
            row["title_jw_raw"] = jaro_winkler(base_norm(tags["title"]), base_norm(gt_rec["title"]))
            row["title_jw_clean"] = jaro_winkler(
                base_norm(clean_title(tags["title"], tags.get("artist"))),
                base_norm(gt_rec["title"]),
            )
            row["title_jw_lower_only"] = jaro_winkler(tags["title"].lower(), gt_rec["title"].lower())
        if gt_rec and tags.get("artist"):
            row["artist_jw"] = jaro_winkler(base_norm(tags["artist"]), base_norm(gt_rec["artist"]))
        cand.append(row)
    report["candidates"] = cand

    # ---- Baselines: AcoustID only ----
    base = []
    for f in files:
        recs = sorted(f["recordings"].values(), key=lambda r: (-r["acoustid"], r["id"]))
        base.append(bool(recs) and recs[0]["id"] == f["truth"]["recording_mbid"])
    report["baseline_acoustid_first_listed_ok"] = int(sum(base))

    # ---- Grid search (step 3) on all files ----
    all_idx = np.arange(len(files))
    comp_mask = np.array([f["file"] in comp_files for f in files])
    report["grid"] = dict(GRID)
    report["grid_dur_scales"] = DUR_SCALES
    report["grid_missing"] = [str(m) for m in MISSING]
    finals = {}
    for label, req in (("unconstrained", False), ("with_text", True)):
        best, all_results = run_grid(files, groups, comp_files, all_idx, require_text=req)
        key, dur, missing, c = best
        r, combos = all_results[(dur, missing)]
        ties, total, ties_list = 0, 0, []
        for (d2, m2), (r2, c2) in all_results.items():
            rec, rg, cover, t0, gap = objective(r2, all_idx, comp_mask)
            hit = (rec == key[0]) & (rg == key[1]) & (cover == key[2])
            ties += int(hit.sum())
            total += len(c2)
            for ci in np.where(hit)[0]:
                ties_list.append(
                    (round(float(gap[ci]), 2), d2, str(m2), c2[ci].tolist(), round(float(t0[ci]), 2))
                )
        ties_list.sort(key=lambda x: -x[0])
        report[label] = {
            "weights": dict(zip(CLUES, combos[c].tolist())),
            "dur_scale": dur,
            "missing": str(missing),
            "key": list(key),
            "configs_total": total,
            "configs_tied_rec_rg_coverage": ties,
            "top_tied_by_gap": ties_list[:10],
        }
        finals[label] = (dur, missing, combos[c])
        print(label, report[label]["weights"], dur, missing, key, "ties", ties, "/", total)

    dur, missing, wvec = finals["with_text"]
    W = np.array([wvec])
    ctx_rows = rows_for_context(files, groups, missing, dur)
    single_rows = rows_for_context(files, [[i] for i in range(len(files))], missing, dur)
    r = evaluate_config_arrays(files, groups, ctx_rows, single_rows, W, comp_files)
    r_strict = evaluate_config_arrays(
        files, groups, ctx_rows, single_rows, W, comp_files, lenient=False
    )
    per_file = []
    for i, f in enumerate(files):
        on_g = bool(r["on_group"][i, 0])
        k = r["chosen_g"][i, 0] if on_g else r["chosen_s"][i, 0]
        row = (ctx_rows if on_g else single_rows)[k] if k >= 0 else None
        rec = f["recordings"].get(row["rec"]) if row else None
        rel = next((x for x in rec["releases"] if x["id"] == row["rel"]), None) if rec else None
        clues = None
        if row:
            clues = dict(zip(CLUES, [None if x != x else round(x, 3) for x in row["v"]]))
        per_file.append(
            {
                "file": f["file"],
                "damages": f["damages"],
                "score": round(float(r["score"][i, 0]), 2),
                "best_wrong": round(float(r["wrong_max"][i, 0]), 2),
                "best_wrong_strict": round(float(r_strict["wrong_max"][i, 0]), 2),
                "rec_ok": bool(r["rec_ok"][i, 0]),
                "near_miss": bool(r["near"][i, 0]),
                "rg_ok": bool(r["rg_ok"][i, 0]),
                "rel_ok": bool(r["rel_ok"][i, 0]),
                "on_group_edition": on_g,
                "proposed_recording": row["rec"] if row else None,
                "proposed_recording_title": rec["title"] if rec else None,
                "proposed_release": row["rel"] if row else None,
                "proposed_release_title": rel["title"] if rel else None,
                "proposed_release_date": rel["date"] if rel else None,
                "clues": clues,
            }
        )
    report["per_file"] = per_file

    groups_out = []
    for gi, g in enumerate(groups):
        rel_ids, pick, gs = r["group_rel"].get(gi, ([], None, None))
        chosen = rel_ids[pick[0]] if rel_ids else None
        gt_rgs = {files[i]["gt_rg"] for i in g}
        rel_rg = {row["rel"]: row["rg"] for row in ctx_rows if row["group"] == gi}
        # Best group score of an edition whose release group is not the right one
        # (for a group whose files come from several editions, every edition is wrong).
        wrong = [
            float(gs[ri, 0])
            for ri, rid in enumerate(rel_ids)
            if len(gt_rgs) > 1 or rel_rg[rid] not in gt_rgs
        ]
        groups_out.append(
            {
                "files": [files[i]["file"] for i in g],
                "score": round(float(r["group_score"][gi, 0]), 2),
                "release": chosen,
                "best_wrong_edition_score": round(max(wrong), 2) if wrong else None,
                "all_right": len(gt_rgs) == 1
                and all(
                    per_file[i]["rec_ok"]
                    and per_file[i]["rg_ok"]
                    and per_file[i]["on_group_edition"]
                    for i in g
                ),
                "all_files_on_edition": all(per_file[i]["on_group_edition"] for i in g),
            }
        )
    report["groups_final"] = groups_out

    # Duplicates (design 8.6: same recording and same edition in the same library),
    # from the proposals, compared with is_duplicate_of in the ground truth.
    by_key = {}
    for p in per_file:
        by_key.setdefault((p["proposed_recording"], p["proposed_release"]), []).append(p["file"])
    rg_of = {}
    for row in single_rows:
        rg_of[row["rel"]] = row["rg"]
    by_rg = {}
    for p in per_file:
        by_rg.setdefault((p["proposed_recording"], rg_of.get(p["proposed_release"])), []).append(
            p["file"]
        )
    expected = {t["file"] for t in truth_list if t["is_duplicate_of"]}
    report["duplicates"] = {
        "ground_truth_duplicates": sorted(expected),
        "same_recording_and_edition": sorted(v for v in by_key.values() if len(v) > 1),
        "same_recording_and_album": sorted(v for v in by_rg.values() if len(v) > 1),
    }

    # Thresholds: files (top proposal and best wrong candidate) and groups.
    thr = []
    for t in (50, 60, 70, 75, 80, 85, 90, 95):
        good = [p for p in per_file if p["rec_ok"] and (p["rg_ok"] or p["file"] in comp_files)]
        thr.append(
            {
                "threshold": t,
                "files_wrong_top_at_or_above": sum(
                    1 for p in per_file if p["score"] >= t and p not in good
                ),
                "files_wrong_candidate_at_or_above": sum(
                    1 for p in per_file if p["best_wrong"] >= t
                ),
                "files_wrong_candidate_strict_at_or_above": sum(
                    1 for p in per_file if p["best_wrong_strict"] >= t
                ),
                "files_right_below": sum(1 for p in good if p["score"] < t),
                "groups_wrong_at_or_above": sum(
                    1 for g in groups_out if not g["all_right"] and g["score"] >= t
                ),
                "groups_wrong_edition_candidate_at_or_above": sum(
                    1
                    for g in groups_out
                    if (g["best_wrong_edition_score"] or -1) >= t
                ),
                "groups_right_below": sum(
                    1 for g in groups_out if g["all_right"] and g["score"] < t
                ),
            }
        )
    report["thresholds"] = thr

    # Same weights, titles compared without cleanup.
    ctx_nc = rows_for_context(files, groups, missing, dur, cleanup=False)
    single_nc = rows_for_context(
        files, [[i] for i in range(len(files))], missing, dur, cleanup=False
    )
    r_nc = evaluate_config_arrays(files, groups, ctx_nc, single_nc, W, comp_files)
    report["no_cleanup"] = {
        "rec_ok": int(r_nc["rec_ok"][:, 0].sum()),
        "rg_ok_noncomp": int((r_nc["rg_ok"][:, 0] & ~comp_mask).sum()),
        "scores": {files[i]["file"]: round(float(r_nc["score"][i, 0]), 2) for i in range(len(files))},
        "wrong_max": round(float(r_nc["wrong_max"][:, 0].max()), 2),
    }

    # Reference configs (same duration scale and missing-tag policy as the final one).
    refs = {
        "acoustid_only": [1, 0, 0, 0, 0, 0, 0, 0],
        "acoustid_duration": [1, 1, 0, 0, 0, 0, 0, 0],
        "file_clues_equal": [1, 1, 1, 1, 0, 0, 0, 0],
        "all_equal": [1] * 8,
    }
    report["refs"] = {}
    for name, wl in refs.items():
        rr = evaluate_config_arrays(
            files, groups, ctx_rows, single_rows, np.array([wl], dtype=float), comp_files
        )
        rec, rg, cover, t0, gap = objective(rr, all_idx, comp_mask)
        report["refs"][name] = {
            "weights": wl,
            "rec_ok": int(rec[0]),
            "near_miss": int(rr["near"][:, 0].sum()),
            "rg_ok_noncomp": int(rg[0]),
            "rel_ok_noncomp": int((rr["rel_ok"][:, 0] & ~comp_mask).sum()),
            "coverage": int(cover[0]),
            "t0": round(float(t0[0]), 2),
            "gap": round(float(gap[0]), 2),
        }

    # ---- Leave one edition out: choose weights on 3 editions, test on the 4th ----
    editions = sorted({f["truth"]["release_mbid"] for f in files})
    report["loeo"] = {}
    for label, req in (("unconstrained", False), ("with_text", True)):
        out = []
        for ed in editions:
            train = np.array([i for i, f in enumerate(files) if f["truth"]["release_mbid"] != ed])
            test = np.array([i for i, f in enumerate(files) if f["truth"]["release_mbid"] == ed])
            b, res_all = run_grid(files, groups, comp_files, train, require_text=req)
            k2, d2, m2, c2 = b
            rr, cc = res_all[(d2, m2)]
            t0 = float(objective(rr, train, comp_mask)[3][c2])
            good = rr["rec_ok"][test, c2] & (rr["rg_ok"][test, c2] | comp_mask[test])
            out.append(
                {
                    "held_out_release": ed,
                    "weights": dict(zip(CLUES, cc[c2].tolist())),
                    "dur_scale": d2,
                    "missing": str(m2),
                    "n_test": int(len(test)),
                    "test_rec_ok": int(rr["rec_ok"][test, c2].sum()),
                    "test_rg_ok_noncomp": int((rr["rg_ok"][test, c2] & ~comp_mask[test]).sum()),
                    "test_n_noncomp": int((~comp_mask[test]).sum()),
                    "train_t0": round(t0, 2),
                    "test_wrong_at_or_above_train_t0": int(
                        (rr["wrong_max"][test, c2] >= t0).sum()
                    ),
                    "test_wrong_max": round(float(rr["wrong_max"][test, c2].max()), 2),
                    "test_right_above_train_t0": int(
                        (good & (rr["score"][test, c2] > t0)).sum()
                    ),
                }
            )
            print("LOEO", label, ed[:8], out[-1])
        report["loeo"][label] = out

    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=1, default=lambda o: sorted(o) if isinstance(o, set) else str(o))
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
