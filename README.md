# Lucid

**A fast, modern reader — and packer — for Microsoft Help Container (`.mshc`) documentation.**

Lucid opens the offline help format that Visual Studio ships (`MSHC` / *MS Help 3*, the format
described on [10tec.com/net-documentation-tool/microsoft-help-viewer-mshc.aspx](https://10tec.com/net-documentation-tool/microsoft-help-viewer-mshc.aspx))
without Visual Studio, without the .NET Framework, and without an embedded browser engine.
It is a native Rust application: one binary, a fast window, and a search engine that is
genuinely better than the classic Help Viewer index.

```
lucid demo testdata/demo-help.mshc      # write the built-in demo package
lucid testdata/demo-help.mshc           # read it
```

Status: **v0.1** — reading, searching, browsing, exporting and packing all work; see
[`SPEC.md`](SPEC.md) for the acceptance criteria and [`docs/DESIGN.md`](docs/DESIGN.md) for
the format notes and architecture.

---

## What it does

**Reading**

* Opens a single `.mshc`, a **folder** of them (a Help Library `ContentStore/<LOCALE>`
  directory), or a `.msha` setup document — and merges the whole content set into one library.
* Rebuilds the table of contents from the manifest *and* from the `Microsoft.Help.*` meta tags,
  so packages from any authoring tool (DocMounter, Sandcastle/SHFB, hand-rolled) work, and a
  topic the manifest forgot still appears.
* Renders topics natively: headings, lists, tables, code, notes, images, and the Microsoft Help
  behaviours `CollapsibleArea`, `CodeSnippet` (with copy + language tabs),
  `LanguageSpecificText`, `InheritedMember`.
* Resolves `ms-xhelp:///?Id=…` links, relative links, `#anchors` and cross-package links;
  opens external URLs in your browser.
* Tabs, per-topic outline ("On this page"), in-topic find with highlighting, zoom,
  bookmarks with history, dark and light themes.

**Searching** (the part that was worth rewriting)

| You type | You get |
| --- | --- |
| `install` | ranked hits, field-weighted (title ≫ keywords > body > code) |
| `phrase search` | both words required |
| `"phrase search"` | exact adjacency, same field |
| `sear*` | prefix match (the last word is automatically a prefix while you type) |
| `serach~` | fuzzy match, ranked below exact hits |
| `-legacy` | exclusion |
| `search OR index` | union |
| `title:install`, `body:"getting started"`, `code:DocSet`, `kw:setup`, `id:…`, `path:…` | field filters |

Search-as-you-type, snippets with exact highlight ranges, filter chips, per-package scope,
sort by relevance/title/path, "did you mean" suggestions, and a **persisted index**
(`$XDG_CACHE_HOME/lucid/index/<fingerprint>.idx`) so reopening a book is instant.

**Packing** — `lucid-forge` builds valid packages from a folder of topic HTML:

```
lucid-forge build ./docs -o Product.mshc --title "Product docs" --locale en-us
lucid-forge validate Product.mshc
lucid-forge outline Product.mshc
```

It copies your tree verbatim, reads the meta tags, synthesizes the `HelpContentSet` manifest,
writes the OPC parts (`[Content_Types].xml`, `_rels/.rels`) and is **byte-for-byte
reproducible**: the same input produces the same package.

---

## Install / build

Rust 1.86 or newer, no system development packages needed (no GTK, no WebKit):

```bash
git clone <this repo> lucid && cd lucid
cargo build --release
./target/release/lucid --help
```

Dependencies are pinned deliberately (see *Development notes*): the window stack is
`eframe`/`egui` 0.32 (OpenGL, Wayland/X11), the format layer uses `zip` + `roxmltree`, and the
search engine is written from scratch.

## Using the GUI

| Keys | Action |
| --- | --- |
| `Ctrl+O` | open a package, folder or setup document |
| `Ctrl+P` | go to a topic by title (fuzzy) |
| `Ctrl+K` | focus search |
| `Ctrl+F` | find inside the current topic (`Enter`/`▲`/`▼`, `Aa` for case) |
| `Ctrl+W` | close tab · **Ctrl+D** bookmark |
| `Alt+←/→` | history back/forward |
| `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | zoom in/out/reset |
| `F5` | rebuild the search index |

Panes: **Contents** (filterable tree), **Index** (keywords, multi-select, `F1` keywords in
green), **Search** (results, snippets, suggestions), **Favorites** (bookmarks + history).
Drag and drop works anywhere in the window, and the right-hand pane shows the outline of the
open topic.

## Command line

```
lucid [<path>] [--topic <id>] [--search <term>] [--f1 <keyword>] [--no-index]
             [--pane contents|index|search|favorites] [--theme dark|light]
             [--size WxH] [--no-maximize] [--screenshot out.png] [--frames N]
lucid info <path> [--json]
lucid toc <path> [--depth N]
lucid search <path> <query> [--limit N] [--json]
lucid extract <path> -o <dir> [--force]
lucid demo <out.mshc> [--force]
lucid --version | --help
```

Exit codes: `0` success, `1` I/O or format error, `2` usage, `3` nothing found.
`lucid --topic <id>` and `lucid --f1 <keyword>` are the hooks for context-sensitive help
(`F1`) in your own application.

The window opens maximized; `--size 1280x800` pins an exact size instead. `--screenshot`
renders the GUI, writes a PNG and exits, which is how the layouts are checked at 900×600
through 2560×1368 without a human at the keyboard:

```
lucid testdata/demo-help.mshc --size 900x600 --screenshot /tmp/lucid-900.png
lucid testdata/demo-help.mshc --theme light --pane search --search bm25 --screenshot /tmp/lucid-search.png
```

## Format support

| Input | Status |
| --- | --- |
| `.mshc` (DocMounter, Help Viewer 1.0/2.x, SHFB/Sandcastle output) | full support |
| Content-store folder (`ContentStore/<LOCALE>`) | full support, merged into one library |
| `.msha` setup document | package discovery |
| `.mshi` / `.metadata` sidecars | best effort |
| `.cab` content packages | detected with an actionable error (extract first) |
| CHS/HxS (`*.hxs`), CHM | roadmap |

## Repository layout

```
crates/lucid-mshc    format core: zip/OPC → packages → topics, TOC, keywords, assets, content sets
crates/lucid-search  inverted index + BM25F, query language, snippets, suggestions, persistence
crates/lucid-forge   the packer (library + CLI) and the demo-package generator
crates/lucid         the application: egui GUI + headless CLI
docs/DESIGN.md       format algorithm, frozen APIs, UX specification
SPEC.md / PLAN.md    requirements, acceptance criteria and the implementation plan
testdata/            demo-help.mshc — committed, self-authored sample package
```

## Development

```bash
cargo test --workspace          # parser, TOC, index, ranking, packer round-trips
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p lucid-forge -- demo testdata/demo-help.mshc --force   # regenerate the sample
```

Notes worth knowing before you touch dependencies:

* **MSRV is 1.86 here.** `image`, `zip` and the `icu_*` chain are pinned for it: newer
  releases require rustc 1.88+, and `eframe → egui-winit → webbrowser → url → idna → icu_*`
  is not something an application can opt out of. `Cargo.lock` therefore carries
  `icu_* = 2.2.0` (the newest icu that still supports 1.86) — if you bump one of these, run
  `cargo tree -i <crate>` first and keep the lock file in the commit.
* **No `unsafe`**: every crate starts with `#![forbid(unsafe_code)]`.
* `cargo clippy`/`cargo fmt` are expected in CI but are not part of this machine's rustc
  installation; here the gate is a warning-free `cargo check --workspace --all-targets`
  plus `cargo test --workspace`.
* The demo package is generated (`lucid demo`), not vendored: no third-party documentation is
  redistributed with Lucid.

## Roadmap

Short version: in-process `.cab` support, syntax highlighting for code snippets, PDF/EPUB
export, CSS subset support, optional semantic (embedding) search, CHM/HxS import, and a GUI
authoring mode. Details and ordering live in [`docs/DESIGN.md`](docs/DESIGN.md) §7.

## License

MIT — see [`LICENSE`](LICENSE). The Lucid name, the demo content and the code are original;
the `.mshc` format belongs to Microsoft, and the product this re-makes is 10Tec's
[Help Viewer](https://10tec.com/help-viewer/) (Windows/.NET), which Lucid is not affiliated
with.
