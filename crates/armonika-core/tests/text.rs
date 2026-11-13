use std::path::{Path, PathBuf};
use std::str::FromStr;

use armonika_core::ids::{
    ArtistId, ItemRef, LibraryAlbumId, LibraryId, PlaylistId, TrackId, UserId,
};
use armonika_core::paths::{rel_path_from_db, rel_path_to_db};
use armonika_core::text::{fold_sort, normalize};

#[test]
fn normalize_folds_case_accents_and_punctuation() {
    assert_eq!(normalize("  Beyoncé  Knowles!"), "beyonce knowles");
    assert_eq!(normalize("Sigur Rós"), "sigur ros");
    assert_eq!(normalize("AC/DC"), "ac dc");
    assert_eq!(normalize("坂本龍一"), "坂本龍一");
    assert_eq!(normalize(""), "");
    assert_eq!(normalize(" ?! "), "");
}

#[test]
fn fold_sort_drops_a_leading_article() {
    let articles = ["The", "L'", "Les"];
    assert_eq!(fold_sort("The Beatles", &articles), "beatles");
    assert_eq!(fold_sort("L'Amour", &articles), "amour");
    assert_eq!(fold_sort("Theatre", &articles), "theatre");
    assert_eq!(fold_sort("The", &articles), "the");
    assert_eq!(fold_sort("Radiohead", &[]), "radiohead");
}

#[test]
fn ids_roundtrip_and_reject_wrong_prefix() {
    let t = TrackId::generate();
    assert!(t.as_str().starts_with("t-"), "{}", t.as_str());
    assert_eq!(t.as_str().len(), 2 + 32);
    assert!(!t.as_str()[2..].contains('-'));
    assert_eq!(TrackId::from_str(t.as_str()).unwrap(), t);
    assert_ne!(TrackId::generate(), t);

    assert!(LibraryAlbumId::generate().as_str().starts_with("al-"));
    assert!(ArtistId::generate().as_str().starts_with("ar-"));
    assert!(UserId::generate().as_str().starts_with("u-"));
    assert!(PlaylistId::generate().as_str().starts_with("pl-"));

    let al = LibraryAlbumId::generate();
    assert!(TrackId::from_str(al.as_str()).is_err());
    assert!(ArtistId::from_str(al.as_str()).is_err());
    assert!(TrackId::from_str("t-").is_err());
    assert!(TrackId::from_str("t").is_err());
    assert!(TrackId::from_str("").is_err());
    assert!(PlaylistId::from_str(UserId::generate().as_str()).is_err());

    assert_eq!(LibraryId::from_str("7").unwrap(), LibraryId(7));
    assert!(LibraryId::from_str("x").is_err());
}

#[test]
fn item_ref_roundtrips_through_its_public_id() {
    for item in [
        ItemRef::Track(TrackId::generate()),
        ItemRef::Album(LibraryAlbumId::generate()),
        ItemRef::Artist(ArtistId::generate()),
    ] {
        let s = item.as_str().to_owned();
        assert_eq!(ItemRef::from_str(&s).unwrap(), item);
        assert_eq!(item.to_string(), s);
    }
    assert!(ItemRef::from_str(UserId::generate().as_str()).is_err());
}

#[test]
fn rel_path_uses_forward_slashes() {
    let rel = Path::new("Artist").join("Album").join("01 Song.flac");
    assert_eq!(rel_path_to_db(&rel), "Artist/Album/01 Song.flac");
    assert_eq!(rel_path_from_db("Artist/Album/01 Song.flac"), rel);
    assert_eq!(
        rel_path_from_db(&rel_path_to_db(&rel)),
        PathBuf::from("Artist").join("Album").join("01 Song.flac")
    );
    #[cfg(windows)]
    assert_eq!(rel_path_to_db(Path::new(r"a\b\c.mp3")), "a/b/c.mp3");
}
