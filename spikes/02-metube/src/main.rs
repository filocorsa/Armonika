//! Spike 2: minimal MeTube client.
//!
//! Usage:
//!   spike-metube run <urls.txt> <metube_base_url> <downloads_dir> <fetch_dir>
//!       add every URL (one per line, "#" starts a comment, text after "|" is
//!       ignored), follow the state until finished or error, fetch the file
//!       over HTTP, check it with ffprobe and compare its size with the file
//!       on disk, then remove the item (and its file) from MeTube so the next
//!       run starts clean. One JSON report per line on stdout.
//!   spike-metube add <metube_base_url> <url>
//!       add one URL and print the raw /add answer and the resulting item.
//!   spike-metube watch <metube_base_url> <url> <seconds>
//!       print the state of the item for <url> every second.
//!   spike-metube fetch <metube_base_url> <downloads_dir> <fetch_dir>
//!       fetch every finished item of /history and verify it.
//!   spike-metube names <metube_base_url> <list.tsv> <fetch_dir>
//!       fetch files by name ("bytes<TAB>name" lines) to test difficult names.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap()
}

/// POST /add: audio only, format "auto" (best audio stream as delivered, no
/// conversion), quality "best", cover embedded when the container allows it.
fn add(c: &reqwest::blocking::Client, base: &str, url: &str) -> Result<(u16, Value), String> {
    let body = json!({
        "url": url,
        "download_type": "audio",
        "format": "auto",
        "quality": "best",
        "audio_tags": "with_cover",
    });
    let r = c
        .post(format!("{base}/add"))
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?;
    let status = r.status().as_u16();
    let text = r.text().map_err(|e| e.to_string())?;
    let v = serde_json::from_str(&text).unwrap_or(Value::String(text));
    Ok((status, v))
}

fn history(c: &reqwest::blocking::Client, base: &str) -> Result<Value, String> {
    c.get(format!("{base}/history"))
        .send()
        .and_then(|r| r.json())
        .map_err(|e| e.to_string())
}

/// Finds the item for `url` in done, queue or pending. Returns (list, item).
fn find_item(h: &Value, url: &str) -> Option<(&'static str, Value)> {
    for list in ["done", "queue", "pending"] {
        if let Some(a) = h[list].as_array() {
            for it in a {
                if it["url"].as_str() == Some(url) {
                    return Some((list, it.clone()));
                }
            }
        }
    }
    None
}

/// Equivalent of the MeTube UI's `encodeURIComponent` (unreserved chars kept).
fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'('
            | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn audio_link(base: &str, folder: &str, filename: &str) -> String {
    let mut link = format!("{base}/audio_download/");
    for part in folder.split('/').filter(|p| !p.is_empty()) {
        link.push_str(&encode_component(part));
        link.push('/');
    }
    link.push_str(&encode_component(filename));
    link
}

fn ffprobe(path: &Path) -> Result<Value, String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

/// Fetch the finished file over HTTP, check it, return a JSON report.
fn fetch_and_verify(
    c: &reqwest::blocking::Client,
    base: &str,
    item: &Value,
    downloads: &Path,
    fetch_dir: &Path,
) -> Value {
    let filename = item["filename"].as_str().unwrap_or_default();
    let folder = item["folder"].as_str().unwrap_or_default();
    let link = audio_link(base, folder, filename);

    let mut rep = json!({ "filename": filename, "http_url": link });
    let resp = match c.get(&link).send() {
        Ok(r) => r,
        Err(e) => {
            rep["error"] = json!(format!("http: {e}"));
            return rep;
        }
    };
    rep["http_status"] = json!(resp.status().as_u16());
    rep["content_type"] = json!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
    );
    if !resp.status().is_success() {
        return rep;
    }
    let bytes = resp.bytes().unwrap_or_default();
    rep["http_bytes"] = json!(bytes.len());

    let disk: PathBuf = downloads.join(folder).join(filename);
    match std::fs::metadata(&disk) {
        Ok(m) => {
            rep["disk_bytes"] = json!(m.len());
            rep["size_match"] = json!(m.len() as usize == bytes.len());
            if let Ok(d) = std::fs::read(&disk) {
                rep["content_identical"] = json!(d == bytes.as_ref());
            }
        }
        Err(e) => rep["disk_error"] = json!(e.to_string()),
    }
    rep["metube_size"] = item["size"].clone();

    // Store under a neutral name: the point here is the bytes, not the name.
    let safe = format!("{}.bin", item["id"].as_str().unwrap_or("item"));
    let local = fetch_dir.join(&safe);
    std::fs::create_dir_all(fetch_dir).unwrap();
    std::fs::write(&local, &bytes).unwrap();
    match ffprobe(&local) {
        Ok(p) => {
            rep["ffprobe_ok"] = json!(true);
            rep["container"] = p["format"]["format_name"].clone();
            rep["duration"] = p["format"]["duration"].clone();
            rep["bit_rate"] = p["format"]["bit_rate"].clone();
            let streams = p["streams"].as_array().cloned().unwrap_or_default();
            if let Some(a) = streams.iter().find(|s| s["codec_type"] == "audio") {
                rep["codec"] = a["codec_name"].clone();
                rep["sample_rate"] = a["sample_rate"].clone();
                rep["channels"] = a["channels"].clone();
            }
            rep["has_cover"] = json!(streams.iter().any(|s| s["codec_type"] == "video"));
            rep["tags"] = p["format"]["tags"].clone();
        }
        Err(e) => {
            rep["ffprobe_ok"] = json!(false);
            rep["ffprobe_error"] = json!(e);
        }
    }
    rep
}

/// Follow one URL until it is finished or failed. Returns the final item.
fn follow(c: &reqwest::blocking::Client, base: &str, url: &str, limit: Duration) -> Option<Value> {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(500));
        let Ok(h) = history(c, base) else { continue };
        match find_item(&h, url) {
            Some(("done", it)) => return Some(it),
            Some((list, it)) => {
                let s = format!(
                    "{list}/{} {}%",
                    it["status"].as_str().unwrap_or("?"),
                    it["percent"]
                );
                if s != last {
                    eprintln!("  {s}");
                    last = s;
                }
            }
            None => {}
        }
    }
    None
}

/// POST /delete for a finished item. "ids" are the item URLs, not the video id.
fn remove_done(c: &reqwest::blocking::Client, base: &str, url: &str, delete_files: bool) {
    let _ = c
        .post(format!("{base}/delete"))
        .json(&json!({"ids": [url], "where": "done", "delete_files": delete_files}))
        .send();
}

fn run(urls_file: &str, base: &str, downloads: &Path, fetch_dir: &Path) {
    let c = client();
    let list = std::fs::read_to_string(urls_file).unwrap();
    for line in list.lines() {
        let url = line.split('|').next().unwrap().trim();
        if url.is_empty() || url.starts_with('#') {
            continue;
        }
        eprintln!("{url}");
        // Items of /history are keyed by URL: a finished item left over from
        // an earlier run would be mistaken for the new one, so drop it first.
        remove_done(&c, base, url, false);
        let t0 = Instant::now();
        let (code, answer) = match add(&c, base, url) {
            Ok(x) => x,
            Err(e) => {
                println!("{}", json!({"url": url, "stage": "add", "error": e}));
                continue;
            }
        };
        let mut rep = json!({ "url": url, "add_http": code, "add_answer": answer });
        if let Some(it) = follow(&c, base, url, Duration::from_secs(300)) {
            rep["status"] = it["status"].clone();
            rep["msg"] = it["msg"].clone();
            rep["error"] = it["error"].clone();
            rep["seconds"] = json!(t0.elapsed().as_secs_f32());
            if it["status"] == "finished" {
                rep["fetch"] = fetch_and_verify(&c, base, &it, downloads, fetch_dir);
            }
            // Remove the item and its file so a second run downloads again.
            remove_done(&c, base, url, true);
        } else {
            rep["status"] = json!("timeout");
        }
        println!("{rep}");
    }
}

/// `list` has one "bytes<TAB>name" line per file served by the instance.
/// Fetch each by name, compare the size and let ffprobe read the bytes.
fn names(base: &str, list: &Path, fetch_dir: &Path) {
    let c = client();
    std::fs::create_dir_all(fetch_dir).unwrap();
    let text = std::fs::read_to_string(list).unwrap();
    for (i, line) in text.lines().enumerate() {
        let Some((size, name)) = line.split_once('\t') else { continue };
        let link = audio_link(base, "", name);
        let mut rep = json!({"name": name, "url": link, "expected_bytes": size});
        match c.get(&link).send() {
            Ok(r) => {
                rep["http"] = json!(r.status().as_u16());
                let b = r.bytes().unwrap_or_default();
                rep["http_bytes"] = json!(b.len());
                rep["size_match"] = json!(b.len().to_string() == size);
                let local = fetch_dir.join(format!("{i}.bin"));
                std::fs::write(&local, &b).unwrap();
                rep["ffprobe_ok"] = json!(ffprobe(&local).is_ok());
            }
            Err(e) => rep["error"] = json!(e.to_string()),
        }
        println!("{rep}");
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let c = client();
    match a.get(1).map(String::as_str) {
        Some("run") if a.len() == 6 => run(&a[2], &a[3], Path::new(&a[4]), Path::new(&a[5])),
        Some("add") if a.len() == 4 => {
            let (code, v) = add(&c, &a[2], &a[3]).unwrap();
            println!("add: http {code} {v}");
            std::thread::sleep(Duration::from_secs(3));
            let h = history(&c, &a[2]).unwrap();
            println!("item: {:?}", find_item(&h, &a[3]));
        }
        Some("watch") if a.len() == 5 => {
            let secs: u64 = a[4].parse().unwrap();
            for _ in 0..secs {
                match history(&c, &a[2]) {
                    Ok(h) => match find_item(&h, &a[3]) {
                        Some((l, it)) => println!(
                            "{l}: status={} percent={} msg={} error={}",
                            it["status"], it["percent"], it["msg"], it["error"]
                        ),
                        None => println!("not in history"),
                    },
                    Err(e) => println!("history failed: {e}"),
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        Some("fetch") if a.len() == 5 => {
            let h = history(&c, &a[2]).unwrap();
            for it in h["done"].as_array().unwrap() {
                if it["status"] == "finished" {
                    println!(
                        "{}",
                        fetch_and_verify(&c, &a[2], it, Path::new(&a[3]), Path::new(&a[4]))
                    );
                }
            }
        }
        Some("names") if a.len() == 5 => names(&a[2], Path::new(&a[3]), Path::new(&a[4])),
        _ => eprintln!("see the doc comment at the top of src/main.rs"),
    }
}
