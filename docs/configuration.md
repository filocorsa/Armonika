# Configuration

Armonika reads its settings from three sources. A later source overrides an earlier one:

1. the defaults in this page;
2. a TOML file: `--config <path>`, otherwise the path in `ARMONIKA_CONFIG`, otherwise `armonika.toml` in the current directory if it exists (a path you name that does not exist is an error);
3. environment variables that start with `ARMONIKA_`. Levels of a key are separated by two underscores: `scan.threads` is `ARMONIKA_SCAN__THREADS`.

An unknown key in the file, or an unknown key inside a known section in the environment, is an error that names the key. An `ARMONIKA_*` variable that is neither a key nor one of the variables in the last section is ignored, with a warning in the log that names it (never its value).

## Keys

| Key | Default | Environment variable | Meaning |
| --- | --- | --- | --- |
| `data_dir` | `data` | `ARMONIKA_DATA_DIR` | Folder for the database, the search index, covers and the secret key file. Created if missing. |
| `server.bind` | `0.0.0.0:4747` | `ARMONIKA_SERVER__BIND` | Address and port the server listens on. |
| `web.enabled` | `true` | `ARMONIKA_WEB__ENABLED` | Serve the web interface at `/`. Set it to `false` for headless mode: `/` answers 404 and only the API under `/rest` is served. There is no web interface yet: for now `/` shows a one-line placeholder with the version. |
| `scan.watch` | `true` | `ARMONIKA_SCAN__WATCH` | Watch the library folders and scan a library when its files change. A folder that cannot be watched (a network share, for example) is left to the periodic scan. |
| `scan.interval` | `1h` | `ARMONIKA_SCAN__INTERVAL` | Periodic scan of every library, for example `30m` or `6h`. `0s` means never. |
| `scan.threads` | `4` | `ARMONIKA_SCAN__THREADS` | Threads that read files during a scan. |
| `library` | none | none (file only) | A library: `[[library]]` with `name` and `path`. Repeat the section for each library. A library that is not in the database yet is created at startup; one that exists is never changed or removed by the file. |
| `ffmpeg_path` | none (`ffmpeg` is searched in `PATH`) | `ARMONIKA_FFMPEG_PATH` | The ffmpeg program used to transcode. Streaming of the original file does not need it. |
| `transcode.format` | `mp3` | `ARMONIKA_TRANSCODE__FORMAT` | Format used when an app asks for transcoding without naming one. |
| `transcode.bitrate_kbps` | `192` | `ARMONIKA_TRANSCODE__BITRATE_KBPS` | Bitrate used when an app asks for transcoding without naming one. |
| `search.ignored_articles` | `The El La Los Las Le Les Il Lo Gli I` | `ARMONIKA_SEARCH__IGNORED_ARTICLES` | Words, separated by spaces, that are ignored at the start of artist names when sorting. |
| `listenbrainz.url` | `https://api.listenbrainz.org` | `ARMONIKA_LISTENBRAINZ__URL` | ListenBrainz server for scrobbles. Change it to use a compatible server. Plays are queued and sent in the background; a play still not accepted 14 days after it was played is dropped. |

The content hasher (the background job that computes a fingerprint of each track's audio so that moved and retagged files keep their identity) always runs; it has no setting.

Example:

```toml
data_dir = "/srv/armonika/data"

[server]
bind = "0.0.0.0:4747"

[scan]
interval = "6h"

[[library]]
name = "Main"
path = "/srv/music"
```

## The `inbox` folder is not active yet

The design describes an `inbox` folder for each library, where files copied in would be picked up and queued for review. **This is not implemented yet.** There is no `inbox` key, and files copied to a folder with that name are not picked up. Do not rely on it.

## Other variables and options

| Name | Meaning |
| --- | --- |
| `ARMONIKA_CONFIG` | Path of the configuration file (same as `--config`). |
| `ARMONIKA_ADMIN_USER`, `ARMONIKA_ADMIN_PASSWORD` | Create the first admin at startup when no admin exists. Ignored once an admin exists. The password is never logged. You can also run `armonika user create <name> --admin`. |
| `ARMONIKA_SECRET_KEY` | The key (32 bytes, base64) that encrypts the secrets the server must read back, such as app passwords and ListenBrainz tokens. Without it a key file `secret.key` is created in `data_dir`. Keep it private and do not lose it: secrets stored with another key cannot be read. |
| `RUST_LOG` | Log level, for example `debug` or `armonika_scanner=debug`. Default `info,symphonia=warn` (the audio decoder logs one line per file at `info`). |

## Empty library folders

If the folder of a library exists but holds no audio files while the library has tracks, Armonika assumes the volume is not mounted (with Docker, an unmounted bind mount looks like an empty folder). It marks nothing as missing and logs a warning that names the library. Folders that do not exist or cannot be read stop the scan with an error, as before. Deleting some files, or all but one, still marks them missing as usual.

If you emptied a library on purpose, run the scan by hand with the override:

```
armonika library scan <name> --allow-empty
```

The tracks then become missing, and `armonika track purge-missing` can delete them. The watcher and the periodic scan never use the override. `armonika library scan <name>` without the option scans a library once and refuses an empty folder in the same way. The search index is updated by the running server only, so restart it after a manual scan if search results look out of date.

## Docker

The image `ghcr.io/filocorsa/armonika` sets `ARMONIKA_DATA_DIR=/data` and `ARMONIKA_SERVER__BIND=0.0.0.0:4747`, runs as uid 1000 and has ffmpeg installed. Mount the data folder at `/data` and your music read-only (`:ro`, Armonika never writes to a library). If the music or the data folder belongs to another user, set `user:` in the compose file to that uid and gid. See `docker-compose.example.yml` for a complete example. The image checks its own health with `armonika healthcheck`, which asks the local server for `/rest/ping` without credentials and exits with 0 when the server answers `ok` or Subsonic error 10 (credentials missing), and with 1 otherwise.

Before the first `docker compose up`, create the `data` folder and `armonika.toml` next to the compose file: Docker creates a missing path as an empty folder owned by root, which the container cannot write to, and a missing `armonika.toml` becomes a folder. `docker stop` kills the process after 10 s by default; the example sets `stop_grace_period: 1m` so the server can finish its work first.

## Search after a crash

If the server was killed during a scan (power cut, `docker kill`, out of memory), search can miss the tracks of the last seconds of that scan. To repair it, stop the server and delete the folder `search` in `data_dir`: the next start rebuilds the index in the background.

## If you use Armonika for a commercial service

Armonika is released under the AGPL-3.0. The design document (section 7.2) says two things you must know before offering it as a commercial service:

- **The AGPL network clause.** The license allows building a service on Armonika, with the obligation to publish your modifications. Read the license text and ask a lawyer if you are unsure what it requires of your service.
- **Third-party services need your own agreements.** Armonika's recognition features use MusicBrainz and AcoustID. MusicBrainz data is free for non-commercial use, and commercial users must reach an agreement with the MetaBrainz Foundation. AcoustID is free for non-commercial applications, and commercial use goes through the paid plans of AcoustID OU. The AcoustID key that the project registers is for open source, non-commercial use. If you run a commercial service, use your own keys and agreements with both. These features are not part of the current version.
