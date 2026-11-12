//! Tantivy variant: lowercase + ASCII folding, fuzzy (distance 1) and fuzzy-prefix queries.

use crate::common::*;
use std::path::PathBuf;
use std::time::Instant;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, FuzzyTermQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value, STORED,
};
use tantivy::tokenizer::{AsciiFoldingFilter, LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, IndexReader, ReloadPolicy, TantivyDocument, Term};

const GROUPS: [(&str, &[&str]); 3] = [
    ("tracks", &["title", "artist", "album"]),
    ("albums", &["name", "artist"]),
    ("artists", &["name"]),
];

pub fn root() -> PathBuf {
    data_dir().join(format!("search-tantivy{}", suffix()))
}

pub fn dir(g: &str) -> PathBuf {
    root().join(g)
}

fn analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default()).filter(LowerCaser).filter(AsciiFoldingFilter).build()
}

fn schema(cols: &[&str]) -> Schema {
    let mut b = Schema::builder();
    let opts = TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default().set_tokenizer("fold").set_index_option(IndexRecordOption::WithFreqs),
    );
    for c in cols {
        b.add_text_field(c, opts.clone());
    }
    let _ = STORED;
    b.build()
}

pub fn build(data: &Data) -> std::time::Duration {
    let t0 = Instant::now();
    for (g, docs) in [("tracks", &data.tracks), ("albums", &data.albums), ("artists", &data.artists)] {
        let cols = GROUPS.iter().find(|x| x.0 == g).unwrap().1;
        let d = dir(g);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let index = Index::create_in_dir(&d, schema(cols)).unwrap();
        index.tokenizers().register("fold", analyzer());
        let fields: Vec<Field> = cols.iter().map(|c| index.schema().get_field(c).unwrap()).collect();
        let mut w = index.writer(200_000_000).unwrap();
        for doc in docs {
            let raw = [&doc.a, &doc.b, &doc.c];
            let mut td = TantivyDocument::default();
            for (i, f) in fields.iter().enumerate() {
                td.add_text(*f, raw[i]);
            }
            w.add_document(td).unwrap();
        }
        w.commit().unwrap();
        w.wait_merging_threads().unwrap();
        let ids = index.searchable_segment_ids().unwrap();
        if ids.len() > 1 {
            let mut w2: tantivy::IndexWriter = index.writer(50_000_000).unwrap();
            w2.merge(&ids).wait().unwrap();
            w2.wait_merging_threads().unwrap();
        }
        eprintln!("tantivy {g}: {} segment(s)", index.searchable_segment_ids().unwrap().len());
    }
    t0.elapsed()
}

struct GroupIdx {
    index: Index,
    reader: IndexReader,
    fields: Vec<Field>,
}

pub struct Tant {
    groups: Vec<GroupIdx>,
    analyzer: TextAnalyzer,
}

impl Tant {
    pub fn open() -> Self {
        let groups = GROUPS
            .iter()
            .map(|(g, cols)| {
                let index = Index::open_in_dir(dir(g)).expect("open tantivy index (run build first)");
                index.tokenizers().register("fold", analyzer());
                let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into().unwrap();
                let fields = cols.iter().map(|c| index.schema().get_field(c).unwrap()).collect();
                GroupIdx { index, reader, fields }
            })
            .collect();
        Tant { groups, analyzer: analyzer() }
    }

    fn tokenize(&self, q: &str) -> Vec<String> {
        let mut a = self.analyzer.clone();
        let mut ts = a.token_stream(q);
        let mut out = Vec::new();
        while ts.advance() {
            out.push(ts.token().text.clone());
        }
        out
    }

    fn group(&self, gi: usize, toks: &[String]) -> Vec<Doc> {
        if toks.is_empty() {
            return Vec::new();
        }
        let g = &self.groups[gi];
        let n = toks.len();
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            let last = i + 1 == n;
            let dist: u8 = if t.chars().count() >= 4 { 1 } else { 0 };
            let mut per_field: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            for f in &g.fields {
                let term = Term::from_field_text(*f, t);
                let fq: Box<dyn Query> = if last {
                    Box::new(FuzzyTermQuery::new_prefix(term, dist, true))
                } else {
                    Box::new(FuzzyTermQuery::new(term, dist, true))
                };
                per_field.push((Occur::Should, fq));
            }
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(per_field))));
            // exact term match only contributes BM25 score, so exact beats fuzzy
            for (fi, f) in g.fields.iter().enumerate() {
                let tq = TermQuery::new(Term::from_field_text(*f, t), IndexRecordOption::WithFreqs);
                let boost = if fi == 0 { 3.0 } else { 1.0 };
                clauses.push((Occur::Should, Box::new(tantivy::query::BoostQuery::new(Box::new(tq), boost))));
            }
        }
        let q = BooleanQuery::new(clauses);
        let searcher = g.reader.searcher();
        let top = searcher.search(&q, &TopDocs::with_limit(CAND).order_by_score()).unwrap();
        let _ = &g.index;
        top.into_iter()
            .map(|(_, addr)| {
                let d: TantivyDocument = searcher.doc(addr).unwrap();
                let get = |i: usize| {
                    g.fields
                        .get(i)
                        .and_then(|f| d.get_first(*f))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                Doc { a: get(0), b: get(1), c: get(2) }
            })
            .collect()
    }
}

impl SearchIndex for Tant {
    fn candidates(&self, q: &str) -> Groups {
        let toks = self.tokenize(q);
        Groups { tracks: self.group(0, &toks), albums: self.group(1, &toks), artists: self.group(2, &toks) }
    }
}
