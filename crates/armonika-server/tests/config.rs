// `Jail::expect_with` closures must return `figment::Result`, whose error
// type is large; that signature is fixed by figment.
#![allow(clippy::result_large_err)]

use std::time::Duration;

use armonika_server::config::Config;
use figment::Jail;

#[test]
fn defaults_without_file() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let c = Config::load(None).unwrap();
        assert_eq!(c.server.bind.to_string(), "0.0.0.0:4747");
        assert_eq!(c.data_dir.to_str(), Some("data"));
        assert_eq!(c.scan.interval, Duration::from_secs(3600));
        assert!(c.web.enabled && c.scan.watch);
        assert_eq!(c.scan.threads, 4);
        assert_eq!(c.transcode.format, "mp3");
        assert_eq!(c.transcode.bitrate_kbps, 192);
        assert_eq!(c.listenbrainz.url, "https://api.listenbrainz.org");
        assert_eq!(
            c.search.ignored_articles,
            "The El La Los Las Le Les Il Lo Gli I"
        );
        assert!(c.libraries.is_empty() && c.ffmpeg_path.is_none());
        Ok(())
    });
}

#[test]
fn file_overrides_defaults() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.create_file(
            "armonika.toml",
            "data_dir = \"/srv/a\"\n[server]\nbind = \"127.0.0.1:1234\"\n[scan]\ninterval = \"30m\"\n",
        )?;
        let c = Config::load(None).unwrap();
        assert_eq!(c.server.bind.to_string(), "127.0.0.1:1234");
        assert_eq!(c.data_dir.to_str(), Some("/srv/a"));
        assert_eq!(c.scan.interval, Duration::from_secs(1800));
        assert_eq!(c.scan.threads, 4);
        Ok(())
    });
}

#[test]
fn env_overrides_file() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.create_file("armonika.toml", "[server]\nbind = \"127.0.0.1:1234\"\n")?;
        jail.set_env("ARMONIKA_SERVER__BIND", "127.0.0.1:9000");
        jail.set_env("ARMONIKA_DATA_DIR", "/tmp/x");
        let c = Config::load(None).unwrap();
        assert_eq!(c.server.bind.to_string(), "127.0.0.1:9000");
        assert_eq!(c.data_dir.to_str(), Some("/tmp/x"));
        Ok(())
    });
}

#[test]
fn libraries_from_toml() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.create_file(
            "armonika.toml",
            "[[library]]\nname = \"Main\"\npath = \"/music\"\n[[library]]\nname = \"Jazz\"\npath = \"/jazz\"\n",
        )?;
        let c = Config::load(None).unwrap();
        assert_eq!(c.libraries.len(), 2);
        assert_eq!(c.libraries[0].name, "Main");
        assert_eq!(c.libraries[1].path.to_str(), Some("/jazz"));
        Ok(())
    });
}

#[test]
fn unknown_key_is_rejected() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.create_file("armonika.toml", "[server]\nbogus_key = 1\n")?;
        let err = format!("{:#}", Config::load(None).unwrap_err());
        assert!(err.contains("bogus_key"), "{err}");
        Ok(())
    });
}

#[test]
fn unrelated_env_vars_are_ignored() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        // Documented in docs/development.md for the recognition tools.
        jail.set_env("ARMONIKA_ACOUSTID_KEY", "k");
        jail.set_env("ARMONIKA_SEARCH_DATASET", "realistic");
        let c = Config::load(None).unwrap();
        assert_eq!(c, Config::default());
        Ok(())
    });
}

#[test]
fn unknown_env_key_in_known_section_is_rejected() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("ARMONIKA_SERVER__BOGUS", "1");
        let err = format!("{:#}", Config::load(None).unwrap_err());
        assert!(err.contains("bogus"), "{err}");
        Ok(())
    });
}

#[test]
fn explicit_missing_file_is_an_error() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        let err = Config::load(Some(std::path::Path::new("missing.toml"))).unwrap_err();
        assert!(
            err.to_string()
                .contains("configuration file not found: missing.toml"),
            "{err:#}"
        );
        Ok(())
    });
}

#[test]
fn reserved_env_vars_do_not_break_config() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.set_env("ARMONIKA_ADMIN_USER", "admin");
        jail.set_env("ARMONIKA_ADMIN_PASSWORD", "pw");
        jail.set_env("ARMONIKA_SECRET_KEY", "k");
        jail.create_file("custom.toml", "[web]\nenabled = false\n")?;
        jail.set_env("ARMONIKA_CONFIG", "custom.toml");
        let c = Config::load(None).unwrap();
        assert!(!c.web.enabled);
        Ok(())
    });
}

#[test]
fn hash_on_start_is_no_longer_a_key() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        jail.create_file(
            "armonika.toml",
            "[scan]
hash_on_start = true
",
        )?;
        let err = format!("{:#}", Config::load(None).unwrap_err());
        assert!(err.contains("hash_on_start"), "{err}");
        Ok(())
    });
}

#[test]
fn recognition_section_defaults_and_env() {
    Jail::expect_with(|jail| {
        jail.clear_env();
        assert!(
            Config::load(None)
                .unwrap()
                .recognition
                .background_fingerprint
        );

        jail.create_file(
            "armonika.toml",
            "[recognition]\nbackground_fingerprint = true\n",
        )?;
        jail.set_env("ARMONIKA_RECOGNITION__BACKGROUND_FINGERPRINT", "false");
        let c = Config::load(None).unwrap();
        assert!(!c.recognition.background_fingerprint);
        Ok(())
    });
}
