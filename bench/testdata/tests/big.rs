use std::collections::HashSet;

use armonika_testdata::names::{
    PathAllocator, PathParts, build_relative_path, build_relative_path_with_limit,
    sanitize_component,
};
use armonika_testdata::{
    AudioFormat, BigLibrarySpec, GeneratedTrack, generate_big_library, plan_big_library,
};
use lofty::file::TaggedFileExt;
use lofty::tag::Accessor;

fn all_formats() -> Vec<AudioFormat> {
    vec![
        AudioFormat::Mp3,
        AudioFormat::Flac,
        AudioFormat::OggVorbis,
        AudioFormat::Opus,
        AudioFormat::M4a,
        AudioFormat::Wav,
        AudioFormat::Aiff,
    ]
}

fn spec(tracks: usize, seed: u64) -> BigLibrarySpec {
    BigLibrarySpec {
        tracks,
        seed,
        formats: all_formats(),
    }
}

#[test]
fn generates_requested_number_of_readable_files() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = generate_big_library(&spec(200, 7), dir.path()).unwrap();
    assert_eq!(manifest.len(), 200);
    let mut seen = HashSet::new();
    for t in &manifest {
        let full = dir.path().join(&t.path);
        assert!(full.is_file(), "missing {}", full.display());
        assert!(seen.insert(full.clone()));
        let tagged = lofty::read_from_path(&full)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", full.display()));
        let tag = tagged
            .primary_tag()
            .or_else(|| tagged.first_tag())
            .unwrap_or_else(|| panic!("no tag in {}", full.display()));
        assert_eq!(
            tag.title().as_deref(),
            Some(t.title.as_str()),
            "{}",
            full.display()
        );
        assert_eq!(
            tag.artist().as_deref(),
            Some(t.artist.as_str()),
            "{}",
            full.display()
        );
    }
    assert!(dir.path().join("manifest.json").is_file());
}

#[test]
fn same_seed_gives_identical_manifest() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    generate_big_library(&spec(60, 3), a.path()).unwrap();
    generate_big_library(&spec(60, 3), b.path()).unwrap();
    let ma = std::fs::read(a.path().join("manifest.json")).unwrap();
    let mb = std::fs::read(b.path().join("manifest.json")).unwrap();
    assert_eq!(ma, mb);
}

#[test]
fn different_seed_gives_different_manifest() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    generate_big_library(&spec(60, 3), a.path()).unwrap();
    generate_big_library(&spec(60, 4), b.path()).unwrap();
    let ma = std::fs::read(a.path().join("manifest.json")).unwrap();
    let mb = std::fs::read(b.path().join("manifest.json")).unwrap();
    assert_ne!(ma, mb);
}

fn assert_windows_safe(component: &str) {
    assert!(!component.is_empty());
    for c in component.chars() {
        assert!(
            !"<>:\"/\\|?*".contains(c),
            "bad char {c:?} in {component:?}"
        );
        assert!(!c.is_control(), "control char in {component:?}");
    }
    assert!(
        !component.ends_with('.') && !component.ends_with(' '),
        "{component:?}"
    );
    let stem = component.split('.').next().unwrap().to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    assert!(!reserved.contains(&stem.as_str()), "{component:?}");
    for prefix in ["COM", "LPT"] {
        for n in 1..=9 {
            assert_ne!(stem, format!("{prefix}{n}"), "{component:?}");
        }
    }
    assert!(component.chars().count() <= 60, "{component:?}");
}

#[test]
fn sanitize_component_rules() {
    assert_eq!(sanitize_component("AC/DC"), "AC_DC");
    assert_eq!(sanitize_component("what?"), "what_");
    assert_eq!(sanitize_component("end. "), "end");
    assert_eq!(sanitize_component(""), "_");
    assert_eq!(sanitize_component("..."), "_");
    assert_ne!(sanitize_component("CON").to_ascii_uppercase(), "CON");
    assert_ne!(sanitize_component("nul"), "nul");
    assert_ne!(
        sanitize_component("con.txt").to_ascii_uppercase(),
        "CON.TXT"
    );
    assert_windows_safe(&sanitize_component("lpt9.log"));
    // Never split a code point: 100 CJK chars, 3 bytes each.
    let long: String = "\u{6f22}".repeat(100);
    let s = sanitize_component(&long);
    assert_eq!(s.chars().count(), 60);
}

#[test]
fn generates_safe_paths_for_hostile_names() {
    let hostile = [
        "a:b?c\"d<e>f|g*h",
        "CON",
        "nul",
        "Com1",
        "lpt3.txt",
        "trailing dot.",
        "trailing space ",
        "\u{6f22}\u{5b57}\u{306e}\u{540d}\u{524d}: \u{30c6}\u{30b9}\u{30c8}?",
        "tab\there\nnewline",
        "...",
        "",
    ];
    for name in hostile {
        for (album, aa, multi) in [
            (Some(name), Some(name), false),
            (Some(name), Some("Various Artists"), true),
            (None, None, false),
        ] {
            let parts = PathParts {
                album_artist: aa,
                track_artist: name,
                album,
                year: 1999,
                disc: 2,
                multi_disc: multi,
                track: 5,
                title: name,
                extension: "flac",
            };
            let rel = build_relative_path(&parts);
            for comp in rel.split('/') {
                assert_windows_safe(comp);
            }
            assert!(!rel.contains('\\'));
        }
    }
}

#[test]
fn case_insensitive_collisions_do_not_overwrite() {
    let mk = |artist: &'static str| PathParts {
        album_artist: Some(artist),
        track_artist: artist,
        album: Some("Gold"),
        year: 1992,
        disc: 1,
        multi_disc: false,
        track: 1,
        title: "Dancing Queen",
        extension: "mp3",
    };
    let mut alloc = PathAllocator::new();
    let p1 = alloc.allocate(build_relative_path(&mk("ABBA")));
    let p2 = alloc.allocate(build_relative_path(&mk("Abba")));
    let p3 = alloc.allocate(build_relative_path(&mk("abba")));
    assert_eq!(p1, "ABBA/Gold (1992)/01 - Dancing Queen.mp3");
    assert_eq!(p2, "Abba/Gold (1992)/01 - Dancing Queen (2).mp3");
    assert_eq!(p3, "abba/Gold (1992)/01 - Dancing Queen (3).mp3");
    let lower: HashSet<String> = [&p1, &p2, &p3].iter().map(|p| p.to_lowercase()).collect();
    assert_eq!(lower.len(), 3);
}

#[test]
fn path_scheme_special_cases() {
    let base = PathParts {
        album_artist: Some("Artist"),
        track_artist: "Artist",
        album: Some("Album"),
        year: 2001,
        disc: 1,
        multi_disc: false,
        track: 3,
        title: "Song",
        extension: "ogg",
    };
    assert_eq!(
        build_relative_path(&base),
        "Artist/Album (2001)/03 - Song.ogg"
    );
    let va = PathParts {
        album_artist: Some("Various Artists"),
        track_artist: "Someone",
        ..base
    };
    assert_eq!(
        build_relative_path(&va),
        "Various Artists/Album (2001)/03 - Someone - Song.ogg"
    );
    let multi = PathParts {
        multi_disc: true,
        disc: 2,
        ..base
    };
    assert_eq!(
        build_relative_path(&multi),
        "Artist/Album (2001)/Disc 2/03 - Song.ogg"
    );
    let single = PathParts {
        album_artist: None,
        album: None,
        ..base
    };
    assert_eq!(build_relative_path(&single), "Artist/Singles/Song.ogg");
}

#[test]
fn paths_stay_under_windows_limit() {
    let long = "x".repeat(300);
    for (aa, multi, album) in [
        (Some("Various Artists"), true, Some(long.as_str())),
        (Some(long.as_str()), true, Some(long.as_str())),
        (None, false, None),
    ] {
        let parts = PathParts {
            album_artist: aa,
            track_artist: &long,
            album,
            year: 2020,
            disc: 2,
            multi_disc: multi,
            track: 12,
            title: &long,
            extension: "opus",
        };
        let rel = build_relative_path(&parts);
        assert!(rel.chars().count() <= 200, "{}", rel.chars().count());
        for comp in rel.split('/') {
            assert!(comp.chars().count() <= 60);
        }
    }
    // The shortening step itself, with a tighter limit.
    let parts = PathParts {
        album_artist: Some(&long),
        track_artist: &long,
        album: Some(&long),
        year: 2020,
        disc: 2,
        multi_disc: true,
        track: 12,
        title: &long,
        extension: "opus",
    };
    let rel = build_relative_path_with_limit(&parts, 90);
    assert!(rel.chars().count() <= 90, "{rel}");
    assert!(rel.ends_with(".opus"));
    // And on a generated library.
    for t in &plan_big_library(&spec(2000, 11)) {
        assert!(t.path.to_string_lossy().chars().count() <= 200);
    }
}

fn pct(count: usize, total: usize) -> f64 {
    count as f64 * 100.0 / total as f64
}

#[test]
fn distribution_matches_rules() {
    let n = 10_000;
    let plan: Vec<GeneratedTrack> = plan_big_library(&spec(n, 1));
    assert_eq!(plan.len(), n);
    let various = plan
        .iter()
        .filter(|t| t.album_artist.as_deref() == Some("Various Artists"))
        .count();
    let feat = plan.iter().filter(|t| t.artist.contains(" feat. ")).count();
    let no_album = plan.iter().filter(|t| t.album.is_none()).count();
    let v = pct(various, n);
    let f = pct(feat, n);
    let a = pct(no_album, n);
    assert!((1.0..=3.0).contains(&v), "various {v}");
    assert!((2.0..=4.0).contains(&f), "feat {f}");
    assert!((0.5..=1.5).contains(&a), "no album {a}");
    assert!(plan.iter().any(|t| t.disc == 2));
    assert!(plan.iter().any(|t| !t.title.is_ascii()));
    let lower: HashSet<String> = plan
        .iter()
        .map(|t| t.path.to_string_lossy().to_lowercase())
        .collect();
    assert_eq!(lower.len(), n);
}

#[test]
fn plan_matches_written_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let s = spec(80, 5);
    let written = generate_big_library(&s, dir.path()).unwrap();
    assert_eq!(written, plan_big_library(&s));
    let json = std::fs::read_to_string(dir.path().join("manifest.json")).unwrap();
    assert!(!json.contains("\\\\"));
    let back: Vec<GeneratedTrack> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, written);
}

#[test]
fn rejects_empty_formats() {
    let dir = tempfile::tempdir().unwrap();
    let s = BigLibrarySpec {
        tracks: 5,
        seed: 1,
        formats: vec![],
    };
    assert!(generate_big_library(&s, dir.path()).is_err());
}

#[test]
fn collision_suffix_keeps_component_limit() {
    let long = "y".repeat(300);
    let parts = PathParts {
        album_artist: Some("Artist"),
        track_artist: "Artist",
        album: Some("Album"),
        year: 2000,
        disc: 1,
        multi_disc: false,
        track: 1,
        title: &long,
        extension: "flac",
    };
    let mut alloc = PathAllocator::new();
    let first = alloc.allocate(build_relative_path(&parts));
    let second = alloc.allocate(build_relative_path(&parts));
    let third = alloc.allocate(build_relative_path(&parts));
    assert_ne!(first.to_lowercase(), second.to_lowercase());
    assert_ne!(second.to_lowercase(), third.to_lowercase());
    for p in [&first, &second, &third] {
        assert!(p.chars().count() <= 200);
        for comp in p.split('/') {
            assert_windows_safe(comp);
        }
        assert!(p.ends_with(".flac"));
    }
    assert!(second.ends_with(" (2).flac"));
}
