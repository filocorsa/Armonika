//! Keeps the search index in line with the database.
//!
//! [`run`] listens to the [`LibraryChange`] events of the scanner and the hasher,
//! gathers the ones that arrive within a short window and applies them in one batch, so
//! the index commits at most once every [`COMMIT_INTERVAL`]. The documents always come
//! from the database, never from the event: an event only says what to look at. What is
//! indexed is what users can see: present tracks, albums with a present track, and
//! artists linked to one of those. Anything an event names that is not visible any more
//! (or never was) leaves the index.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use armonika_core::Result;
use armonika_core::db::Db;
use armonika_core::ids::{ArtistId, LibraryAlbumId, TrackId};
use armonika_core::model::LibraryChange;
use armonika_core::search_source::{
    self, IndexAlbum, IndexArtist, IndexTrack, all_present_track_ids, all_visible_album_ids,
    all_visible_artist_ids, present_tracks, visible_albums, visible_artists,
};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::{DocId, INDEX_SCHEMA_VERSION, SearchDoc, SearchIndex};

/// Events that arrive within this time of the first one are applied together.
pub const COMMIT_INTERVAL: Duration = Duration::from_secs(2);

/// Name of the `settings` row holding the schema version the index was built with.
pub const VERSION_SETTING: &str = "search_index_version";

/// Documents per commit during a rebuild.
const REBUILD_BATCH: usize = 5000;

/// What to look at in the database.
#[derive(Default)]
struct Pending {
    tracks: BTreeSet<TrackId>,
    albums: BTreeSet<LibraryAlbumId>,
    artists: BTreeSet<ArtistId>,
}

impl Pending {
    fn add(&mut self, change: LibraryChange) {
        self.tracks.extend(change.tracks_upserted);
        self.tracks.extend(change.tracks_hidden);
        self.albums.extend(change.albums_touched);
        self.artists.extend(change.artists_touched);
    }
}

/// Applies the changes it receives until the channel closes. Errors are logged: the
/// next event, or the rebuild at the next start, repairs the index.
pub async fn run(
    db: Db,
    index: Arc<dyn SearchIndex>,
    mut rx: mpsc::UnboundedReceiver<LibraryChange>,
) {
    while let Some(first) = rx.recv().await {
        let mut pending = Pending::default();
        pending.add(first);
        let deadline = Instant::now() + COMMIT_INTERVAL;
        let mut closed = false;
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => break,
                next = rx.recv() => match next {
                    Some(change) => pending.add(change),
                    None => {
                        closed = true;
                        break;
                    }
                },
            }
        }
        if let Err(e) = apply(&db, &index, pending).await {
            tracing::warn!(error = %e, "cannot update the search index");
        }
        if closed {
            break;
        }
    }
}

async fn apply(db: &Db, index: &Arc<dyn SearchIndex>, pending: Pending) -> Result<()> {
    let tracks: Vec<TrackId> = pending.tracks.into_iter().collect();
    let albums: Vec<LibraryAlbumId> = pending.albums.into_iter().collect();
    let artists: Vec<ArtistId> = pending.artists.into_iter().collect();
    let (found_tracks, found_albums, found_artists) = {
        let (t, al, ar) = (tracks.clone(), albums.clone(), artists.clone());
        db.read(move |c| {
            Ok((
                present_tracks(c, &t)?,
                visible_albums(c, &al)?,
                visible_artists(c, &ar)?,
            ))
        })
        .await?
    };
    let docs: Vec<SearchDoc> = found_tracks
        .iter()
        .map(track_doc)
        .chain(found_albums.iter().map(album_doc))
        .chain(found_artists.iter().map(artist_doc))
        .collect();
    let kept: BTreeSet<String> = docs.iter().map(|d| d.id.to_string()).collect();
    let gone: Vec<DocId> = tracks
        .into_iter()
        .map(DocId::Track)
        .chain(albums.into_iter().map(DocId::Album))
        .chain(artists.into_iter().map(DocId::Artist))
        .filter(|id| !kept.contains(id.as_str()))
        .collect();
    write(index, gone, docs).await
}

/// Removes `gone` and adds `docs`, on a blocking thread.
async fn write(index: &Arc<dyn SearchIndex>, gone: Vec<DocId>, docs: Vec<SearchDoc>) -> Result<()> {
    let index = Arc::clone(index);
    tokio::task::spawn_blocking(move || {
        if !gone.is_empty() {
            index.remove(gone)?;
        }
        if !docs.is_empty() {
            index.upsert(docs)?;
        }
        Ok(())
    })
    .await?
}

/// Empties the index and indexes everything visible in the database, then records
/// [`INDEX_SCHEMA_VERSION`].
pub async fn rebuild(db: Db, index: Arc<dyn SearchIndex>) -> Result<()> {
    let idx = Arc::clone(&index);
    tokio::task::spawn_blocking(move || idx.clear()).await??;
    let (tracks, albums, artists) = db
        .read(|c| {
            Ok((
                all_present_track_ids(c)?,
                all_visible_album_ids(c)?,
                all_visible_artist_ids(c)?,
            ))
        })
        .await?;
    for chunk in tracks.chunks(REBUILD_BATCH) {
        let ids = chunk.to_vec();
        let found = db.read(move |c| present_tracks(c, &ids)).await?;
        write(&index, Vec::new(), found.iter().map(track_doc).collect()).await?;
    }
    for chunk in albums.chunks(REBUILD_BATCH) {
        let ids = chunk.to_vec();
        let found = db.read(move |c| visible_albums(c, &ids)).await?;
        write(&index, Vec::new(), found.iter().map(album_doc).collect()).await?;
    }
    for chunk in artists.chunks(REBUILD_BATCH) {
        let ids = chunk.to_vec();
        let found = db.read(move |c| visible_artists(c, &ids)).await?;
        write(&index, Vec::new(), found.iter().map(artist_doc).collect()).await?;
    }
    db.write(|c| search_source::set_setting(c, VERSION_SETTING, &INDEX_SCHEMA_VERSION.to_string()))
        .await?;
    tracing::info!(
        tracks = tracks.len(),
        albums = albums.len(),
        artists = artists.len(),
        "search index rebuilt"
    );
    Ok(())
}

/// Runs [`rebuild`] when the index was just created (`created`) or was built with
/// another schema version. Returns whether it rebuilt.
pub async fn rebuild_if_needed(db: Db, index: Arc<dyn SearchIndex>, created: bool) -> Result<bool> {
    let stored = db
        .read(|c| search_source::setting(c, VERSION_SETTING))
        .await?;
    let current = INDEX_SCHEMA_VERSION.to_string();
    if !created && stored.as_deref() == Some(current.as_str()) {
        return Ok(false);
    }
    rebuild(db, index).await?;
    Ok(true)
}

fn track_doc(t: &IndexTrack) -> SearchDoc {
    SearchDoc {
        id: DocId::Track(t.id.clone()),
        libraries: vec![t.library],
        primary: t.title.clone(),
        secondary: t.artist_credit.clone(),
        tertiary: t.album_title.clone(),
    }
}

fn album_doc(a: &IndexAlbum) -> SearchDoc {
    SearchDoc {
        id: DocId::Album(a.id.clone()),
        libraries: vec![a.library],
        primary: a.title.clone(),
        secondary: a.artist_credit.clone(),
        tertiary: String::new(),
    }
}

fn artist_doc(a: &IndexArtist) -> SearchDoc {
    SearchDoc {
        id: DocId::Artist(a.id.clone()),
        libraries: a.libraries.clone(),
        primary: a.name.clone(),
        secondary: String::new(),
        tertiary: String::new(),
    }
}
