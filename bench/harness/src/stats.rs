//! Pure logic: percentiles, parsing of what the server reports, and the thresholds.

use serde::Serialize;

/// Hard thresholds of section 3.
pub const SEARCH_P95_LIMIT_MS: f64 = 50.0;
pub const FIRST_BYTE_P95_LIMIT_MS: f64 = 200.0;

/// Nearest-rank percentile of `values` (any order). `p` is in 0..=100.
/// An empty slice gives 0.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The milliseconds of `app;dur=<ms>` in a `Server-Timing` header value.
pub fn parse_server_timing(header: &str) -> Option<f64> {
    header.split(',').find_map(|metric| {
        let mut parts = metric.trim().split(';');
        if parts.next()?.trim() != "app" {
            return None;
        }
        parts.find_map(|p| p.trim().strip_prefix("dur=")?.parse().ok())
    })
}

/// The `files_read` field of a `scan finished` log line.
pub fn parse_files_read(line: &str) -> Option<u64> {
    log_field(line, "files_read")
}

/// The `duration_ms` field of a `scan finished` log line.
pub fn parse_scan_duration_ms(line: &str) -> Option<u64> {
    log_field(line, "duration_ms")
}

/// The unsigned integer after `name=` in a log line. Terminal colour codes around the
/// name and the equals sign are skipped; the name must start a word.
fn log_field(line: &str, name: &str) -> Option<u64> {
    let clean = strip_ansi(line);
    let rest = clean.match_indices(name).find_map(|(i, _)| {
        let starts_word = clean[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let rest = clean[i + name.len()..].strip_prefix('=')?;
        starts_word.then_some(rest)
    })?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `text` without ANSI escape sequences (ESC, `[`, parameters, a letter).
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What is measured for one group of timings.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
}

pub fn summarize(values: &[f64]) -> Summary {
    Summary {
        count: values.len(),
        p50_ms: percentile(values, 50.0),
        p95_ms: percentile(values, 95.0),
    }
}

/// The numbers the hard thresholds look at.
#[derive(Debug, Clone)]
pub struct HardMetrics {
    /// Server-side p95 of each query type, `(type name, p95 ms)`.
    pub search_p95_by_type: Vec<(String, f64)>,
    pub first_byte_p95_ms: f64,
    pub second_scan_files_read: u64,
}

/// The hard thresholds that are missed, as readable lines. Empty means all passed.
/// The first scan time and the idle RAM are never checked here: they are indicative.
pub fn violations(m: &HardMetrics) -> Vec<String> {
    let mut out = Vec::new();
    for (ty, p95) in &m.search_p95_by_type {
        if *p95 >= SEARCH_P95_LIMIT_MS {
            out.push(format!(
                "search p95 for {ty} queries is {p95:.1} ms (limit {SEARCH_P95_LIMIT_MS} ms)"
            ));
        }
    }
    if m.first_byte_p95_ms >= FIRST_BYTE_P95_LIMIT_MS {
        out.push(format!(
            "first byte p95 is {:.1} ms (limit {FIRST_BYTE_P95_LIMIT_MS} ms)",
            m.first_byte_p95_ms
        ));
    }
    if m.second_scan_files_read != 0 {
        out.push(format!(
            "the second scan read {} files (must be 0)",
            m.second_scan_files_read
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 50.0), 50.0);
        assert_eq!(percentile(&v, 95.0), 95.0);
        assert_eq!(percentile(&v, 100.0), 100.0);
        assert_eq!(percentile(&v, 0.0), 1.0);
    }

    #[test]
    fn percentile_ignores_input_order_and_handles_small_sets() {
        assert_eq!(percentile(&[9.0, 1.0, 5.0], 50.0), 5.0);
        assert_eq!(percentile(&[7.0], 95.0), 7.0);
        assert_eq!(percentile(&[], 95.0), 0.0);
        // 20 values: p95 is the 19th.
        let v: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(percentile(&v, 95.0), 19.0);
    }

    #[test]
    fn server_timing_is_parsed() {
        assert_eq!(parse_server_timing("app;dur=12.3"), Some(12.3));
        assert_eq!(parse_server_timing("cache;desc=x, app;dur=0.4"), Some(0.4));
        assert_eq!(parse_server_timing("app;dur=7"), Some(7.0));
        assert_eq!(parse_server_timing("db;dur=1"), None);
        assert_eq!(parse_server_timing("app;dur=abc"), None);
    }

    #[test]
    fn scan_line_fields_are_parsed() {
        let line = "2026-11-15T10:00:00Z  INFO armonika_scanner::reconcile: scan finished \
                    library=lib seen=5000 new=0 unchanged=5000 files_read=0 duration_ms=812";
        assert_eq!(parse_files_read(line), Some(0));
        assert_eq!(parse_scan_duration_ms(line), Some(812));
        let colored = "scan finished \u{1b}[3mfiles_read\u{1b}[0m\u{1b}[2m=\u{1b}[0m17 x";
        assert_eq!(parse_files_read(colored), Some(17));
        assert_eq!(parse_files_read("scan finished"), None);
        // A field that only ends in the name must not match.
        assert_eq!(parse_files_read("scan finished prefiles_read=3"), None);
    }

    #[test]
    fn summary_has_count_and_percentiles() {
        let s = summarize(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(s.count, 4);
        assert_eq!(s.p50_ms, 2.0);
        assert_eq!(s.p95_ms, 4.0);
    }

    fn passing() -> HardMetrics {
        HardMetrics {
            search_p95_by_type: vec![("prefix".into(), 3.0), ("typo".into(), 49.9)],
            first_byte_p95_ms: 199.9,
            second_scan_files_read: 0,
        }
    }

    #[test]
    fn thresholds_just_inside_pass() {
        assert!(violations(&passing()).is_empty());
    }

    #[test]
    fn thresholds_are_strict_at_the_limit() {
        let mut m = passing();
        m.search_p95_by_type[1].1 = 50.0;
        let v = violations(&m);
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("typo"));

        let mut m = passing();
        m.first_byte_p95_ms = 200.0;
        assert_eq!(violations(&m).len(), 1);

        let mut m = passing();
        m.second_scan_files_read = 1;
        assert_eq!(violations(&m).len(), 1);
    }

    #[test]
    fn every_missed_threshold_is_listed() {
        let m = HardMetrics {
            search_p95_by_type: vec![("prefix".into(), 80.0), ("typo".into(), 90.0)],
            first_byte_p95_ms: 500.0,
            second_scan_files_read: 12,
        };
        assert_eq!(violations(&m).len(), 4);
    }
}
