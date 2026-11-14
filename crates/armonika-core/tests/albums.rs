use armonika_core::albums::{
    GroupKey, SINGLES_TITLE, VARIOUS_ARTISTS, album_credit, album_folder, group_key,
};
use armonika_core::ids::ArtistId;
use armonika_core::model::TagFacts;

fn s(v: &str) -> Option<String> {
    Some(v.to_owned())
}

fn track(album: Option<&str>, album_artist: Option<&str>, artist: Option<&str>) -> TagFacts {
    TagFacts {
        album_title: album.map(str::to_owned),
        album_artist_credit: album_artist.map(str::to_owned),
        artist_credit: artist.map(str::to_owned),
        ..TagFacts::default()
    }
}

#[test]
fn album_credit_prefers_the_album_artist_tag() {
    let mut t = track(Some("Hits"), Some("Queen"), Some("Queen feat. David Bowie"));
    t.compilation = true;
    assert_eq!(album_credit(&t, true), "Queen");
}

#[test]
fn album_credit_without_album_artist() {
    let mut t = track(Some("Hits"), None, Some("Queen"));
    assert_eq!(album_credit(&t, false), "Queen");
    assert_eq!(album_credit(&t, true), VARIOUS_ARTISTS);
    t.compilation = true;
    assert_eq!(album_credit(&t, false), VARIOUS_ARTISTS);
    assert_eq!(album_credit(&track(Some("Hits"), None, None), false), "");
}

#[test]
fn group_key_by_release_then_tags() {
    let mut t = track(Some("A Night at the Opera"), None, Some("Queen"));
    t.mb_release_id = s("rel-1");
    assert_eq!(group_key(&t, "Queen", None).as_str(), "mb:rel-1");
    t.mb_release_id = None;
    let key = group_key(&t, "Queen", None);
    assert_eq!(key.as_str(), "tag:a night at the opera|queen");
    // Case, accents and punctuation do not split an album.
    let other = track(Some("A NIGHT AT THE OPERA!"), None, Some("Quéen"));
    assert_eq!(group_key(&other, "Quéen", None), key);
}

#[test]
fn group_key_without_album_title_is_the_artist_singles() {
    let artist: ArtistId = "ar-0123abc".parse().unwrap();
    let t = track(None, None, Some("Queen"));
    let key: GroupKey = group_key(&t, "Queen", Some(&artist));
    assert_eq!(key.as_str(), "singles:ar-0123abc");
    assert!(key.is_singles());
    assert_eq!(group_key(&t, "", None).as_str(), "singles:");
    assert_eq!(SINGLES_TITLE, "[Singles]");
}

#[test]
fn album_folder_skips_disc_folders() {
    assert_eq!(album_folder("a.mp3"), "");
    assert_eq!(album_folder("Queen/Hits/01.mp3"), "Queen/Hits");
    assert_eq!(album_folder("Queen/Hits/Disc 1/01.mp3"), "Queen/Hits");
    assert_eq!(album_folder("Queen/Hits/CD2/01.mp3"), "Queen/Hits");
    assert_eq!(
        album_folder("Queen/Hits/disk 10 - Live/01.mp3"),
        "Queen/Hits"
    );
    assert_eq!(album_folder("Disc 1/01.mp3"), "");
    // Not a disc folder.
    assert_eq!(album_folder("Queen/Discovery/01.mp3"), "Queen/Discovery");
    assert_eq!(album_folder("Queen/CDs/01.mp3"), "Queen/CDs");
}
