<p align="center">
  <img alt="Armonika" src="images/armonika-brand-kit/armonika-logo-banner.svg" width="480"><br>
  <strong>The lightweight self-hosted music server that puts your library in order.</strong>
</p>

Armonika is a music server written in Rust. It streams your own music to the Subsonic apps you already use, and it will grow into a tool that organizes the library from the browser: recognizing tracks, fixing tags and renaming files, always with confirmation and undo.

## Status

Early development, not yet stable. The server works today with OpenSubsonic apps, but there is no web interface yet: users and libraries are managed from the command line. Expect breaking changes until the first release.

## What it does today

- **Scans your libraries** fully, then incrementally, and watches the folders for changes. Supported formats: MP3, FLAC, Ogg Vorbis, Opus, M4A (AAC and ALAC), WAV and AIFF.
- **Keeps your data when files move.** Stars, ratings, play counts and playlist entries follow a file that you move, rename or retag outside Armonika. An unmounted disk is recognized and nothing is marked missing. Deleted files are only marked missing, with their data kept, and come back as they were if the file returns.
- **Searches fast and forgivingly**: typos, missing accents and partial titles still find the track, and Chinese, Japanese and Korean titles are searchable.
- **Streams** with seeking, and transcodes to MP3, Opus or AAC with ffmpeg when an app asks for a lower bitrate. Covers are read from the files or from the album folder.
- **Speaks the OpenSubsonic API**: browsing, search, stars, ratings, playlists, play queue sync across devices and now playing.
- **Several users and libraries**, with per-library access (listen or manage) and an optional ban on downloading original files.
- **Scrobbles to ListenBrainz**, with a queue that waits out network problems.
- **Is light**: a single binary of about 10 MB. On a test library of 100,000 tracks, search answers in about 20 ms (p95) and the idle server uses about 75 MB of RAM. Details in [Benchmarks](docs/benchmarks.md).

## Install with Docker

The image `ghcr.io/filocorsa/armonika:main` runs on amd64 and arm64 (for example a Raspberry Pi 4 or 5 with a 64-bit system) and includes ffmpeg.

1. Create a folder for Armonika and, inside it, the `data` folder and the configuration file. Create both before the first start: if they are missing, Docker creates them as folders owned by root and the server cannot start.

   ```sh
   mkdir armonika && cd armonika
   mkdir data
   cat > armonika.toml <<'EOF'
   [[library]]
   name = "Music"
   path = "/music"
   EOF
   ```

   On Windows, PowerShell has no `cat <<'EOF'`: create `armonika.toml` with a text editor and the same three lines.

2. Copy [docker-compose.example.yml](docker-compose.example.yml) to `docker-compose.yml` in the same folder. Replace `/path/to/your/music` with your music folder. The music is mounted read-only: Armonika never writes to a library.

3. Start it:

   ```sh
   docker compose up -d
   docker compose logs -f
   ```

   The first start scans the library in the background. Search returns results once the index is built, a short while after the scan. Until an admin exists, the log says so.

4. Create the admin user. The password (at least 8 characters) is asked twice and not shown:

   ```sh
   docker compose exec armonika armonika user create admin --admin
   ```

5. Point an OpenSubsonic app (for example Symfonium, Feishin or DSub) at `http://<your-server>:4747` and log in with the admin user.

The container runs as uid 1000. If your music or the `data` folder belong to another user, see the comments at the top of the compose file.

The container must be able to read every folder and file of the music. Music copied from another computer, or extracted from an archive, can arrive with folders that only their owner may open. If the library folder itself cannot be read, the scan stops with an error; a folder inside it that cannot be read is skipped, with a warning in the log. Make everything readable with:

```sh
chmod -R a+rX /path/to/your/music
```

### Proxmox

Docker runs inside an unprivileged LXC container once the container has the `nesting` and `keyctl` features enabled (Options, Features). Put the music on a mount point of the container and use its path inside the container in the compose file. In an unprivileged container, uid 1000 inside is uid 101000 on the Proxmox host: files written from the host must be readable by that uid, or by everyone as above.

### Users, libraries and ListenBrainz

Run the command-line tool inside the container:

```sh
# Add a user (asks for the password) and give access to a library
docker compose exec armonika armonika user create anna
docker compose exec armonika armonika user grant anna Music listen

# A user who may stream but not download original files
docker compose exec armonika armonika user create guest --no-download

# Link a ListenBrainz account (paste the user token from listenbrainz.org/settings)
docker compose exec armonika armonika user listenbrainz anna
```

Passwords and tokens are asked without showing them, so they do not end up in the shell history. Do not add `-T` to these commands: it removes the terminal they ask on. Scripts can pass them on standard input with `--password-stdin` or `--token-stdin`. ListenBrainz only accepts plays from accounts with a verified email address on MetaBrainz.

`armonika --help` lists every command, including adding more libraries, renaming them and scanning one now. To add a library, mount its folder in the compose file and add a `[[library]]` block to `armonika.toml`.

Every setting, with its default and environment variable, is in [Configuration](docs/configuration.md).

## Build from source

You need a recent stable Rust toolchain and, for transcoding, ffmpeg on the `PATH`. See [Development notes](docs/development.md).

```sh
cargo build --release
```

Create `armonika.toml` next to where you will run it, with one `[[library]]` block as above but with the real path of your music. From the same folder, create the admin (the password is asked) and start the server:

```sh
./target/release/armonika user create admin --admin
./target/release/armonika serve
```

It listens on port 4747. Stop it with Ctrl+C: it finishes the work in progress first.

## What comes next

- **Phase 2**: a web interface for users, libraries and settings, track recognition with MusicBrainz and AcoustID, tag editing, renaming and moving files, duplicate checks, all with undo.
- **Phase 3**: adding music from a link through [MeTube](https://github.com/alexta69/metube).

## Documentation

- [Configuration](docs/configuration.md): every setting, Docker notes, headless mode, notes for commercial use.
- [Benchmarks](docs/benchmarks.md): how the performance targets are measured, and the results.
- [Development notes](docs/development.md): requirements, build, tests and lint.
- [Design document](docs/superpowers/specs/2026-11-12-armonika-design.md) (Italian): goals, behavior, phases.
- [Spike results](docs/spikes/README.md) (Italian): the measurements behind the technical choices.

## Security

To report a vulnerability, see [SECURITY.md](SECURITY.md).

## License

[AGPL-3.0](LICENSE).
