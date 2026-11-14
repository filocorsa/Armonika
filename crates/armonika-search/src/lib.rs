//! Search index of the Armonika music server.
//!
//! [`SearchIndex`] is the engine-swappable interface: the engine produces candidates in
//! its own order and [`rerank`] orders them (see the search spike in
//! `spikes/01-search`). [`tantivy_index::TantivyIndex`] is the engine, [`updater`] keeps
//! it in line with the database.

pub mod cjk;
pub mod rerank;
pub mod tantivy_index;
pub mod updater;

use std::sync::Arc;

use armonika_core::Result;
use armonika_core::ids::{ArtistId, LibraryAlbumId, LibraryId, TrackId};

pub use armonika_core::ids::ItemRef as DocId;
pub use tantivy_index::TantivyIndex;

/// Version of the index schema and of the way documents are built. When it differs from
/// the one stored in `settings` the index is rebuilt at startup.
pub const INDEX_SCHEMA_VERSION: u32 = 1;

/// A page size or offset for each kind of result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupLimits {
    pub tracks: usize,
    pub albums: usize,
    pub artists: usize,
}

/// Extra cost reduction of an item, for the popularity signal: the final cost is
/// `cost - boost(id)`. The caller computes it on the candidates only.
pub type Boost = Arc<dyn Fn(&DocId) -> f32 + Send + Sync>;

/// A search. An empty `libraries` list matches nothing: callers always pass the
/// libraries the user may listen to.
#[derive(Clone, Default)]
pub struct SearchQuery {
    pub text: String,
    pub libraries: Vec<LibraryId>,
    pub limits: GroupLimits,
    pub offsets: GroupLimits,
    pub boost: Option<Boost>,
}

/// The identifiers found, best first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchHits {
    pub tracks: Vec<TrackId>,
    pub albums: Vec<LibraryAlbumId>,
    pub artists: Vec<ArtistId>,
}

/// A document of the index. Track: title, credit, album title. Album: title, credit,
/// empty. Artist: name, empty, empty. An artist belongs to the libraries where it has
/// visible tracks or albums.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchDoc {
    pub id: DocId,
    pub libraries: Vec<LibraryId>,
    pub primary: String,
    pub secondary: String,
    pub tertiary: String,
}

/// A full-text index over tracks, albums and artists.
pub trait SearchIndex: Send + Sync {
    /// The best matches for `q`.
    fn search(&self, q: &SearchQuery) -> Result<SearchHits>;
    /// Adds the documents, replacing the ones with the same identifier.
    fn upsert(&self, docs: Vec<SearchDoc>) -> Result<()>;
    /// Removes the documents with these identifiers; unknown ones are ignored.
    fn remove(&self, ids: Vec<DocId>) -> Result<()>;
    /// Removes every document.
    fn clear(&self) -> Result<()>;
}
