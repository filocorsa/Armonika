use armonika_core::ids::{LibraryId, TrackId};
use armonika_search::cjk::{ngrams, sequences};
use armonika_search::{DocId, GroupLimits, SearchDoc, SearchIndex, SearchQuery, TantivyIndex};

const LIB: LibraryId = LibraryId(1);

fn track_id(n: u32) -> TrackId {
    format!("t-{n:04}").parse().unwrap()
}

fn doc(n: u32, title: &str, credit: &str) -> SearchDoc {
    SearchDoc {
        id: DocId::Track(track_id(n)),
        libraries: vec![LIB],
        primary: title.into(),
        secondary: credit.into(),
        tertiary: String::new(),
    }
}

fn found(index: &TantivyIndex, text: &str) -> Vec<TrackId> {
    let q = SearchQuery {
        text: text.into(),
        libraries: vec![LIB],
        limits: GroupLimits {
            tracks: 10,
            albums: 10,
            artists: 10,
        },
        ..SearchQuery::default()
    };
    index.search(&q).unwrap().tracks
}

fn index() -> (tempfile::TempDir, TantivyIndex) {
    let dir = tempfile::tempdir().unwrap();
    let index = TantivyIndex::open(&dir.path().join("search")).unwrap();
    (dir, index)
}

// Sakamoto Ryuichi, "Ai" (love), "Yoru ni kakeru".
const RYUICHI: &str = "\u{5742}\u{672c}\u{9f8d}\u{4e00}";
const AI: &str = "\u{611b}";
const YORU_NI: &str = "\u{591c}\u{306b}";
const YORU_NI_KAKERU: &str = "\u{591c}\u{306b}\u{99c6}\u{3051}\u{308b}";

#[test]
fn bigrams_and_single_chars() {
    assert_eq!(sequences(&format!("abc {RYUICHI} x {AI}")), [RYUICHI, AI]);
    assert_eq!(
        ngrams("\u{5742}\u{672c}\u{9f8d}"),
        ["\u{5742}\u{672c}", "\u{672c}\u{9f8d}"]
    );
    assert_eq!(ngrams(AI), [AI]);
}

#[test]
fn cjk_substring_found() {
    let (_d, index) = index();
    index
        .upsert(vec![doc(1, "Title", RYUICHI), doc(2, "Other", "Someone")])
        .unwrap();
    assert_eq!(found(&index, "\u{9f8d}\u{4e00}"), [track_id(1)]);
    assert_eq!(found(&index, RYUICHI), [track_id(1)]);
    assert!(found(&index, "\u{4e00}\u{9f8d}").is_empty());
}

#[test]
fn single_cjk_char_found() {
    let (_d, index) = index();
    index
        .upsert(vec![doc(1, AI, "X"), doc(2, "Other", "Someone")])
        .unwrap();
    assert_eq!(found(&index, AI), [track_id(1)]);
}

#[test]
fn cjk_mixed_with_latin() {
    let (_d, index) = index();
    index
        .upsert(vec![
            doc(1, YORU_NI_KAKERU, "YOASOBI"),
            doc(2, YORU_NI_KAKERU, "Someone Else"),
            doc(3, "Other", "YOASOBI"),
        ])
        .unwrap();
    assert_eq!(found(&index, &format!("YOASOBI {YORU_NI}")), [track_id(1)]);
}
