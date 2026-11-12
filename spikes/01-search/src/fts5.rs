//! SQLite FTS5 variant. Two index files: unicode61 (word/prefix) and trigram (substring, typo).
//! `prefold` = text is folded in Rust before indexing and querying (handles o-slash, l-stroke, ...).
//! Otherwise the tokenizer alone does the diacritics removal.

use crate::common::*;
use rusqlite::{Connection, OpenFlags, params};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

const PRUNE_K: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strategy {
    Unicode,
    Trigram,
    Cascade,
}

pub fn db_path(kind: &str, prefold: bool) -> PathBuf {
    let mode = if prefold { "prefold" } else { "native" };
    data_dir().join(format!("search-fts5{}", suffix())).join(format!("{kind}-{mode}.db"))
}

const GROUPS: [(&str, &[&str]); 3] = [
    ("tracks", &["title", "artist", "album"]),
    ("albums", &["name", "artist"]),
    ("artists", &["name"]),
];

fn tokenizer(kind: &str) -> &'static str {
    match kind {
        "u" => "unicode61 remove_diacritics 2', prefix='2 3 4",
        _ => "trigram remove_diacritics 1",
    }
}

pub fn build(kind: &str, prefold: bool, data: &Data) -> std::time::Duration {
    let path = db_path(kind, prefold);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    let t0 = Instant::now();
    let mut conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;").unwrap();
    for (g, cols) in GROUPS {
        let mut defs: Vec<String> = cols.iter().map(|c| c.to_string()).collect();
        if prefold {
            defs.extend(cols.iter().map(|c| format!("o_{c} UNINDEXED")));
        }
        let tok = tokenizer(kind);
        let sql = if kind == "u" {
            format!("CREATE VIRTUAL TABLE {g} USING fts5({}, tokenize='{tok}')", defs.join(", "))
        } else {
            format!("CREATE VIRTUAL TABLE {g} USING fts5({}, tokenize='{tok}')", defs.join(", "))
        };
        conn.execute(&sql, []).unwrap();
    }
    if kind == "t" {
        for (g, _) in GROUPS {
            conn.execute(&format!("CREATE VIRTUAL TABLE {g}_v USING fts5vocab({g}, 'row')"), []).unwrap();
        }
    }
    let tx = conn.transaction().unwrap();
    for (g, docs) in [("tracks", &data.tracks), ("albums", &data.albums), ("artists", &data.artists)] {
        let cols = GROUPS.iter().find(|x| x.0 == g).unwrap().1;
        let n = cols.len();
        let mut names: Vec<String> = cols.iter().map(|c| c.to_string()).collect();
        if prefold {
            names.extend(cols.iter().map(|c| format!("o_{c}")));
        }
        let ph: Vec<String> = (1..=names.len()).map(|i| format!("?{i}")).collect();
        let mut st = tx
            .prepare(&format!("INSERT INTO {g}({}) VALUES ({})", names.join(","), ph.join(",")))
            .unwrap();
        for d in docs {
            let raw = [&d.a, &d.b, &d.c];
            let mut vals: Vec<String> = raw[..n].iter().map(|s| s.to_string()).collect();
            if prefold {
                let folded: Vec<String> = vals.iter().map(|s| fold(s)).collect();
                vals = folded.into_iter().chain(vals).collect();
            }
            st.execute(rusqlite::params_from_iter(vals.iter())).unwrap();
        }
    }
    tx.commit().unwrap();
    for (g, _) in GROUPS {
        conn.execute(&format!("INSERT INTO {g}({g}) VALUES('optimize')"), []).unwrap();
    }
    conn.execute_batch("VACUUM").unwrap();
    t0.elapsed()
}

pub struct Fts5 {
    u: Option<Connection>,
    t: Option<Connection>,
    prefold: bool,
    strategy: Strategy,
    prune: bool,
}

fn open(kind: &str, prefold: bool) -> Connection {
    let c = Connection::open_with_flags(db_path(kind, prefold), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open fts5 db (run build first)");
    c
}

impl Fts5 {
    pub fn open(strategy: Strategy, prefold: bool, prune: bool) -> Self {
        let u = matches!(strategy, Strategy::Unicode | Strategy::Cascade).then(|| open("u", prefold));
        let t = matches!(strategy, Strategy::Trigram | Strategy::Cascade).then(|| open("t", prefold));
        Fts5 { u, t, prefold, strategy, prune }
    }

    fn query_tokens(&self, q: &str) -> Vec<String> {
        if self.prefold { tokens(q) } else { split_tokens(&q.to_lowercase()) }
    }

    fn unicode_expr(&self, q: &str) -> Option<String> {
        let toks = self.query_tokens(q);
        if toks.is_empty() {
            return None;
        }
        let n = toks.len();
        Some(
            toks.iter()
                .enumerate()
                .map(|(i, t)| if i + 1 == n { format!("\"{t}\"*") } else { format!("\"{t}\"") })
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    /// OR of the query trigrams. With `prune`, only the PRUNE_K rarest trigrams that exist in the
    /// index are kept (document frequency read from an fts5vocab table), which bounds the
    /// posting lists scanned for common trigrams.
    fn trigram_expr(&self, conn: &Connection, g: &str, q: &str) -> Option<String> {
        let mut seen = HashSet::new();
        let mut tris: Vec<String> = Vec::new();
        for t in self.query_tokens(q) {
            let ch: Vec<char> = t.chars().collect();
            for w in ch.windows(3) {
                let tri: String = w.iter().collect();
                if seen.insert(tri.clone()) {
                    tris.push(tri);
                }
            }
        }
        if self.prune {
            let mut st = conn.prepare_cached(&format!("SELECT doc FROM {g}_v WHERE term = ?1")).unwrap();
            let mut with_df: Vec<(i64, String)> = tris
                .into_iter()
                .filter_map(|t| {
                    let df: Option<i64> = st.query_row(params![t], |r| r.get(0)).ok();
                    df.filter(|d| *d > 0).map(|d| (d, t))
                })
                .collect();
            with_df.sort();
            with_df.truncate(PRUNE_K);
            tris = with_df.into_iter().map(|x| x.1).collect();
        }
        let parts: Vec<String> = tris.iter().map(|t| format!("\"{t}\"")).collect();
        (!parts.is_empty()).then(|| parts.join(" OR "))
    }

    fn run(&self, conn: &Connection, g: &str, cols: &[&str], expr: &str) -> Vec<Doc> {
        let sel: Vec<String> =
            cols.iter().map(|c| if self.prefold { format!("o_{c}") } else { c.to_string() }).collect();
        let weights = if cols.len() == 3 { "3.0, 1.0, 1.0" } else if cols.len() == 2 { "3.0, 1.0" } else { "1.0" };
        let sql = format!(
            "SELECT {} FROM {g} WHERE {g} MATCH ?1 ORDER BY bm25({g}, {weights}) LIMIT {CAND}",
            sel.join(",")
        );
        let mut st = conn.prepare_cached(&sql).unwrap();
        let rows = st
            .query_map(params![expr], |r| {
                let mut v = vec![String::new(); 3];
                for i in 0..cols.len() {
                    v[i] = r.get::<_, String>(i)?;
                }
                Ok(Doc { a: v[0].clone(), b: v[1].clone(), c: v[2].clone() })
            })
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    fn group(&self, g: &str, cols: &[&str], q: &str) -> Vec<Doc> {
        let from_u = |s: &Self| match (&s.u, s.unicode_expr(q)) {
            (Some(c), Some(e)) => s.run(c, g, cols, &e),
            _ => Vec::new(),
        };
        let from_t = |s: &Self| match &s.t {
            Some(c) => match s.trigram_expr(c, g, q) {
                Some(e) => s.run(c, g, cols, &e),
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        match self.strategy {
            Strategy::Unicode => from_u(self),
            Strategy::Trigram => from_t(self),
            Strategy::Cascade => {
                let mut out = from_u(self);
                if out.len() < TOP {
                    for d in from_t(self) {
                        if out.len() >= CAND {
                            break;
                        }
                        if !out.contains(&d) {
                            out.push(d);
                        }
                    }
                }
                out
            }
        }
    }
}

impl SearchIndex for Fts5 {
    fn candidates(&self, q: &str) -> Groups {
        Groups {
            tracks: self.group("tracks", GROUPS[0].1, q),
            albums: self.group("albums", GROUPS[1].1, q),
            artists: self.group("artists", GROUPS[2].1, q),
        }
    }
}
