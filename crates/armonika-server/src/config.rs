//! Configuration: defaults, then a TOML file, then `ARMONIKA_*` variables.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

/// Variables with the `ARMONIKA_` prefix that other code reads directly.
/// They are not configuration keys and must not reach the strict parser.
/// The [`TOP_LEVEL_KEYS`] filter already excludes them; this list is defense
/// in depth in case a future section shares a name with one of them.
const RESERVED_ENV: &[&str] = &["admin_user", "admin_password", "secret_key", "config"];

/// Top-level keys of [`Config`]. Only `ARMONIKA_*` variables whose first
/// level is one of these are configuration; others (for example
/// `ARMONIKA_ACOUSTID_KEY`, used by development tools) are left alone.
/// Unknown keys inside a known section are still rejected.
const TOP_LEVEL_KEYS: &[&str] = &[
    "data_dir",
    "server",
    "web",
    "scan",
    "library",
    "ffmpeg_path",
    "transcode",
    "search",
    "listenbrainz",
];

const DEFAULT_FILE: &str = "armonika.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub data_dir: PathBuf,
    pub server: ServerConfig,
    pub web: WebConfig,
    pub scan: ScanConfig,
    #[serde(rename = "library")]
    pub libraries: Vec<LibraryConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ffmpeg_path: Option<PathBuf>,
    pub transcode: TranscodeConfig,
    pub search: SearchConfig,
    pub listenbrainz: ListenBrainzConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServerConfig {
    pub bind: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct WebConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScanConfig {
    pub watch: bool,
    /// Periodic rescan interval; zero means never.
    #[serde(with = "humantime_serde")]
    pub interval: Duration,
    pub threads: usize,
    pub hash_on_start: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryConfig {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TranscodeConfig {
    pub format: String,
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SearchConfig {
    pub ignored_articles: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ListenBrainzConfig {
    pub url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data"),
            server: ServerConfig::default(),
            web: WebConfig::default(),
            scan: ScanConfig::default(),
            libraries: Vec::new(),
            ffmpeg_path: None,
            transcode: TranscodeConfig::default(),
            search: SearchConfig::default(),
            listenbrainz: ListenBrainzConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], 4747)),
        }
    }
}

impl Default for WebConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            watch: true,
            interval: Duration::from_secs(3600),
            threads: 4,
            hash_on_start: true,
        }
    }
}

impl Default for TranscodeConfig {
    fn default() -> Self {
        Self {
            format: "mp3".to_string(),
            bitrate_kbps: 192,
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            ignored_articles: "The El La Los Las Le Les Il Lo Gli I".to_string(),
        }
    }
}

impl Default for ListenBrainzConfig {
    fn default() -> Self {
        Self {
            url: "https://api.listenbrainz.org".to_string(),
        }
    }
}

impl Config {
    /// Loads the configuration. `path` is the `--config` value; without it
    /// `ARMONIKA_CONFIG` is used, then `armonika.toml` if it exists.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Config> {
        let mut figment = Figment::from(Serialized::defaults(Config::default()));

        let file = match path {
            Some(p) => Some(p.to_path_buf()),
            None => match std::env::var_os("ARMONIKA_CONFIG") {
                Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
                _ => {
                    let default = PathBuf::from(DEFAULT_FILE);
                    default.is_file().then_some(default)
                }
            },
        };
        if let Some(file) = file {
            if !file.is_file() {
                anyhow::bail!("configuration file not found: {}", file.display());
            }
            figment = figment.merge(Toml::file(file));
        }

        warn_ignored_env();
        let env = Env::prefixed("ARMONIKA_")
            .ignore(RESERVED_ENV)
            .split("__")
            .filter(|key| {
                let top = key.as_str().split('.').next().unwrap_or_default();
                TOP_LEVEL_KEYS.iter().any(|k| k.eq_ignore_ascii_case(top))
            });
        figment = figment.merge(env);

        figment.extract().context("invalid configuration")
    }
}

/// Logs the name (never the value) of each `ARMONIKA_*` variable that is
/// neither configuration nor reserved, so a typo does not go unnoticed.
fn warn_ignored_env() {
    for (name, _) in std::env::vars_os() {
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_prefix("ARMONIKA_") else {
            continue;
        };
        let top = rest.split("__").next().unwrap_or_default();
        let known = TOP_LEVEL_KEYS.iter().any(|k| k.eq_ignore_ascii_case(top))
            || RESERVED_ENV.iter().any(|k| k.eq_ignore_ascii_case(rest));
        if !known {
            tracing::warn!(
                variable = name,
                "ignoring environment variable: not a configuration key"
            );
        }
    }
}
