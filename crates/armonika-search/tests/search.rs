use std::sync::Arc;

use armonika_core::ids::{ArtistId, LibraryId, TrackId};
use armonika_search::{DocId, GroupLimits, SearchDoc, SearchIndex, SearchQuery, TantivyIndex};

const LIB: LibraryId = LibraryId(1);

struct Fixture {
    _dir: tempfile::TempDir,
    index: TantivyIndex,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let index = TantivyIndex::open(&dir.path().join("search")).unwrap();
    Fixture { _dir: dir, index }
}

fn track_id(n: u32) -> TrackId {
    format!("t-{n:04}").parse().unwrap()
}

fn track(n: u32, lib: LibraryId, title: &str, credit: &str, album: &str) -> SearchDoc {
    SearchDoc {
        id: DocId::Track(track_id(n)),
        libraries: vec![lib],
        primary: title.into(),
        secondary: credit.into(),
        tertiary: album.into(),
    }
}

fn artist(n: u32, name: &str) -> SearchDoc {
    SearchDoc {
        id: DocId::Artist(format!("ar-{n:04}").parse().unwrap()),
        libraries: vec![LIB],
        primary: name.into(),
        secondary: String::new(),
        tertiary: String::new(),
    }
}

fn album(n: u32, title: &str, credit: &str) -> SearchDoc {
    SearchDoc {
        id: DocId::Album(format!("al-{n:04}").parse().unwrap()),
        libraries: vec![LIB],
        primary: title.into(),
        secondary: credit.into(),
        tertiary: String::new(),
    }
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        libraries: vec![LIB],
        limits: GroupLimits {
            tracks: 10,
            albums: 10,
            artists: 10,
        },
        ..SearchQuery::default()
    }
}

fn tracks(index: &TantivyIndex, q: &SearchQuery) -> Vec<TrackId> {
    index.search(q).unwrap().tracks
}

#[test]
fn finds_with_one_typo() {
    let f = fixture();
    f.index
        .upsert(vec![track(1, LIB, "Bohemian Rhapsody", "Queen", "Opera")])
        .unwrap();
    assert_eq!(tracks(&f.index, &query("bohemain")), [track_id(1)]);
}

#[test]
fn finds_without_accents() {
    let f = fixture();
    f.index
        .upsert(vec![artist(1, "Beyonc\u{e9}"), artist(2, "Sigur R\u{f3}s")])
        .unwrap();
    let a1: ArtistId = "ar-0001".parse().unwrap();
    let a2: ArtistId = "ar-0002".parse().unwrap();
    assert_eq!(f.index.search(&query("beyonce")).unwrap().artists, [a1]);
    assert_eq!(f.index.search(&query("sigur ros")).unwrap().artists, [a2]);
}

#[test]
fn prefix_on_last_word() {
    let f = fixture();
    f.index
        .upsert(vec![track(1, LIB, "Bohemian Rhapsody", "Queen", "Opera")])
        .unwrap();
    assert_eq!(tracks(&f.index, &query("bohemian rhap")), [track_id(1)]);
}

#[test]
fn multiword_is_and() {
    let f = fixture();
    f.index
        .upsert(vec![
            track(1, LIB, "Bohemian Rhapsody", "Queen", "Opera"),
            track(2, LIB, "Bohemian Like You", "Dandy Warhols", "Thirteen"),
        ])
        .unwrap();
    assert_eq!(tracks(&f.index, &query("bohemian queen")), [track_id(1)]);
    assert!(tracks(&f.index, &query("bohemian zeppelin")).is_empty());
}

#[test]
fn title_beats_album_match() {
    let f = fixture();
    f.index
        .upsert(vec![
            track(1, LIB, "Other Song", "Someone", "Thunder"),
            track(2, LIB, "Thunder", "Someone", "Other Album"),
        ])
        .unwrap();
    assert_eq!(
        tracks(&f.index, &query("thunder")),
        [track_id(2), track_id(1)]
    );
}

#[test]
fn library_filter_hides_other_libraries() {
    let f = fixture();
    f.index
        .upsert(vec![
            track(1, LIB, "Same Name", "A", ""),
            track(2, LibraryId(2), "Same Name", "A", ""),
        ])
        .unwrap();
    assert_eq!(tracks(&f.index, &query("same name")), [track_id(1)]);
    let mut q = query("same name");
    q.libraries = vec![LibraryId(2)];
    assert_eq!(tracks(&f.index, &q), [track_id(2)]);
    q.libraries = vec![LIB, LibraryId(2)];
    assert_eq!(tracks(&f.index, &q).len(), 2);
    q.libraries = Vec::new();
    assert!(tracks(&f.index, &q).is_empty());
}

#[test]
fn boost_reorders_ties() {
    let f = fixture();
    f.index
        .upsert(vec![
            track(1, LIB, "Echo", "A", ""),
            track(2, LIB, "Echo", "A", ""),
        ])
        .unwrap();
    let plain = tracks(&f.index, &query("echo"));
    assert_eq!(plain.len(), 2);
    let favourite = plain[1].clone();
    let mut q = query("echo");
    let fav = DocId::Track(favourite.clone());
    q.boost = Some(Arc::new(
        move |id: &DocId| {
            if *id == fav { 0.5 } else { 0.0 }
        },
    ));
    assert_eq!(tracks(&f.index, &q)[0], favourite);
}

#[test]
fn offset_pages_results() {
    let f = fixture();
    let docs: Vec<SearchDoc> = (1..=5)
        .map(|n| track(n, LIB, &format!("Song {n}"), "A", ""))
        .collect();
    f.index.upsert(docs).unwrap();
    let mut q = query("song");
    q.limits.tracks = 5;
    let all = tracks(&f.index, &q);
    assert_eq!(all.len(), 5);
    q.limits.tracks = 2;
    q.offsets.tracks = 2;
    assert_eq!(tracks(&f.index, &q), all[2..4]);
    q.offsets.tracks = 4;
    assert_eq!(tracks(&f.index, &q), all[4..]);
}

#[test]
fn removed_doc_disappears() {
    let f = fixture();
    f.index
        .upsert(vec![
            track(1, LIB, "Gone Soon", "A", ""),
            album(1, "Gone Soon", "A"),
        ])
        .unwrap();
    assert_eq!(tracks(&f.index, &query("gone")).len(), 1);
    f.index.remove(vec![DocId::Track(track_id(1))]).unwrap();
    let hits = f.index.search(&query("gone")).unwrap();
    assert!(hits.tracks.is_empty());
    assert_eq!(hits.albums.len(), 1);
    f.index.clear().unwrap();
    assert!(f.index.search(&query("gone")).unwrap().albums.is_empty());
}

#[test]
fn upsert_replaces_a_document() {
    let f = fixture();
    f.index
        .upsert(vec![track(1, LIB, "Old Title", "A", "")])
        .unwrap();
    f.index
        .upsert(vec![track(1, LIB, "New Title", "A", "")])
        .unwrap();
    assert!(tracks(&f.index, &query("old")).is_empty());
    assert_eq!(tracks(&f.index, &query("new")), [track_id(1)]);
}

#[test]
fn index_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("search");
    {
        let index = TantivyIndex::open(&path).unwrap();
        assert!(index.was_created());
        index
            .upsert(vec![track(1, LIB, "Persistent", "A", "")])
            .unwrap();
    }
    let index = TantivyIndex::open(&path).unwrap();
    assert!(!index.was_created());
    assert_eq!(tracks(&index, &query("persistent")), [track_id(1)]);
}
