//! Spike 1: SQLite FTS5 versus Tantivy for search-as-you-type. Throwaway code.
//!
//! Usage:
//!   spike-search all                  build queries and indexes, bench everything, print report
//!   spike-search queries              generate bench/data/search-queries.json
//!   spike-search build <fts5|tantivy> [prefold|native]
//!   spike-search bench <engine>       engine: tantivy | fts5-{unicode,trigram,cascade} | fts5n-{...}
//!   spike-search report               print markdown tables from stored results

mod common;
mod fts5;
mod queries;
mod tant;

use common::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

const ENGINES: [&str; 9] = [
    "fts5f-trigram",
    "fts5f-cascade",
    "fts5-unicode",
    "fts5-trigram",
    "fts5-cascade",
    "fts5n-unicode",
    "fts5n-trigram",
    "fts5n-cascade",
    "tantivy",
];

fn results_dir() -> PathBuf {
    let d = data_dir().join("search-results");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Working set of this process (resident memory); on Windows this is the process working set.
fn rss_mib() -> f64 {
    let mut sys = System::new();
    let pid = Pid::from_u32(std::process::id());
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::everything());
    sys.process(pid).map(|p| p.memory() as f64 / 1048576.0).unwrap_or(f64::NAN)
}

fn dir_size(p: &std::path::Path) -> u64 {
    if p.is_file() {
        return std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    }
    std::fs::read_dir(p).map(|r| r.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Stat {
    n: usize,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    recall10: f64,
    native10: f64,
    cand50: f64,
}

#[derive(Serialize, Deserialize, Default)]
struct BenchResult {
    engine: String,
    size_mib: f64,
    build_secs: f64,
    rss_open_mib: f64,
    rss_after_mib: f64,
    stats: BTreeMap<String, Stat>,
}

fn hit(q: &queries::Query, docs: &[Doc]) -> bool {
    docs.iter().any(|d| {
        d.a == q.name
            && match (&q.artist, q.kind.as_str()) {
                (Some(a), "track") | (Some(a), "album") => &d.b == a,
                _ => true,
            }
    })
}

/// Nearest-rank percentile.
fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

struct Acc {
    lat: Vec<f64>,
    hit: usize,
    native: usize,
    cand: usize,
}

fn buckets(q: &queries::Query) -> Vec<String> {
    let mut v = vec![q.ty.clone(), "all".into()];
    if q.ty == "prefix" {
        v.push(if q.full {
            "prefix:full".into()
        } else if q.chars <= 4 {
            "prefix:3-4".into()
        } else if q.chars <= 8 {
            "prefix:5-8".into()
        } else {
            "prefix:9+".into()
        });
    }
    v
}

fn bench(engine: &str) {
    let build: BTreeMap<String, f64> = std::fs::read_to_string(results_dir().join("build.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let prefold = !engine.starts_with("fts5n");
    // fts5f / fts5n use all trigrams; fts5- prunes to the rarest ones
    let prune = engine.starts_with("fts5-");
    let (idx, size, build_secs): (Box<dyn SearchIndex>, u64, f64) = if engine == "tantivy" {
        let size = dir_size(&data_dir().join("search-tantivy"));
        (Box::new(tant::Tant::open()), size, *build.get("tantivy").unwrap_or(&f64::NAN))
    } else {
        let strat = match engine.split('-').nth(1).unwrap() {
            "unicode" => fts5::Strategy::Unicode,
            "trigram" => fts5::Strategy::Trigram,
            _ => fts5::Strategy::Cascade,
        };
        let mode = if prefold { "prefold" } else { "native" };
        let mut size = 0;
        let mut secs = 0.0;
        for k in ["u", "t"] {
            let need = match strat {
                fts5::Strategy::Unicode => k == "u",
                fts5::Strategy::Trigram => k == "t",
                fts5::Strategy::Cascade => true,
            };
            if need {
                size += dir_size(&fts5::db_path(k, prefold));
                secs += build.get(&format!("fts5-{k}-{mode}")).copied().unwrap_or(f64::NAN);
            }
        }
        (Box::new(fts5::Fts5::open(strat, prefold, prune)), size, secs)
    };
    let rss_open = rss_mib();
    let qs = queries::load();
    // warm-up (excluded from the numbers): every 15th query
    for q in qs.queries.iter().step_by(15) {
        let _ = search(idx.as_ref(), &q.text);
    }
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();
    for q in &qs.queries {
        let t = Instant::now();
        let cands = idx.candidates(&q.text);
        let res = rerank(&q.text, &cands);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let pick = |g: &Groups| match q.kind.as_str() {
            "track" => g.tracks.clone(),
            "album" => g.albums.clone(),
            _ => g.artists.clone(),
        };
        let final_ = pick(&res);
        let c = pick(&cands);
        let h = hit(q, &final_);
        let nat = hit(q, &c[..c.len().min(TOP)]);
        let cand = hit(q, &c);
        for b in buckets(q) {
            let a = accs.entry(b).or_insert(Acc { lat: vec![], hit: 0, native: 0, cand: 0 });
            a.lat.push(ms);
            a.hit += h as usize;
            a.native += nat as usize;
            a.cand += cand as usize;
        }
    }
    let rss_after = rss_mib();
    let mut stats = BTreeMap::new();
    for (k, mut a) in accs {
        a.lat.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let n = a.lat.len();
        stats.insert(
            k,
            Stat {
                n,
                p50_ms: pct(&a.lat, 0.5),
                p95_ms: pct(&a.lat, 0.95),
                max_ms: *a.lat.last().unwrap(),
                recall10: a.hit as f64 / n as f64,
                native10: a.native as f64 / n as f64,
                cand50: a.cand as f64 / n as f64,
            },
        );
    }
    let r = BenchResult {
        engine: engine.into(),
        size_mib: size as f64 / 1048576.0,
        build_secs,
        rss_open_mib: rss_open,
        rss_after_mib: rss_after,
        stats,
    };
    std::fs::write(results_dir().join(format!("{engine}.json")), serde_json::to_string_pretty(&r).unwrap())
        .unwrap();
    println!("bench {engine} done");
}

fn record_build(key: &str, secs: f64) {
    let p = results_dir().join("build.json");
    let mut m: BTreeMap<String, f64> =
        std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    m.insert(key.into(), secs);
    std::fs::write(p, serde_json::to_string_pretty(&m).unwrap()).unwrap();
}

fn report() {
    let types = ["prefix", "typo", "diacritics", "multiword"];
    let rs: Vec<BenchResult> = ENGINES
        .iter()
        .filter_map(|e| std::fs::read_to_string(results_dir().join(format!("{e}.json"))).ok())
        .map(|s| serde_json::from_str(&s).unwrap())
        .collect();
    println!("| engine | type | n | p50 ms | p95 ms | max ms | recall@10 | native@10 | cand@50 |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for r in &rs {
        for t in types
            .iter()
            .map(|s| s.to_string())
            .chain(["prefix:3-4", "prefix:5-8", "prefix:9+", "prefix:full", "all"].map(String::from))
        {
            if let Some(s) = r.stats.get(&t) {
                println!(
                    "| {} | {} | {} | {:.3} | {:.3} | {:.2} | {:.3} | {:.3} | {:.3} |",
                    r.engine, t, s.n, s.p50_ms, s.p95_ms, s.max_ms, s.recall10, s.native10, s.cand50
                );
            }
        }
    }
    println!();
    println!("| engine | build s | size MiB | RSS open MiB | RSS after MiB |");
    println!("|---|---|---|---|---|");
    for r in &rs {
        println!(
            "| {} | {:.1} | {:.1} | {:.1} | {:.1} |",
            r.engine, r.build_secs, r.size_mib, r.rss_open_mib, r.rss_after_mib
        );
    }
}

fn run_self(args: &[&str]) {
    let exe = std::env::current_exe().unwrap();
    let st = std::process::Command::new(exe).args(args).status().unwrap();
    assert!(st.success(), "child {args:?} failed");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["queries"] => {
            let data = load_data();
            let qs = queries::build(&data);
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for q in &qs.queries {
                *counts.entry(q.ty.as_str()).or_default() += 1;
            }
            std::fs::create_dir_all(data_dir()).unwrap();
            std::fs::write(queries::path(), serde_json::to_string(&qs).unwrap()).unwrap();
            println!("seed {} targets {} queries {:?}", qs.seed, qs.targets, counts);
        }
        ["build", "fts5", rest @ ..] => {
            let data = load_data();
            let modes: Vec<bool> = match rest {
                ["prefold"] => vec![true],
                ["native"] => vec![false],
                _ => vec![true, false],
            };
            for prefold in modes {
                let mode = if prefold { "prefold" } else { "native" };
                for k in ["u", "t"] {
                    let d = fts5::build(k, prefold, &data);
                    println!("fts5 {k} {mode}: {:.1}s", d.as_secs_f64());
                    record_build(&format!("fts5-{k}-{mode}"), d.as_secs_f64());
                }
            }
        }
        ["build", "tantivy"] => {
            let data = load_data();
            let d = tant::build(&data);
            println!("tantivy: {:.1}s", d.as_secs_f64());
            record_build("tantivy", d.as_secs_f64());
        }
        ["bench", e] => bench(e),
        ["report"] => report(),
        ["all"] => {
            run_self(&["queries"]);
            run_self(&["build", "fts5"]);
            run_self(&["build", "tantivy"]);
            for e in ENGINES {
                run_self(&["bench", e]);
            }
            run_self(&["report"]);
        }
        _ => eprintln!("see the header of src/main.rs"),
    }
}
