//! The Tantivy engine: three indexes (tracks, albums, artists) under one directory.
//!
//! Text fields use the `fold` analyzer (simple tokenizer, lowercase, ASCII folding);
//! each has a twin `*_cjk` field with the bigram tokenizer. A query word becomes a fuzzy
//! clause on every text field (distance 1 from four characters, the last word also as
//! a prefix), words in AND; exact matches only add score, the title three times. The
//! CJK runs of the query become bigram clauses on the `*_cjk` fields, in AND. Tantivy
//! returns candidates, [`crate::rerank`] orders them.
//!
//! An `IndexWriter` lives only for the length of one operation (heap of 15 MB, one
//! thread) and is closed right after the commit, so an idle server keeps no indexing
//! memory.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use armonika_core::ids::{ArtistId, LibraryAlbumId, LibraryId, TrackId};
use armonika_core::{Error, Result};
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
    Value,
};
use tantivy::tokenizer::{AsciiFoldingFilter, LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{Index, IndexReader, ReloadPolicy, TantivyDocument, Term};

use crate::cjk::{self, CjkBigramTokenizer};
use crate::rerank::{self, Doc};
use crate::{DocId, SearchDoc, SearchHits, SearchIndex, SearchQuery};

const FOLD: &str = "fold";
const WRITER_HEAP: usize = 15_000_000;
const MIN_CANDIDATES: usize = 50;
const MAX_CANDIDATES: usize = 500;
const TEXT_FIELDS: [&str; 3] = ["primary", "secondary", "tertiary"];
const CJK_FIELDS: [&str; 3] = ["primary_cjk", "secondary_cjk", "tertiary_cjk"];

fn search_err(e: impl std::fmt::Display) -> Error {
    Error::Search(e.to_string())
}

#[derive(Clone, Copy)]
enum Kind {
    Track,
    Album,
    Artist,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Track, Kind::Album, Kind::Artist];

    fn dir(self) -> &'static str {
        match self {
            Kind::Track => "tracks",
            Kind::Album => "albums",
            Kind::Artist => "artists",
        }
    }

    fn of(id: &DocId) -> Kind {
        match id {
            DocId::Track(_) => Kind::Track,
            DocId::Album(_) => Kind::Album,
            DocId::Artist(_) => Kind::Artist,
        }
    }

    fn slot(self) -> usize {
        self as usize
    }
}

fn analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build()
}

fn schema() -> Schema {
    let mut b = Schema::builder();
    b.add_text_field("id", STRING | STORED);
    b.add_u64_field("library", INDEXED);
    let text = |tokenizer: &str, stored: bool| {
        let opts = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(tokenizer)
                .set_index_option(IndexRecordOption::WithFreqs),
        );
        if stored { opts.set_stored() } else { opts }
    };
    for name in TEXT_FIELDS {
        b.add_text_field(name, text(FOLD, true));
    }
    for name in CJK_FIELDS {
        b.add_text_field(name, text(cjk::TOKENIZER_NAME, false));
    }
    b.build()
}

struct Group {
    index: Index,
    reader: IndexReader,
    id: Field,
    library: Field,
    text: [Field; 3],
    cjk: [Field; 3],
}

impl Group {
    fn open(dir: &Path) -> Result<(Group, bool)> {
        let created = !dir.join("meta.json").exists();
        std::fs::create_dir_all(dir).map_err(search_err)?;
        let index = match Self::open_index(dir) {
            Ok(index) => index,
            Err(e) => {
                // A schema from another version, or damaged files: the index only
                // holds a copy of the database, so start again (the caller rebuilds).
                tracing::warn!(dir = %dir.display(), error = %e, "search index unusable, recreating it");
                std::fs::remove_dir_all(dir).map_err(search_err)?;
                std::fs::create_dir_all(dir).map_err(search_err)?;
                Self::open_index(dir)?
            }
        };
        index.tokenizers().register(FOLD, analyzer());
        index
            .tokenizers()
            .register(cjk::TOKENIZER_NAME, CjkBigramTokenizer);
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(search_err)?;
        let s = index.schema();
        let field = |n: &str| s.get_field(n).map_err(search_err);
        let group = Group {
            id: field("id")?,
            library: field("library")?,
            text: [field("primary")?, field("secondary")?, field("tertiary")?],
            cjk: [
                field("primary_cjk")?,
                field("secondary_cjk")?,
                field("tertiary_cjk")?,
            ],
            index,
            reader,
        };
        Ok((group, created))
    }

    fn open_index(dir: &Path) -> Result<Index> {
        let mmap = MmapDirectory::open(dir).map_err(search_err)?;
        Index::open_or_create(mmap, schema()).map_err(search_err)
    }

    /// Runs `f` with a fresh writer, commits, closes the writer and reloads the reader.
    fn write(&self, f: impl FnOnce(&mut tantivy::IndexWriter) -> Result<()>) -> Result<()> {
        let mut writer: tantivy::IndexWriter = self
            .index
            .writer_with_num_threads(1, WRITER_HEAP)
            .map_err(search_err)?;
        f(&mut writer)?;
        writer.commit().map_err(search_err)?;
        writer.wait_merging_threads().map_err(search_err)?;
        self.reader.reload().map_err(search_err)
    }

    fn add(&self, doc: &SearchDoc) -> TantivyDocument {
        let mut d = TantivyDocument::default();
        d.add_text(self.id, doc.id.as_str());
        for lib in &doc.libraries {
            d.add_u64(self.library, lib.0.unsigned_abs());
        }
        for (i, text) in [&doc.primary, &doc.secondary, &doc.tertiary]
            .into_iter()
            .enumerate()
        {
            d.add_text(self.text[i], text);
            d.add_text(self.cjk[i], text);
        }
        d
    }

    /// The Tantivy query for `words` (folded query words) and `cjk_runs`.
    fn query(
        &self,
        words: &[String],
        cjk_runs: &[String],
        libraries: &[LibraryId],
    ) -> Box<dyn Query> {
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        for (i, word) in words.iter().enumerate() {
            let last = i + 1 == words.len();
            let distance = u8::from(word.chars().count() >= 4);
            let mut per_field: Vec<(Occur, Box<dyn Query>)> = Vec::new();
            for field in self.text {
                let term = Term::from_field_text(field, word);
                let fuzzy: Box<dyn Query> = if last {
                    Box::new(FuzzyTermQuery::new_prefix(term, distance, true))
                } else {
                    Box::new(FuzzyTermQuery::new(term, distance, true))
                };
                per_field.push((Occur::Should, fuzzy));
            }
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(per_field))));
            // Exact matches only add score, so exact beats fuzzy and the title beats
            // the other fields.
            for (fi, field) in self.text.iter().enumerate() {
                let exact = TermQuery::new(
                    Term::from_field_text(*field, word),
                    IndexRecordOption::WithFreqs,
                );
                let boost = if fi == 0 { 3.0 } else { 1.0 };
                clauses.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(exact), boost)),
                ));
            }
        }
        for run in cjk_runs {
            for gram in cjk::ngrams(run) {
                let per_field: Vec<(Occur, Box<dyn Query>)> = self
                    .cjk
                    .iter()
                    .map(|f| {
                        let q: Box<dyn Query> = Box::new(TermQuery::new(
                            Term::from_field_text(*f, &gram),
                            IndexRecordOption::Basic,
                        ));
                        (Occur::Should, q)
                    })
                    .collect();
                clauses.push((Occur::Must, Box::new(BooleanQuery::new(per_field))));
            }
        }
        let in_library: Vec<(Occur, Box<dyn Query>)> = libraries
            .iter()
            .map(|l| {
                let q: Box<dyn Query> = Box::new(TermQuery::new(
                    Term::from_field_u64(self.library, l.0.unsigned_abs()),
                    IndexRecordOption::Basic,
                ));
                (Occur::Should, q)
            })
            .collect();
        clauses.push((Occur::Must, Box::new(BooleanQuery::new(in_library))));
        Box::new(BooleanQuery::new(clauses))
    }

    /// The best `limit` candidates in the order of the engine: identifier and text.
    fn candidates(&self, q: &dyn Query, limit: usize) -> Result<Vec<(String, Doc)>> {
        let searcher = self.reader.searcher();
        let top = searcher
            .search(q, &TopDocs::with_limit(limit).order_by_score())
            .map_err(search_err)?;
        let mut out = Vec::with_capacity(top.len());
        for (_, addr) in top {
            let d: TantivyDocument = searcher.doc(addr).map_err(search_err)?;
            let get = |f: Field| {
                d.get_first(f)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned()
            };
            out.push((
                get(self.id),
                Doc {
                    a: get(self.text[0]),
                    b: get(self.text[1]),
                    c: get(self.text[2]),
                },
            ));
        }
        Ok(out)
    }
}

/// The index on disk, in `data_dir/search/`.
pub struct TantivyIndex {
    groups: [Group; 3],
    analyzer: TextAnalyzer,
    created: bool,
    /// One writer at a time: Tantivy refuses a second one.
    write_lock: Mutex<()>,
}

impl TantivyIndex {
    /// Opens the index in `dir`, creating it empty if it does not exist (or cannot be
    /// read: it only holds a copy of the database).
    pub fn open(dir: &Path) -> Result<TantivyIndex> {
        let mut created = false;
        let mut groups = Vec::new();
        for kind in Kind::ALL {
            let path: PathBuf = dir.join(kind.dir());
            let (group, new) = Group::open(&path)?;
            created |= new;
            groups.push(group);
        }
        let Ok(groups) = <[Group; 3]>::try_from(groups) else {
            unreachable!("three kinds, three groups")
        };
        Ok(TantivyIndex {
            groups,
            analyzer: analyzer(),
            created,
            write_lock: Mutex::new(()),
        })
    }

    /// Whether [`TantivyIndex::open`] had to create the index: it is empty and needs a
    /// rebuild from the database.
    pub fn was_created(&self) -> bool {
        self.created
    }

    fn group(&self, kind: Kind) -> &Group {
        &self.groups[kind.slot()]
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn words(&self, text: &str) -> Vec<String> {
        let mut analyzer = self.analyzer.clone();
        let mut stream = analyzer.token_stream(text);
        let mut out = Vec::new();
        while stream.advance() {
            out.push(stream.token().text.clone());
        }
        out
    }

    fn search_kind(
        &self,
        kind: Kind,
        q: &SearchQuery,
        words: &[String],
        runs: &[String],
        offset: usize,
        limit: usize,
    ) -> Result<Vec<DocId>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let group = self.group(kind);
        let want = (offset + limit).clamp(MIN_CANDIDATES, MAX_CANDIDATES);
        let query = group.query(words, runs, &q.libraries);
        let found = group.candidates(query.as_ref(), want)?;
        let ids: Vec<DocId> = found
            .iter()
            .filter_map(|(id, _)| parse_id(kind, id))
            .collect();
        // Candidates whose identifier cannot be read are dropped from both lists.
        let docs: Vec<Doc> = found
            .into_iter()
            .filter(|(id, _)| parse_id(kind, id).is_some())
            .map(|(_, doc)| doc)
            .collect();
        let boost = |i: usize| q.boost.as_ref().map_or(0.0, |b| b(&ids[i]));
        let order = rerank::rerank_group(&q.text, &docs, &boost);
        Ok(order
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|i| ids[i].clone())
            .collect())
    }
}

fn parse_id(kind: Kind, id: &str) -> Option<DocId> {
    match kind {
        Kind::Track => id.parse::<TrackId>().ok().map(DocId::Track),
        Kind::Album => id.parse::<LibraryAlbumId>().ok().map(DocId::Album),
        Kind::Artist => id.parse::<ArtistId>().ok().map(DocId::Artist),
    }
}

impl SearchIndex for TantivyIndex {
    fn search(&self, q: &SearchQuery) -> Result<SearchHits> {
        let runs = cjk::sequences(&q.text);
        let words = self.words(&cjk::without_cjk(&q.text));
        let mut hits = SearchHits::default();
        if (words.is_empty() && runs.is_empty()) || q.libraries.is_empty() {
            return Ok(hits);
        }
        let run = |kind, offset, limit| self.search_kind(kind, q, &words, &runs, offset, limit);
        for id in run(Kind::Track, q.offsets.tracks, q.limits.tracks)? {
            if let DocId::Track(id) = id {
                hits.tracks.push(id);
            }
        }
        for id in run(Kind::Album, q.offsets.albums, q.limits.albums)? {
            if let DocId::Album(id) = id {
                hits.albums.push(id);
            }
        }
        for id in run(Kind::Artist, q.offsets.artists, q.limits.artists)? {
            if let DocId::Artist(id) = id {
                hits.artists.push(id);
            }
        }
        Ok(hits)
    }

    fn upsert(&self, docs: Vec<SearchDoc>) -> Result<()> {
        let _guard = self.lock();
        for kind in Kind::ALL {
            let mine: Vec<&SearchDoc> = docs
                .iter()
                .filter(|d| Kind::of(&d.id).slot() == kind.slot())
                .collect();
            if mine.is_empty() {
                continue;
            }
            let group = self.group(kind);
            group.write(|w| {
                for doc in mine {
                    w.delete_term(Term::from_field_text(group.id, doc.id.as_str()));
                    w.add_document(group.add(doc)).map_err(search_err)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn remove(&self, ids: Vec<DocId>) -> Result<()> {
        let _guard = self.lock();
        for kind in Kind::ALL {
            let mine: Vec<&DocId> = ids
                .iter()
                .filter(|d| Kind::of(d).slot() == kind.slot())
                .collect();
            if mine.is_empty() {
                continue;
            }
            let group = self.group(kind);
            group.write(|w| {
                for id in mine {
                    w.delete_term(Term::from_field_text(group.id, id.as_str()));
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let _guard = self.lock();
        for kind in Kind::ALL {
            self.group(kind).write(|w| {
                w.delete_all_documents().map_err(search_err)?;
                Ok(())
            })?;
        }
        Ok(())
    }
}
