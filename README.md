<p align="center">
  <img alt="Armonika" src="images/armonika-brand-kit/armonika-logo-banner.svg" width="480"><br>
  <strong>The lightweight self-hosted music server that puts your library in order.</strong>
</p>

## Status

Early development. The server runs: it scans libraries, searches, streams and speaks the OpenSubsonic API, but there is no web interface yet. Expect breaking changes.

Phase 0 (spikes and scaffolding) is done: the technical choices are made and measured, and CI runs on Linux, Windows and macOS. Phase 1 (listening foundations: library scan, search, streaming, OpenSubsonic API) is next.

## What it will be

A music server written in Rust that combines fast streaming to existing Subsonic apps with organizing the library from the browser: recognizing tracks with MusicBrainz and AcoustID, fixing tags, and renaming files, always with confirmation and undo. Optional integration with MeTube lets you add music from a link.

## How to try it

You need a recent stable Rust toolchain (see [Development notes](docs/development.md)).

```sh
cargo build --release
```

Create `armonika.toml` next to where you will run it:

```toml
[[library]]
name = "Main"
path = "/path/to/your/music"
```

Start the server, creating the first admin from the environment (the password needs at least 8 characters):

```sh
ARMONIKA_ADMIN_USER=admin ARMONIKA_ADMIN_PASSWORD=choose-a-password ./target/release/armonika serve
```

It listens on port 4747 and scans your library in the background. Point an OpenSubsonic app (Symfonium, Feishin, DSub and others) at `http://<your-server>:4747` with that user and password. Search is empty for a short time after the first start, until the index is built. Stop it with Ctrl+C: it finishes the work in progress first.

Every setting, with its default and environment variable, is in [Configuration](docs/configuration.md). Run `armonika --help` for the user, library and track commands. Files in your library are only read, never changed.

## Documentation

- [Design document](docs/superpowers/specs/2026-11-12-armonika-design.md) (Italian): goals, glossary, behavior, phases.
- [Spike results](docs/spikes/README.md) (Italian): the measurements behind the technical choices.
- [Configuration](docs/configuration.md): every setting, headless mode, notes for commercial use.
- [Development notes](docs/development.md): requirements, build, tests and lint.

## License

[AGPL-3.0](LICENSE).
