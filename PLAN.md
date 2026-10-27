# PLAN — Lucid v0.1 implementation

Reference: [`SPEC.md`](SPEC.md) (requirements / acceptance) · [`docs/DESIGN.md`](docs/DESIGN.md)
(frozen APIs, format algorithm, UX).
Status legend: `[x]` done · `[~]` in progress · `[ ]` todo · `[!]` blocked

## Milestones

| # | Milestone | Deliverable | Exit criteria |
| --- | --- | --- | --- |
| M0 | Skeleton | workspace, 4 crates, docs, git repo | **done** — `cargo check` green |
| M1 | Format core | `lucid-mshc` | **done** — 77 unit + 18 integration tests green |
| M2 | Search engine | `lucid-search` | **done** — 70 tests green incl. 10k-doc perf test |
| M3 | Packer | `lucid-forge` + `testdata/demo-help.mshc` | **done** — 15 tests green, demo validates clean |
| M4 | Application | `lucid` GUI + headless CLI | **done** — acceptance 3 & 6 verified by CLI, 4 by GUI smoke test |
| M5 | Polish | README, release build, perf pass, initial commit | **done** (clippy/rustfmt unavailable in this toolchain, see below) |

## Workstreams

```
        ┌────────────── main: workspace, docs, lucid-mshc (M0,M1)
        │                                   │
t0 ─────┼─► subagent SEARCH: lucid-search (M2) ──┐
        │                                        ├─► main: lucid app (M4) ─► M5
        └─► subagent FORGE: lucid-forge (M3) ────┘
                     │
             main: lucid-mshc tests consume forge output (fixtures)
```

Interface stability: `docs/DESIGN.md` §4 (search API) and §5 (forge CLI + package layout) are
**frozen** before the subagents start; only additive changes are allowed afterwards.

## P0 — Skeleton (M0)

- [x] `git init` in CWD, repo-local identity `gishi <gishi@localhost>`
- [x] `Cargo.toml` workspace with pinned deps (rustc 1.86 compatible; egui/eframe 0.32.3 — pre-cached)
- [x] `docs/DESIGN.md` (format algorithm, frozen APIs, UX)
- [x] `SPEC.md`
- [x] `PLAN.md`
- [x] `.gitignore`, `LICENSE`, `README.md`
- [x] crate stubs: `crates/lucid-mshc`, `crates/lucid-search`, `crates/lucid-forge`, `crates/lucid`
- [x] `cargo check --workspace` (zero warnings)

## P1 — `lucid-mshc` (M1) — owner: main

- [ ] `error.rs` — `Error` enum (`thiserror`): Io, Zip, NotOpc, NotMshc, Xml, Utf8, NotFound, Cab
- [ ] `zipread.rs` — zip loading (from path **and** from bytes), entry table, name
      normalization (lowercase, `\`→`/`, strip leading `./`), lazy read by index, cached
      decompression for hot entries
- [ ] `meta.rs` — tolerant meta-tag extraction from XHTML (namespaced, case-insensitive,
      `<title>`, `<meta name=… content=…>`, `Title` meta alias, `Microsoft.Help.*` set)
- [ ] `topic.rs` — `Topic` model + body text extraction (headings, visible text, code text,
      anchor list, links) — feeds the search index, no HTML rendering here
- [ ] `toc.rs` — `TocNode` tree builder, manifest path + meta-tag path, synthetic grouping
      nodes, orphan handling, ordering (`TocOrder`, then title), stable titles
- [ ] `manifest.rs` — tolerant XML manifest reader (`roxmltree`): find manifest parts
      (OPC relationship → `*.msha` → any XML with TOC-ish nodes), accept
      `HelpTOCNode|HelpTOC|Topic|Node`, attributes `Id|Title|Url` in any case/namespace;
      read `HelpIndex`/`HelpKeywordIndex`/`HelpIndexEntry`
- [ ] `asset.rs` — case-insensitive asset resolution, URL decoding, `..` resolution,
      basename fallback, MIME by extension
- [ ] `package.rs` — `Package::open` (one `.mshc`), topic classification, keyword merging,
      locale, package title
- [ ] `docset.rs` — `DocSet::open_path` (file / folder / `.msha` setup / `.mshi` best-effort),
      multi-package merge, id map, duplicate resolution, link resolution
      (`ms-xhelp:///?Id=`, relative, anchors, cross-package), fingerprint for the index cache
- [ ] `export.rs` — self-contained topic/book export (rewrite asset URLs, copy assets)
- [ ] `tests/` — unit + integration tests built on `lucid-forge` generated fixtures
      (dev-dependency), incl. manifest-less and OPC-less variants, case-mismatch assets,
      duplicate ids, orphan topics, windows-1252 topic, BOM, empty package error path
- [ ] `benches/` or `tests/perf.rs` — open a synthetic 1 000-topic package and assert budget

## P2 — `lucid-search` (M2) — owner: subagent SEARCH (frozen API: DESIGN §4)

- [ ] tokenizer: Unicode word segmentation, case folding, camelCase/identifier splitting
      (`GetWindowRect` → `getwindowrect`, `get`, `window`, `rect`), CJK per-character
- [ ] stemmer (`rust-stemmers`, English) + optional stopword list
- [ ] `IndexBuilder`: per-field term frequencies, doc lengths, docs metas, `weight`
- [ ] `Index`: sorted term dictionary (binary-search prefix ranges), postings, BM25F scoring,
      boosts, deterministic tie-breaks, `scope` filtering
- [ ] query parser: AND/OR/`"phrase"`/`term*`/`term~`/`-exclude`/`field:` (DESIGN §4.1)
- [ ] snippets: window scoring, ellipses, char-index highlight ranges; code fallback
- [ ] suggestions: vocabulary lookup by edit distance/Jaro-Winkler (`strsim`), term + spelling kinds
- [ ] persistence: versioned bincode `to_bytes`/`from_bytes`/`save`/`load` + round-trip test
- [ ] tests: ranking sanity, phrase adjacency, prefix, fuzzy, exclusion, field filters,
      snippet ranges point at real matches, persistence round-trip, 10 k-doc perf smoke

## P3 — `lucid-forge` (M3) — owner: subagent FORGE (frozen CLI/layout: DESIGN §5)

- [ ] lib: `build_package(input_dir, out, opts)` — copy tree, scan topics, synthesize manifest
      TOC from `TocParent`/`TocOrder`, write `[Content_Types].xml`, `_rels/.rels`, `<name>.msha`
- [ ] lib: `demo_package()` — self-authored content covering every behaviour (DESIGN §5)
      incl. hand-encoded `.bmp` asset (no deps), nested TOC, orphan topic, anchors, `ms-xhelp://`
- [ ] bin: `build`, `demo`, `validate`, `outline` subcommands + `--help` text
- [ ] tests: validate(demo) clean; validate catches dangling URL/duplicate id; rebuild is
      deterministic (byte-stable except timestamps); `--no-manifest` / `--no-opc` variants
- [ ] artifact: `testdata/demo-help.mshc` written by `lucid demo`/`lucid-forge demo` and committed

## P4 — `lucid` application (M4) — owner: main

- [ ] `cli.rs` — argument parsing (hand-rolled, no clap): GUI open, `--topic`, `--search`,
      `info`/`toc`/`search`/`extract`/`demo`, `--version`, `--help`, exit codes per DESIGN §6
- [ ] `state.rs` — `AppState` persistence (recent, bookmarks, history, theme, font size,
      pane widths, TOC expansion, last topic, preferred code language)
- [ ] `theme.rs` — modern dark/light `Visuals` (surface tokens, corner radius, spacing, font sizes)
- [ ] `worker.rs` — background load/index threads + progress channel, cancellable reload
- [ ] `panes/contents.rs` — TOC tree, filter, reveal active, keyboard nav
- [ ] `panes/index.rs` — keyword list + instant filter + multi-select open-all + copy
- [ ] `panes/search.rs` — live search, filter chips, scope/sort, result list with highlights,
      suggestions, timing footer
- [ ] `panes/favorites.rs` — bookmark folders + history
- [ ] `quickopen.rs` — Ctrl+P fuzzy topic-title jump
- [ ] `view/tabs.rs` — tab bar, close/close-all, per-tab scroll memory
- [ ] `view/render.rs` — XHTML → egui block model + draw (headings, lists, tables, code,
      callouts, collapsibles, images, anchors, link hit-testing, zoom)
- [ ] `view/find.rs` — in-topic find bar (highlight all, next/prev, counter) — shares the
      tokenizer with the search crate
- [ ] `view/outline.rs` — "on this page" headings outline with current-section tracking
- [ ] `status.rs` — package/topic/index status, cache-hit indicator, query timing
- [ ] smoke test: `lucid info/toc/search` against `testdata/demo-help.mshc` in CI-style test

## P5 — Polish & ship (M5) — owner: main

- [ ] `README.md` final pass (features, install, screenshots-to-be, shortcuts, CLI, format notes)
- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] perf pass: measure `lucid search` p95 on the demo corpus + a synthetic 10 k-topic set
- [ ] initial git commit(s) with a summary of the format research
- [ ] roadmap items triaged into GitHub issues text file (`docs/ROADMAP.md`, optional)

## Verification playbook (per milestone)

1. `cargo check --workspace` → `cargo test -p <crate>` → `cargo clippy -p <crate> --all-targets`
2. Deterministic artifacts: build `testdata/demo-help.mshc` twice, compare hashes (must match).
3. Headless end-to-end on the demo package: `lucid info`, `lucid toc --depth 3`,
   `lucid search "phrase"`, `lucid search "title:install"`, `lucid search "no-such-term"`
   (expect suggestions), `lucid extract -o /tmp/opencode/lucid-extract`.
4. GUI manual checklist (SPEC §8.4): open by arg, drag & drop, TOC filter, index filter,
   search-as-you-type, phrase search, fuzzy fallback, result highlighting, tab close,
   history back/forward, bookmark, outline jump, find-in-topic, zoom, theme toggle,
   reopen-from-recent (index cache hit).
5. Negative paths: open a non-zip file, a zip without topics, a `.cab`, a folder with no
   `.mshc` → friendly errors, no panic.

## Risks & mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| `.msha`-in-package schema undocumented | TOC wrong for some packages | meta-tag TOC is authoritative; tolerant XML reader; DESIGN §2.4 cross-check; A-3 open question |
| egui 0.32 API drift vs documentation available | compile churn | pin cached versions, iterate on compiler errors, keep API surface small and centralised (`theme.rs`, `view/render.rs`) |
| HTML in topics is not well-formed XML | parser failures | `scraper`/html5ever tokenizer (never validates), tolerant meta scan as fallback |
| Large content sets blow memory | OOM/slow | lazy topic reads, metadata-only pass, `Arc` index, no per-topic HTML cache beyond the open tabs |
| No real-world `.mshc` sample available in this environment | false confidence | generate fixtures that mirror documented structures (incl. uppercase asset refs, missing OPC parts); keep `lucid info` around for user-supplied packages; ask the user to run it against a real file |
| Long first build (egui) | slow iteration | incremental dev profile (`opt-level=1`, deps at 2), build once and keep the target dir |

## Session log

- **S2** — implemented all four crates and verified them end to end.
  - **Delegation:** both background subagents (search, packer) were killed twice by
    infrastructure timeouts mid-write. `lucid-forge` had already produced a complete, green
    crate (kept as-is, 15 tests). `lucid-search` was left in a mixed state (my `index.rs`/
    `lib.rs`/`query.rs`/`rank.rs` + their `tokenizer.rs`/`snippet.rs`/`error.rs`); the useful
    halves were merged (their richer generator-style tokenizer and snippet builder, my index
    and query engine) and `persist.rs` was dropped in favour of persistence inside `index.rs`.
  - **Toolchain:** `cargo clippy`/`cargo fmt` are not installed in this nix rustc, so the
    quality gate used instead: `cargo check --workspace --all-targets` (zero warnings) plus
    the full test suite. The `icu_*` crates were pinned to 2.2.0 in `Cargo.lock` because
    `eframe → egui-winit/links → webbrowser → url → idna → icu_* 2.4` requires rustc 1.88
    while this machine has 1.86 (2.2.0 is the newest release that still supports 1.86).
  - **Bugs the tests caught** (all fixed, each with a regression test): query group order
    (`-x` before a pending group), quoted terms becoming prefixes while typing, phrase
    adjacency enforced only as a boost instead of a requirement, OR-alternatives summing
    their scores (now best-per-condition), stem expansion only running as a last resort
    (broke `title:install` when the exact word lived in another field), snippet windows
    overshooting `max_chars` by the leading context unit, code-only matches showing body
    snippets, TOC merge duplicating a topic when manifest and meta tags disagree, inline
    `<code>` runs gluing words together, and a byte-slice panic in the HTML exporter.
  - **Verification:** `cargo test --workspace` → 222 tests green; `lucid --help`,
    `lucid info|toc|search|extract|demo` exercised against `testdata/demo-help.mshc`
    (search-as-you-type prefixes, `"phrases"`, `title:`/`kw:`/`code:` filters, exclusions,
    "did you mean" on a typo with exit code 3, cache hit on the second run);
    `cargo build --release -p lucid` succeeded; the GUI was launched on the demo package and
    stayed up (Wayland + OpenGL), which covers startup, package load, index build and
    rendering.
  - **Left for the next session:** interactive GUI checklist (bookmarks, tabs, find-in-topic
    by hand), cross-checking a package in Microsoft Help Viewer on Windows (impossible here),
    syntax highlighting for code snippets, `.cab`/CHM import, and packaging (Nix flake/deb).

- **S1** — repo created; format research done (MS Help Viewer SDK content-metadata reference,
  10Tec product pages, `andlabs/mshc2devhelp` reference parser: topics are `.htm` with
  `Microsoft.Help.Id|TocParent|TocOrder|Keywords|F1`, assets referenced with **uppercase**
  paths while zip entries are lowercase, non-topic parts start with `ic`); workspace +
  DESIGN/SPEC/PLAN written.
