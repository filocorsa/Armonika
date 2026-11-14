use std::sync::Arc;

use armonika_core::db::Db;
use armonika_core::ids::{ArtistId, LibraryId, TrackId};
use armonika_core::model::LibraryChange;
use armonika_core::search_source;
use armonika_search::{
    GroupLimits, INDEX_SCHEMA_VERSION, SearchIndex, SearchQuery, TantivyIndex, updater,
};
use rusqlite::params;
use tokio::sync::mpsc;

const LIB: LibraryId = LibraryId(1);

struct Env {
    _dir: tempfile::TempDir,
    db: Db,
    index: Arc<TantivyIndex>,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("armonika.db")).unwrap();
    db.write_blocking(|c| {
        c.execute(
            "INSERT INTO libraries (id, name, root, created_at) VALUES (1, 'Music', '/m', 0)",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let index = Arc::new(TantivyIndex::open(&dir.path().join("search")).unwrap());
    Env {
        _dir: dir,
        db,
        index,
    }
}

impl Env {
    fn track(&self, public: &str, title: &str, state: &str, artist: &str) {
        self.db
            .write_blocking(|c| {
                c.execute(
                    "INSERT INTO tracks (public_id, library_id, state, rel_path, size, mtime_ms,
                                         tag_schema, format, title, artist_credit, added_at,
                                         updated_at)
                     VALUES (?1, 1, ?2, ?1, 0, 0, 1, 'mp3', ?3, ?4, 0, 0)",
                    params![public, state, title, artist],
                )?;
                let id = c.last_insert_rowid();
                let artist_key = self.artist_key(c, artist)?;
                c.execute(
                    "INSERT INTO track_artists (track_id, position, artist_id, credited_name)
                     VALUES (?1, 0, ?2, ?3)",
                    params![id, artist_key, artist],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn artist_key(&self, c: &rusqlite::Connection, name: &str) -> armonika_core::Result<i64> {
        let found: Option<i64> = c
            .query_row(
                "SELECT id FROM artists WHERE norm_name = ?1",
                [name.to_lowercase()],
                |r| r.get(0),
            )
            .ok();
        if let Some(id) = found {
            return Ok(id);
        }
        c.execute(
            "INSERT INTO artists (public_id, name, norm_name, updated_at) VALUES (?1, ?2, ?3, 0)",
            params![ArtistId::generate(), name, name.to_lowercase()],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// An artist without any track or album.
    fn orphan_artist(&self, name: &str) -> ArtistId {
        self.db
            .write_blocking(|c| {
                let key = self.artist_key(c, name)?;
                Ok(
                    c.query_row("SELECT public_id FROM artists WHERE id = ?1", [key], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap()
    }

    fn set_state(&self, public: &str, state: &str) {
        self.db
            .write_blocking(|c| {
                c.execute(
                    "UPDATE tracks SET state = ?2 WHERE public_id = ?1",
                    params![public, state],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn find(&self, text: &str) -> armonika_search::SearchHits {
        let q = SearchQuery {
            text: text.into(),
            libraries: vec![LIB],
            limits: GroupLimits {
                tracks: 20,
                albums: 20,
                artists: 20,
            },
            ..SearchQuery::default()
        };
        self.index.search(&q).unwrap()
    }

    fn as_index(&self) -> Arc<dyn SearchIndex> {
        self.index.clone()
    }
}

fn tid(public: &str) -> TrackId {
    public.parse().unwrap()
}

#[tokio::test]
async fn rebuild_matches_database() {
    let e = env();
    e.track("t-a1", "Alpha Song", "present", "Alpha Band");
    e.track("t-a2", "Alpha Tune", "missing", "Alpha Band");
    e.track("t-a3", "Alpha Gone", "deleted", "Alpha Band");
    e.orphan_artist("Alpha Orphan");

    updater::rebuild(e.db.clone(), e.as_index()).await.unwrap();

    let hits = e.find("alpha");
    assert_eq!(hits.tracks, [tid("t-a1")]);
    assert_eq!(
        hits.artists.len(),
        1,
        "only the artist with a visible track"
    );
    let stored =
        e.db.read_blocking(|c| search_source::setting(c, "search_index_version"))
            .unwrap();
    assert_eq!(stored, Some(INDEX_SCHEMA_VERSION.to_string()));
}

#[tokio::test]
async fn rebuild_if_needed_only_when_missing_or_outdated() {
    let e = env();
    e.track("t-b1", "Beta Song", "present", "Beta Band");
    // New index: rebuilt, and the version is recorded.
    assert!(
        updater::rebuild_if_needed(e.db.clone(), e.as_index(), true)
            .await
            .unwrap()
    );
    assert_eq!(e.find("beta").tracks.len(), 1);
    // Same version, existing index: left alone.
    e.index.clear().unwrap();
    assert!(
        !updater::rebuild_if_needed(e.db.clone(), e.as_index(), false)
            .await
            .unwrap()
    );
    assert!(e.find("beta").tracks.is_empty());
    // Another version: rebuilt.
    e.db.write_blocking(|c| search_source::set_setting(c, "search_index_version", "0"))
        .unwrap();
    assert!(
        updater::rebuild_if_needed(e.db.clone(), e.as_index(), false)
            .await
            .unwrap()
    );
    assert_eq!(e.find("beta").tracks.len(), 1);
}

async fn apply(e: &Env, change: LibraryChange) {
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(change).unwrap();
    drop(tx);
    updater::run(e.db.clone(), e.as_index(), rx).await;
}

#[tokio::test]
async fn missing_track_leaves_index() {
    let e = env();
    e.track("t-c1", "Gamma Song", "present", "Gamma Band");
    e.track("t-c2", "Gamma Tune", "present", "Gamma Band");
    updater::rebuild(e.db.clone(), e.as_index()).await.unwrap();
    assert_eq!(e.find("gamma").tracks.len(), 2);

    e.set_state("t-c2", "missing");
    let mut change = LibraryChange::new(LIB);
    change.tracks_hidden.push(tid("t-c2"));
    apply(&e, change).await;
    assert_eq!(e.find("gamma").tracks, [tid("t-c1")]);

    // A new present track in the same event stream is indexed.
    e.track("t-c3", "Gamma Fresh", "present", "Gamma Band");
    let mut change = LibraryChange::new(LIB);
    change.tracks_upserted.push(tid("t-c3"));
    apply(&e, change).await;
    assert_eq!(e.find("gamma").tracks.len(), 2);
}

#[tokio::test]
async fn touched_artist_without_links_is_removed() {
    let e = env();
    e.track("t-d1", "Delta Song", "present", "Delta Band");
    updater::rebuild(e.db.clone(), e.as_index()).await.unwrap();
    let artist = e.find("delta").artists[0].clone();

    // The track goes and its artist keeps no visible link, then an orphan name-only
    // artist is touched too: neither may stay (or enter) the index.
    e.set_state("t-d1", "missing");
    let orphan = e.orphan_artist("Delta Orphan");
    let mut change = LibraryChange::new(LIB);
    change.tracks_hidden.push(tid("t-d1"));
    change.artists_touched.push(artist);
    change.artists_touched.push(orphan);
    apply(&e, change).await;
    assert!(e.find("delta").artists.is_empty());
    assert!(e.find("delta").tracks.is_empty());
}

/// An index whose writes can be made to fail.
struct Flaky {
    inner: Arc<TantivyIndex>,
    fail: std::sync::atomic::AtomicBool,
}

impl Flaky {
    fn new(e: &Env) -> Arc<Flaky> {
        Arc::new(Flaky {
            inner: e.index.clone(),
            fail: std::sync::atomic::AtomicBool::new(true),
        })
    }
}

impl SearchIndex for Flaky {
    fn search(&self, q: &SearchQuery) -> armonika_core::Result<armonika_search::SearchHits> {
        self.inner.search(q)
    }
    fn upsert(&self, docs: Vec<armonika_search::SearchDoc>) -> armonika_core::Result<()> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(armonika_core::Error::Search("injected".into()));
        }
        self.inner.upsert(docs)
    }
    fn remove(&self, ids: Vec<armonika_search::DocId>) -> armonika_core::Result<()> {
        self.inner.remove(ids)
    }
    fn clear(&self) -> armonika_core::Result<()> {
        self.inner.clear()
    }
}

fn stored_version(e: &Env) -> Option<String> {
    e.db.read_blocking(|c| search_source::setting(c, "search_index_version"))
        .unwrap()
}

#[tokio::test]
async fn interrupted_rebuild_leaves_no_version() {
    let e = env();
    e.track("t-e1", "Epsilon Song", "present", "Epsilon Band");
    updater::rebuild(e.db.clone(), e.as_index()).await.unwrap();
    assert!(stored_version(&e).is_some());

    let flaky: Arc<dyn SearchIndex> = Flaky::new(&e);
    assert!(updater::rebuild(e.db.clone(), flaky).await.is_err());
    assert_eq!(
        stored_version(&e),
        None,
        "a partial index must not look complete"
    );
    // So the next start rebuilds.
    assert!(
        updater::rebuild_if_needed(e.db.clone(), e.as_index(), false)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn failed_batch_marks_the_index_outdated() {
    let e = env();
    e.track("t-f1", "Zeta Song", "present", "Zeta Band");
    updater::rebuild(e.db.clone(), e.as_index()).await.unwrap();
    assert!(stored_version(&e).is_some());

    let flaky: Arc<dyn SearchIndex> = Flaky::new(&e);
    e.track("t-f2", "Zeta Tune", "present", "Zeta Band");
    let mut change = LibraryChange::new(LIB);
    change.tracks_upserted.push(tid("t-f2"));
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send(change).unwrap();
    drop(tx);
    updater::run(e.db.clone(), flaky, rx).await;
    assert_eq!(stored_version(&e), None);
}

#[tokio::test]
async fn closing_the_channel_flushes_without_waiting_for_the_window() {
    let e = env();
    e.track("t-g1", "Eta Song", "present", "Eta Band");
    let mut change = LibraryChange::new(LIB);
    change.tracks_upserted.push(tid("t-g1"));
    let started = std::time::Instant::now();
    apply(&e, change).await;
    assert!(started.elapsed() < updater::COMMIT_INTERVAL);
    assert_eq!(e.find("eta").tracks, [tid("t-g1")]);
}
