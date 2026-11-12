//! Name generation, path building and file name safety rules.
//!
//! The path scheme follows the default naming of the design (par. 8.6):
//! `Album artist/Album (Year)/NN - Title.ext`, with the special cases for
//! compilations (`NN - Track artist - Title.ext`), multi-disc editions
//! (`Disc N` folder) and files without an album (`Track artist/Singles/Title.ext`).
//! Relative paths are always built with `/` separators; the generator stores them
//! that way in the manifest so it is byte-identical on every platform.

use std::collections::HashSet;

use rand::RngExt;
use rand_chacha::ChaCha8Rng;

/// Longest allowed single path component, in characters.
pub const MAX_COMPONENT_CHARS: usize = 60;
/// Longest allowed relative path, in characters.
pub const MAX_PATH_CHARS: usize = 200;
/// Album artist used for compilations.
pub const VARIOUS_ARTISTS: &str = "Various Artists";

const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

fn is_reserved_stem(stem: &str) -> bool {
    let up = stem.trim_end().to_ascii_uppercase();
    if RESERVED.contains(&up.as_str()) {
        return true;
    }
    for prefix in ["COM", "LPT"] {
        if let Some(rest) = up.strip_prefix(prefix)
            && rest.len() == 1
            && matches!(rest.as_bytes()[0], b'1'..=b'9')
        {
            return true;
        }
    }
    false
}

fn trim_end_bad(s: &str) -> &str {
    s.trim_end_matches(['.', ' '])
}

/// Sanitize one path component for Windows, macOS and Linux and cut it to `max` chars.
fn sanitize_limit(s: &str, max: usize) -> String {
    let mapped: String = s
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = trim_end_bad(mapped.trim_start());
    let cut: String = trimmed.chars().take(max).collect();
    let mut out = trim_end_bad(&cut).to_string();
    if out.is_empty() {
        out.push('_');
    }
    let stem = out.split('.').next().unwrap_or("");
    if is_reserved_stem(stem) {
        out.insert(0, '_');
        let cut: String = out.chars().take(max.max(1)).collect();
        out = trim_end_bad(&cut).to_string();
        if out.is_empty() {
            out.push('_');
        }
    }
    out
}

/// Make `s` a safe path component: forbidden characters and control characters become `_`,
/// no trailing dot or space, Windows reserved names are altered, empty becomes `_`, and the
/// result has at most 60 characters (cut on character boundaries).
pub fn sanitize_component(s: &str) -> String {
    sanitize_limit(s, MAX_COMPONENT_CHARS)
}

/// The fields that determine where a file lands in the library tree.
#[derive(Debug, Clone, Copy)]
pub struct PathParts<'a> {
    pub album_artist: Option<&'a str>,
    pub track_artist: &'a str,
    pub album: Option<&'a str>,
    pub year: u32,
    pub disc: u32,
    pub multi_disc: bool,
    pub track: u32,
    pub title: &'a str,
    pub extension: &'a str,
}

/// Build the relative path (always `/` separated) with the default limits.
pub fn build_relative_path(parts: &PathParts) -> String {
    build_relative_path_with_limit(parts, MAX_PATH_CHARS)
}

fn clen(s: &str) -> usize {
    s.chars().count()
}

/// Same as [`build_relative_path`] with a custom limit on the whole path. When the path is
/// too long the longest components are shortened one character at a time.
pub fn build_relative_path_with_limit(parts: &PathParts, max_path: usize) -> String {
    let ext = parts.extension;
    let ext_len = 1 + clen(ext);
    let stem_max = MAX_COMPONENT_CHARS.saturating_sub(ext_len).max(1);
    let mut dirs: Vec<String> = Vec::new();
    let stem_raw: String = match parts.album {
        None => {
            dirs.push(sanitize_component(parts.track_artist));
            dirs.push("Singles".to_string());
            parts.title.to_string()
        }
        Some(album) => {
            let various = parts.album_artist == Some(VARIOUS_ARTISTS);
            let artist = parts.album_artist.unwrap_or(parts.track_artist);
            dirs.push(sanitize_component(artist));
            let year_suffix = format!(" ({})", parts.year);
            let budget = MAX_COMPONENT_CHARS.saturating_sub(clen(&year_suffix));
            dirs.push(format!("{}{}", sanitize_limit(album, budget), year_suffix));
            if parts.multi_disc {
                dirs.push(format!("Disc {}", parts.disc));
            }
            if various {
                let who: String = parts.track_artist.chars().take(25).collect();
                format!("{:02} - {} - {}", parts.track, who, parts.title)
            } else {
                format!("{:02} - {}", parts.track, parts.title)
            }
        }
    };
    let mut stem = sanitize_limit(&stem_raw, stem_max);
    let total = |dirs: &[String], stem: &str| {
        dirs.iter().map(|d| clen(d) + 1).sum::<usize>() + clen(stem) + ext_len
    };
    while total(&dirs, &stem) > max_path {
        // Find the longest component; the file stem counts with its extension.
        let mut best: Option<(usize, usize)> = None; // (index, len); dirs.len() is the stem
        for (i, d) in dirs.iter().enumerate() {
            let l = clen(d);
            if l > 1 && best.is_none_or(|(_, bl)| l > bl) {
                best = Some((i, l));
            }
        }
        let sl = clen(&stem) + ext_len;
        if clen(&stem) > 1 && best.is_none_or(|(_, bl)| sl > bl) {
            best = Some((dirs.len(), sl));
        }
        let Some((idx, _)) = best else { break };
        if idx == dirs.len() {
            let n = clen(&stem) - 1;
            stem = sanitize_limit(&stem, n);
        } else {
            let n = clen(&dirs[idx]) - 1;
            dirs[idx] = sanitize_limit(&dirs[idx], n);
        }
    }
    let mut out = String::new();
    for d in &dirs {
        out.push_str(d);
        out.push('/');
    }
    out.push_str(&stem);
    out.push('.');
    out.push_str(ext);
    out
}

/// Hands out unique relative paths. Paths are compared in lowercase because Windows and
/// macOS file systems are case-insensitive; a clash gets ` (2)`, ` (3)`, ... before the
/// extension. Nothing is ever handed out twice.
#[derive(Debug, Default)]
pub struct PathAllocator {
    used: HashSet<String>,
}

impl PathAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(&mut self, rel: String) -> String {
        if self.used.insert(rel.to_lowercase()) {
            return rel;
        }
        let (dir, file) = match rel.rfind('/') {
            Some(i) => (&rel[..=i], &rel[i + 1..]),
            None => ("", rel.as_str()),
        };
        let (stem, ext) = match file.rfind('.') {
            Some(i) if i > 0 => (&file[..i], &file[i..]),
            _ => (file, ""),
        };
        let mut n = 2u32;
        loop {
            let suffix = format!(" ({n})");
            let tail = clen(&suffix) + clen(ext);
            let room = MAX_COMPONENT_CHARS
                .saturating_sub(tail)
                .min(MAX_PATH_CHARS.saturating_sub(clen(dir) + tail))
                .max(1);
            let short: String = stem.chars().take(room).collect();
            let candidate = format!("{dir}{}{suffix}{ext}", trim_end_bad(&short));
            if self.used.insert(candidate.to_lowercase()) {
                return candidate;
            }
            n += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic name generation
// ---------------------------------------------------------------------------

const ADJECTIVES: &[&str] = &[
    "Silent",
    "Golden",
    "Broken",
    "Electric",
    "Midnight",
    "Crimson",
    "Hollow",
    "Gentle",
    "Wild",
    "Distant",
    "Frozen",
    "Burning",
    "Lonely",
    "Velvet",
    "Rusty",
    "Bright",
    "Shallow",
    "Ancient",
    "Neon",
    "Quiet",
    "Savage",
    "Tender",
    "Restless",
    "Paper",
    "Crystal",
    "Iron",
    "Scarlet",
    "Faded",
    "Wandering",
    "Secret",
    "Salt",
    "Winter",
    "Summer",
    "Autumn",
    "Lucky",
    "Blue",
    "Dusty",
    "Cosmic",
    "Sleepy",
    "Brave",
];

const NOUNS: &[&str] = &[
    "River",
    "Mirror",
    "Garden",
    "Engine",
    "Harbor",
    "Shadow",
    "Thunder",
    "Letter",
    "Window",
    "Compass",
    "Lantern",
    "Bridge",
    "Ocean",
    "Forest",
    "Signal",
    "Castle",
    "Ribbon",
    "Desert",
    "Meadow",
    "Orchard",
    "Station",
    "Horizon",
    "Anchor",
    "Carousel",
    "Echo",
    "Machine",
    "Island",
    "Tower",
    "Season",
    "Highway",
    "Cathedral",
    "Satellite",
    "Kingdom",
    "Memory",
    "Whisper",
    "Parade",
    "Voyage",
    "Ember",
    "Canyon",
    "Harvest",
];

const FIRST_NAMES: &[&str] = &[
    "Anna", "Marco", "Elena", "Jonas", "Clara", "Luca", "Mira", "Oskar", "Nina", "Felix", "Iris",
    "Tomas", "Greta", "Hugo", "Lena", "Victor", "Sofia", "Emil", "Ruth", "Ivan", "Dora", "Leo",
    "Edith", "Pablo", "Vera", "Axel", "Maya", "Bruno", "Alma", "Otto",
];

const LAST_NAMES: &[&str] = &[
    "Berg", "Rossi", "Novak", "Keller", "Moreau", "Silva", "Larsen", "Weber", "Costa", "Fischer",
    "Romano", "Dubois", "Horvat", "Lindgren", "Ferrari", "Kovac", "Bianchi", "Schmidt", "Ortega",
    "Petrov", "Marino", "Hansen", "Lopez", "Greco", "Vogel", "Conti", "Bauer", "Neri", "Fontana",
    "Baker",
];

const VERBS: &[&str] = &[
    "Chasing",
    "Leaving",
    "Holding",
    "Finding",
    "Burning",
    "Dreaming",
    "Waiting",
    "Falling",
    "Running",
    "Calling",
    "Crossing",
    "Counting",
    "Following",
    "Breaking",
    "Remembering",
    "Dancing",
    "Turning",
    "Watching",
];

const ACCENTED: &[&str] = &[
    "Caf\u{e9} del Mar",
    "Bj\u{f6}rk\u{e4}n",
    "Se\u{f1}or Coconut",
    "\u{c9}toile du Nord",
    "Motley Cr\u{fc}e",
    "Beyonc\u{e9}s",
    "Sigur R\u{f3}s",
    "\u{c5}ngstr\u{f6}m",
    "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}",
    "Cr\u{e8}me Br\u{fb}l\u{e9}e",
    "Na\u{ef}ve Fa\u{e7}ade",
    "\u{d8}stergade",
    "Dvo\u{159}\u{e1}k",
    "S\u{e3}o Paulo",
    "\u{1e62}ara\u{1e63}vat\u{12b}",
];

const CJK: &[&str] = &[
    "\u{6771}\u{4eac}\u{306e}\u{591c}",
    "\u{5fc3}\u{306e}\u{65c5}",
    "\u{6708}\u{5149}",
    "\u{5927}\u{6d77}\u{306e}\u{6b4c}",
    "\u{30b5}\u{30e0}\u{30e9}\u{30a4}\u{30fb}\u{30d6}\u{30eb}\u{30fc}\u{30b9}",
    "\u{ba85}\u{c0c1}\u{c758} \u{bc24}",
    "\u{6c38}\u{9060}\u{7684}\u{590f}\u{5929}",
    "\u{6843}\u{82b1}\u{6e90}",
    "\u{3055}\u{304f}\u{3089}\u{3055}\u{304f}\u{3089}",
    "\u{5922}\u{306e}\u{5f8c}",
    "\u{ae38}\u{c704}\u{c758} \u{ceec}\u{b7ec}",
    "\u{7e41}\u{661f}\u{6eff}\u{5929}",
];

fn pick<'a>(rng: &mut ChaCha8Rng, list: &[&'a str]) -> &'a str {
    list[rng.random_range(0..list.len())]
}

/// 10% of all generated names use accents or CJK characters.
fn exotic(rng: &mut ChaCha8Rng) -> Option<String> {
    if rng.random_bool(0.10) {
        let base = if rng.random_bool(0.5) {
            pick(rng, ACCENTED)
        } else {
            pick(rng, CJK)
        };
        // A short numeric tail keeps the exotic names from repeating too much.
        if rng.random_bool(0.5) {
            Some(format!("{base} {}", rng.random_range(2..100)))
        } else {
            Some(base.to_string())
        }
    } else {
        None
    }
}

/// An artist name such as `Anna Berg` or `The Silent River`.
pub fn artist_name(rng: &mut ChaCha8Rng) -> String {
    if let Some(n) = exotic(rng) {
        return n;
    }
    match rng.random_range(0..3) {
        0 => format!("{} {}", pick(rng, FIRST_NAMES), pick(rng, LAST_NAMES)),
        1 => format!("The {} {}", pick(rng, ADJECTIVES), pick(rng, NOUNS)),
        _ => format!("{} {}", pick(rng, ADJECTIVES), pick(rng, NOUNS)),
    }
}

/// An album title.
pub fn album_title(rng: &mut ChaCha8Rng) -> String {
    if let Some(n) = exotic(rng) {
        return n;
    }
    match rng.random_range(0..3) {
        0 => format!("{} {}", pick(rng, ADJECTIVES), pick(rng, NOUNS)),
        1 => format!("{} the {}", pick(rng, VERBS), pick(rng, NOUNS)),
        _ => format!("{} of {}", pick(rng, NOUNS), pick(rng, NOUNS)),
    }
}

/// A track title. Some titles contain characters that are illegal in file names on purpose,
/// so the path building rules get exercised on every run.
pub fn track_title(rng: &mut ChaCha8Rng) -> String {
    if let Some(n) = exotic(rng) {
        return n;
    }
    let base = match rng.random_range(0..4) {
        0 => format!("{} {}", pick(rng, ADJECTIVES), pick(rng, NOUNS)),
        1 => format!("{} the {}", pick(rng, VERBS), pick(rng, NOUNS)),
        2 => format!(
            "{} {} {}",
            pick(rng, VERBS),
            pick(rng, ADJECTIVES),
            pick(rng, NOUNS)
        ),
        _ => format!("The {} {}", pick(rng, NOUNS), pick(rng, VERBS)),
    };
    match rng.random_range(0..60) {
        0 => format!("{base}?"),
        1 => format!("{base}: Part {}", rng.random_range(1..4)),
        2 => format!("{base} (Live)"),
        3 => format!("{base} / Reprise"),
        _ => base,
    }
}
