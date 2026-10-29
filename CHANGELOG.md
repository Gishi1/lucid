# Changelog

All notable changes to Lucid are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] — 2026-10-28

### Added

- Read-only CHM (Compiled HTML Help, Help 1.x) support: opens `.chm` archives directly,
  decodes `#SYSTEM` metadata, the HHC contents tree and the HHK keyword index, and exposes
  topics, assets, search and `ms-its:` / `mk:@MSITStore:` links like an MSHC package.
- Self-authored `testdata/demo-help.chm` fixture plus end-to-end CHM tests (metadata,
  TOC, index, links, assets, error handling).

### Notes

- CHM decoding uses a vendored copy of `libchm` 0.3.0 (MIT) with MSRV substitutions for
  Rust 1.86. CHM support is read-only; Lucid does not compile `.chm` files.

## [1.0.0] — 2026-10-27

First public release. The 0.1.0 milestone was internal.

### Added

- Native egui/eframe reader that opens a `.mshc`, a Help Library `ContentStore/<LOCALE>`
  folder or a `.msha` setup document and merges the content set into one library.
- Table of contents rebuilt from the manifest and from the `Microsoft.Help.*` meta tags.
- Native topic renderer: headings, lists, tables, code snippets with language tabs,
  collapsible areas, callouts, images and legacy SHFB/Sandcastle markup.
- Ranked search: BM25F field weights, phrases, prefixes, fuzzy fallback, exclusions,
  boolean union, field filters, snippets with exact highlights and spelling suggestions.
- Persisted search index under `$XDG_CACHE_HOME/lucid/index`, rebuilt with `F5`.
- Reader features: tabs, history, bookmarks, per-topic outline, in-topic find, zoom,
  dark/light themes and drag-and-drop loading.
- `lucid-forge`: deterministic MSHC packer, validator and outline printer.
- Headless CLI: `info`, `toc`, `search`, `extract`, `demo`, F1 keyword resolution and
  `--screenshot` mode for visual verification.
- Self-authored `testdata/demo-help.mshc` fixture so no third-party documentation is
  redistributed.

### Notes

- MSRV is Rust 1.86. `Cargo.lock` pins `image`, `zip` and the `icu_*` chain to versions
  that still build on it.
- Every crate is `#![forbid(unsafe_code)]`.
- Release archives are built for Linux x86_64, macOS x86_64/arm64 and Windows x86_64 by the
  `v*` tag workflow; GitLab CI mirrors the same set into its package registry.
