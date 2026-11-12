<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="images/armonika-brand-kit/armonika-logo-dark.svg">
    <img alt="Armonika" src="images/armonika-brand-kit/armonika-logo-light.svg" width="420">
  </picture>
</p>

<p align="center"><strong>The lightweight self-hosted music server that puts your library in order.</strong></p>

## Status

Early development. **Armonika is not usable yet.**

Phase 0 (spikes and scaffolding) is done: the technical choices are made and measured, and CI runs on Linux, Windows and macOS. Phase 1 (listening foundations: library scan, search, streaming, OpenSubsonic API) is next.

## What it will be

A music server written in Rust that combines fast streaming to existing Subsonic apps with organizing the library from the browser: recognizing tracks with MusicBrainz and AcoustID, fixing tags, and renaming files, always with confirmation and undo. Optional integration with MeTube lets you add music from a link.

## Documentation

- [Design document](docs/superpowers/specs/2026-11-12-armonika-design.md) (Italian): goals, glossary, behavior, phases.
- [Spike results](docs/spikes/README.md) (Italian): the measurements behind the technical choices.
- [Development notes](docs/development.md): requirements, build, tests and lint.

## License

[AGPL-3.0](LICENSE).
