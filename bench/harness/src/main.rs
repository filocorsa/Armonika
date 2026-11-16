//! `armonika-bench`: measures the section 3 thresholds against a real `armonika serve`.
//!
//! It starts the server on an empty data directory with the given library, then
//! measures the first scan, a second scan without changes, search latency, the time to
//! the first byte of a raw stream, and the idle memory. Hard thresholds that are missed
//! end the run with exit code 1; the first scan and the memory are only reported.

mod manifest;
mod queries;
mod stats;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::Value;
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::queries::{QueryType, Rng};
use crate::stats::{HardMetrics, Summary};

const ADMIN_USER: &str = "bench";
const ADMIN_PASSWORD: &str = "bench-password";

#[derive(Parser)]
#[command(name = "armonika-bench", version)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Start a server on a fresh data directory and measure the section 3 thresholds.
    Run(RunArgs),
}

#[derive(Parser)]
struct RunArgs {
    /// Library root, as written by `gen-big`.
    #[arg(long)]
    library: PathBuf,
    /// The `manifest.json` of the library.
    #[arg(long)]
    manifest: PathBuf,
    /// The armonika executable.
    #[arg(long)]
    binary: PathBuf,
    /// Search queries to run.
    #[arg(long, default_value_t = 1000)]
    queries: usize,
    /// Tracks whose first stream byte is timed.
    #[arg(long, default_value_t = 200)]
    streams: usize,
    /// Seconds without requests before the memory is read.
    #[arg(long, default_value_t = 60)]
    idle_secs: u64,
    /// Longest wait, in seconds, for the server to stop using CPU (the background
    /// content hash runs for a while after a scan) before search is measured. 0 measures
    /// at once, which shows what a user sees right after adding a library.
    #[arg(long, default_value_t = 3600)]
    quiet_timeout_secs: u64,
    /// Write the results as JSON to this file too.
    #[arg(long)]
    out: Option<PathBuf>,
}

/// Everything measured by one run.
#[derive(Debug, Serialize)]
struct Results {
    tracks_in_manifest: usize,
    first_scan: FirstScan,
    index_settled_after_secs: f64,
    quiet_wait: QuietWait,
    second_scan: SecondScan,
    search: SearchResults,
    first_byte: Summary,
    idle_memory_mb: f64,
    index_size_mb: f64,
    database_size_mb: f64,
    violations: Vec<String>,
}

/// How the wait for a quiet server ended.
#[derive(Debug, Serialize)]
struct QuietWait {
    waited_secs: f64,
    /// True when the server used almost no CPU for 3 s in a row; false when the wait
    /// was skipped or ran out of time (search was then measured while it was busy).
    reached_calm: bool,
    limit_secs: u64,
}

#[derive(Debug, Serialize)]
struct FirstScan {
    /// From the start of the process to the end of the scan.
    wall_secs: f64,
    /// The duration the scanner reports for itself.
    scanner_secs: f64,
    tracks: usize,
    files_read: u64,
}

#[derive(Debug, Serialize)]
struct SecondScan {
    wall_secs: f64,
    scanner_secs: f64,
    files_read: u64,
}

#[derive(Debug, Serialize)]
struct SearchResults {
    queries: usize,
    /// `Server-Timing` of the server, per query type, and for all queries.
    server: BTreeMap<String, Summary>,
    /// Time seen by the client (includes the HTTP round trip), same grouping.
    client: BTreeMap<String, Summary>,
}

fn main() -> ExitCode {
    match run_cli() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("armonika-bench: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run_cli() -> Result<ExitCode> {
    let Action::Run(args) = Cli::parse().command;
    let results = run(&args)?;
    print_table(&results);
    let json = serde_json::to_string_pretty(&results)?;
    if let Some(out) = &args.out {
        std::fs::write(out, &json).with_context(|| format!("cannot write {}", out.display()))?;
    }
    if results.violations.is_empty() {
        println!("\nAll hard thresholds passed.");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("\nHard thresholds missed:");
        for v in &results.violations {
            println!("  - {v}");
        }
        Ok(ExitCode::from(1))
    }
}

// ---------------------------------------------------------------- the server process

/// A running `armonika serve` with the lines it logged.
struct Server {
    child: Child,
    logs: Arc<Mutex<Vec<String>>>,
    base: String,
    started: Instant,
}

impl Server {
    fn start(binary: &Path, library: &Path, work: &Path) -> Result<Server> {
        let port = free_port()?;
        let data_dir = work.join("data");
        let config = work.join("armonika.toml");
        // TOML literal strings keep Windows backslashes as they are.
        std::fs::write(
            &config,
            format!(
                "data_dir = '{}'\n\n[server]\nbind = '127.0.0.1:{port}'\n\n[web]\nenabled = false\n\n\
                 [scan]\nwatch = false\ninterval = '0s'\n\n[[library]]\nname = 'bench'\npath = '{}'\n",
                data_dir.display(),
                library.display()
            ),
        )?;
        let mut child = Command::new(binary)
            .arg("--config")
            .arg(&config)
            .arg("serve")
            .env("ARMONIKA_ADMIN_USER", ADMIN_USER)
            .env("ARMONIKA_ADMIN_PASSWORD", ADMIN_PASSWORD)
            .env("NO_COLOR", "1")
            .env_remove("RUST_LOG")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot start {}", binary.display()))?;
        let started = Instant::now();
        let logs = Arc::new(Mutex::new(Vec::new()));
        let stderr = child.stderr.take().context("no stderr")?;
        let sink = logs.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Ok(mut logs) = sink.lock() {
                    logs.push(line);
                }
            }
        });
        Ok(Server {
            child,
            logs,
            base: format!("http://127.0.0.1:{port}"),
            started,
        })
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The `scan finished` lines logged so far.
    fn scans_finished(&self) -> Vec<String> {
        self.logs
            .lock()
            .map(|l| {
                l.iter()
                    .filter(|line| line.contains("scan finished"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The last log lines, for an error message.
    fn log_tail(&self) -> String {
        self.logs
            .lock()
            .map(|l| l[l.len().saturating_sub(15)..].join("\n"))
            .unwrap_or_default()
    }

    /// Waits for the next `scan finished` line after `already` of them.
    fn wait_scan(&mut self, already: usize, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(line) = self.scans_finished().get(already) {
                return Ok(line.clone());
            }
            if let Some(status) = self.child.try_wait()? {
                bail!("the server exited ({status}):\n{}", self.log_tail());
            }
            if Instant::now() > deadline {
                bail!(
                    "the scan did not finish in {timeout:?}:\n{}",
                    self.log_tail()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

// ---------------------------------------------------------------- the HTTP client

struct Client {
    agent: ureq::Agent,
    base: String,
}

/// A Subsonic answer with the time it took.
struct Answer {
    body: Value,
    /// The `app;dur` of `Server-Timing`, in milliseconds.
    server_ms: Option<f64>,
    client_ms: f64,
}

impl Client {
    fn new(base: &str) -> Client {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(120)))
            .build()
            .into();
        Client {
            agent,
            base: base.to_owned(),
        }
    }

    fn request(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let mut req = self
            .agent
            .get(format!("{}/rest/{method}", self.base))
            .query("u", ADMIN_USER)
            .query("p", ADMIN_PASSWORD)
            .query("v", "1.16.1")
            .query("c", "armonika-bench")
            .query("f", "json");
        for (k, v) in params {
            req = req.query(k, v);
        }
        req
    }

    fn call(&self, method: &str, params: &[(&str, &str)]) -> Result<Answer> {
        let started = Instant::now();
        let mut response = self
            .request(method, params)
            .call()
            .with_context(|| format!("{method} failed"))?;
        let server_ms = response
            .headers()
            .get("server-timing")
            .and_then(|v| v.to_str().ok())
            .and_then(stats::parse_server_timing);
        let body: Value = response.body_mut().read_json()?;
        let client_ms = started.elapsed().as_secs_f64() * 1000.0;
        let reply = &body["subsonic-response"];
        if reply["status"] != "ok" {
            bail!("{method} gave an error: {reply}");
        }
        Ok(Answer {
            body: reply.clone(),
            server_ms,
            client_ms,
        })
    }

    /// Milliseconds to the first byte of the body of a raw `stream`.
    fn first_byte_ms(&self, id: &str) -> Result<f64> {
        let started = Instant::now();
        let mut response = self
            .request("stream", &[("id", id), ("format", "raw")])
            .call()
            .context("stream failed")?;
        if !response.status().is_success() {
            bail!("stream gave HTTP {}", response.status());
        }
        let mut byte = [0u8; 1];
        let n = response.body_mut().as_reader().read(&mut byte)?;
        if n == 0 {
            bail!("stream gave an empty body");
        }
        Ok(started.elapsed().as_secs_f64() * 1000.0)
    }

    fn wait_ready(&self, server: &mut Server) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if self.call("ping", &[]).is_ok() {
                return Ok(());
            }
            if let Some(status) = server.child.try_wait()? {
                bail!("the server exited ({status}):\n{}", server.log_tail());
            }
            if Instant::now() > deadline {
                bail!("the server did not answer in 120 s:\n{}", server.log_tail());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Every track the server knows: `(id, path)`, paged with an empty search.
    fn all_tracks(&self) -> Result<Vec<(String, String)>> {
        let mut all = Vec::new();
        loop {
            let offset = all.len().to_string();
            let answer = self.call(
                "search3",
                &[
                    ("query", ""),
                    ("artistCount", "0"),
                    ("albumCount", "0"),
                    ("songCount", "500"),
                    ("songOffset", &offset),
                ],
            )?;
            let songs = answer.body["searchResult3"]["song"]
                .as_array()
                .context("search3 gave no song list")?;
            if songs.is_empty() {
                return Ok(all);
            }
            for s in songs {
                let id = s["id"].as_str().context("song without id")?;
                let path = s["path"].as_str().context("song without path")?;
                all.push((id.to_owned(), path.to_owned()));
            }
        }
    }
}

// ---------------------------------------------------------------- the measurements

fn run(args: &RunArgs) -> Result<Results> {
    let entries = manifest::load(&args.manifest)?;
    let library = args
        .library
        .canonicalize()
        .with_context(|| format!("no library at {}", args.library.display()))?;
    let binary = args
        .binary
        .canonicalize()
        .with_context(|| format!("no executable at {}", args.binary.display()))?;
    let work = tempfile::tempdir()?;
    eprintln!("starting the server on a fresh data directory");
    let mut server = Server::start(&binary, strip_verbatim(&library), work.path())?;
    let client = Client::new(&server.base);

    // 1. First scan.
    client.wait_ready(&mut server)?;
    let line = server.wait_scan(0, Duration::from_secs(4 * 3600))?;
    let first_wall = server.started.elapsed().as_secs_f64();
    let tracks = client.all_tracks()?;
    let diff = manifest::compare(
        entries.iter().map(|e| e.path.as_str()),
        tracks.iter().map(|(_, p)| p.as_str()),
    );
    if !diff.is_empty() {
        bail!(
            "the scanned tracks differ from the manifest: {} missing (e.g. {:?}), {} unexpected (e.g. {:?})",
            diff.missing_count,
            diff.missing,
            diff.unexpected_count,
            diff.unexpected
        );
    }
    let first_scan = FirstScan {
        wall_secs: first_wall,
        scanner_secs: stats::parse_scan_duration_ms(&line).unwrap_or(0) as f64 / 1000.0,
        tracks: tracks.len(),
        files_read: stats::parse_files_read(&line).unwrap_or(0),
    };
    eprintln!(
        "first scan: {} tracks in {:.1} s",
        first_scan.tracks, first_scan.wall_secs
    );

    // The search index is filled in the background after the scan.
    let settled = wait_index(&client, &entries, &mut Rng::new(queries::SEED))?;
    eprintln!("search index settled {settled:.1} s after the scan");

    // 2. Second scan, no changes.
    let t = Instant::now();
    client.call("startScan", &[])?;
    let line = server.wait_scan(1, Duration::from_secs(4 * 3600))?;
    let second_scan = SecondScan {
        wall_secs: t.elapsed().as_secs_f64(),
        scanner_secs: stats::parse_scan_duration_ms(&line).unwrap_or(0) as f64 / 1000.0,
        files_read: stats::parse_files_read(&line)
            .context("no files_read in the second scan line")?,
    };
    eprintln!(
        "second scan: {:.1} s, {} files read",
        second_scan.wall_secs, second_scan.files_read
    );

    // The index updater and the database may still be busy after the scans: measure
    // only once the server is quiet.
    let quiet = wait_quiet(server.pid(), Duration::from_secs(args.quiet_timeout_secs));
    eprintln!(
        "server quiet wait: {:.1} s, calm reached: {} (limit {} s)",
        quiet.waited_secs, quiet.reached_calm, quiet.limit_secs
    );

    // 3. Search.
    let search = measure_search(&client, &entries, args.queries)?;

    // 4. First byte of a raw stream.
    let first_byte = measure_first_byte(&client, &tracks, args.streams)?;

    // 5. Idle memory.
    eprintln!("idle for {} s", args.idle_secs);
    std::thread::sleep(Duration::from_secs(args.idle_secs));
    let idle_memory_mb = process_memory_mb(server.pid());
    let index_size_mb = dir_size(&work.path().join("data").join("search")) as f64 / 1e6;
    let database_size_mb = dir_size_matching(&work.path().join("data"), "armonika.db") as f64 / 1e6;

    let hard = HardMetrics {
        search_p95_by_type: QueryType::ALL
            .iter()
            .filter_map(|t| {
                search
                    .server
                    .get(t.name())
                    .map(|s| (t.name().to_owned(), s.p95_ms))
            })
            .collect(),
        first_byte_p95_ms: first_byte.p95_ms,
        second_scan_files_read: second_scan.files_read,
    };
    Ok(Results {
        tracks_in_manifest: entries.len(),
        first_scan,
        index_settled_after_secs: settled,
        quiet_wait: quiet,
        second_scan,
        search,
        first_byte,
        idle_memory_mb,
        index_size_mb,
        database_size_mb,
        violations: stats::violations(&hard),
    })
}

/// A Windows `\\?\` canonical path written the plain way, which the TOML file and the
/// server both accept; other paths are returned as they are.
fn strip_verbatim(path: &Path) -> &Path {
    path.to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .map_or(path, Path::new)
}

/// Waits until a sample of tracks is found by searching for their own title; returns
/// the seconds it took.
fn wait_index(client: &Client, entries: &[manifest::Entry], rng: &mut Rng) -> Result<f64> {
    let started = Instant::now();
    // Titles with CJK characters are left out: an index built without its CJK fields
    // (the comparison of docs/benchmarks.md) cannot find them.
    let probes: Vec<&manifest::Entry> = (0..40)
        .map(|_| &entries[rng.below(entries.len())])
        .chain(entries.last())
        .filter(|e| !queries::has_cjk(&e.title))
        .collect();
    if probes.is_empty() {
        return Ok(0.0);
    }
    loop {
        let mut found = 0;
        for p in &probes {
            let answer = client.call(
                "search3",
                &[
                    ("query", p.title.as_str()),
                    ("artistCount", "0"),
                    ("albumCount", "0"),
                    ("songCount", "20"),
                ],
            )?;
            let hit = answer.body["searchResult3"]["song"]
                .as_array()
                .is_some_and(|songs| songs.iter().any(|s| s["title"] == p.title.as_str()));
            found += usize::from(hit);
        }
        if found == probes.len() {
            return Ok(started.elapsed().as_secs_f64());
        }
        if started.elapsed() > Duration::from_secs(3600) {
            bail!(
                "the search index is still incomplete after 1 hour ({found}/{} probes found)",
                probes.len()
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn measure_search(
    client: &Client,
    entries: &[manifest::Entry],
    count: usize,
) -> Result<SearchResults> {
    let queries = queries::build(entries, count, queries::SEED);
    // A short warm-up with other queries, so caches are filled and not measured.
    for q in queries::build(entries, 50, queries::SEED + 1) {
        client.call("search3", &search_params(&q.text))?;
    }
    let mut server: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut client_times: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for q in &queries {
        let answer = client.call("search3", &search_params(&q.text))?;
        let ms = answer
            .server_ms
            .context("the response has no Server-Timing header")?;
        for key in [q.ty.name(), "all"] {
            server.entry(key).or_default().push(ms);
            client_times.entry(key).or_default().push(answer.client_ms);
        }
    }
    let summarize = |m: BTreeMap<&str, Vec<f64>>| -> BTreeMap<String, Summary> {
        m.into_iter()
            .map(|(k, v)| (k.to_owned(), stats::summarize(&v)))
            .collect()
    };
    Ok(SearchResults {
        queries: queries.len(),
        server: summarize(server),
        client: summarize(client_times),
    })
}

fn search_params(text: &str) -> [(&str, &str); 4] {
    [
        ("query", text),
        ("artistCount", "10"),
        ("albumCount", "10"),
        ("songCount", "10"),
    ]
}

fn measure_first_byte(
    client: &Client,
    tracks: &[(String, String)],
    count: usize,
) -> Result<Summary> {
    let mut rng = Rng::new(queries::SEED + 2);
    let mut times = Vec::with_capacity(count);
    for _ in 0..count.min(tracks.len()) {
        let (id, _) = &tracks[rng.below(tracks.len())];
        times.push(client.first_byte_ms(id)?);
    }
    Ok(stats::summarize(&times))
}

/// Waits until the process uses almost no CPU for 3 seconds in a row, at most
/// `timeout`, and says which of the two ended the wait.
fn wait_quiet(pid: u32, timeout: Duration) -> QuietWait {
    let started = Instant::now();
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    let mut calm = 0;
    while started.elapsed() < timeout && calm < 3 {
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        std::thread::sleep(Duration::from_secs(1));
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        let busy = system.process(pid).is_none_or(|p| p.cpu_usage() > 2.0);
        calm = if busy { 0 } else { calm + 1 };
    }
    QuietWait {
        waited_secs: started.elapsed().as_secs_f64(),
        reached_calm: calm >= 3,
        limit_secs: timeout.as_secs(),
    }
}

fn process_memory_mb(pid: u32) -> f64 {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system
        .process(pid)
        .map_or(0.0, |p| p.memory() as f64 / (1024.0 * 1024.0))
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// The size of the files in `dir` whose name starts with `prefix` (database and journals).
fn dir_size_matching(dir: &Path, prefix: &str) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------- the report

fn print_table(r: &Results) {
    println!(
        "Library: {} tracks in the manifest, {} scanned",
        r.tracks_in_manifest, r.first_scan.tracks
    );
    println!(
        "First scan (indicative): {:.1} s wall, {:.1} s by the scanner, {} files read",
        r.first_scan.wall_secs, r.first_scan.scanner_secs, r.first_scan.files_read
    );
    println!(
        "Quiet server wait: {:.0} s, calm reached: {} (limit {} s)",
        r.quiet_wait.waited_secs, r.quiet_wait.reached_calm, r.quiet_wait.limit_secs
    );
    println!(
        "Search index complete {:.1} s after the scan",
        r.index_settled_after_secs
    );
    println!(
        "Second scan (hard: 0 files read): {:.2} s wall, {:.2} s by the scanner, {} files read",
        r.second_scan.wall_secs, r.second_scan.scanner_secs, r.second_scan.files_read
    );
    println!(
        "\nSearch, {} queries (hard: server p95 < 50 ms for every type)",
        r.search.queries
    );
    println!(
        "{:<12} {:>7} {:>12} {:>12} {:>12} {:>12}",
        "type", "n", "srv p50 ms", "srv p95 ms", "cli p50 ms", "cli p95 ms"
    );
    let mut types: Vec<&str> = QueryType::ALL.iter().map(|t| t.name()).collect();
    types.push("all");
    for ty in types {
        if let (Some(s), Some(c)) = (r.search.server.get(ty), r.search.client.get(ty)) {
            println!(
                "{ty:<12} {:>7} {:>12.2} {:>12.2} {:>12.2} {:>12.2}",
                s.count, s.p50_ms, s.p95_ms, c.p50_ms, c.p95_ms
            );
        }
    }
    println!(
        "\nFirst byte of a raw stream (hard: p95 < 200 ms), {} tracks: p50 {:.2} ms, p95 {:.2} ms",
        r.first_byte.count, r.first_byte.p50_ms, r.first_byte.p95_ms
    );
    println!(
        "Idle memory (indicative): {:.1} MB. Search index {:.1} MB, database {:.1} MB on disk",
        r.idle_memory_mb, r.index_size_mb, r.database_size_mb
    );
}
