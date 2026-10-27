# SPEC — Lucid, a modern MSHC (Microsoft Help Container) viewer & packer

Status: **draft for v0.1** · Owner: gishi · Deep technical reference: [`docs/DESIGN.md`](docs/DESIGN.md)

## 1. Problem

`.mshc` (Microsoft Help Container, a.k.a. MSHelp3 / Help 2.x content) is the help format
introduced with Microsoft Help Viewer 1.0 / Visual Studio 2010. It is a zip archive of
XHTML topics carrying `Microsoft.Help.*` meta tags. The only first-party viewer
(Microsoft Help Viewer) ships inside Visual Studio, and the established third-party
alternative (10Tec Help Viewer, `https://10tec.com/help-viewer/`) is Windows/.NET-only,
closed, keyboard-driven, and its search/index panes are the classic flat-list Help 1.x
experience.

Result: hundreds of thousands of pages of shipped offline documentation are effectively
unreadable on Linux/macOS, and even on Windows the reading/search experience is dated.

## 2. Goal

Ship **Lucid**: a single-binary, cross-platform, native Rust application that

1. opens any `.mshc` package (and whole content sets) without Visual Studio, .NET, or a
   browser engine;
2. presents it with a modern GUI (Contents / Index / Search / Favorites + tabbed topic
   browser);
3. searches dramatically better than Help Viewer 2.x (instant, ranked, phrase/prefix/
   fuzzy, field filters, snippets, highlighting, suggestions, persisted index);
4. can also **produce** `.mshc` packages (`lucid-forge`) so the format stays usable
   without DocMounter/VS.

## 3. Users & scenarios

| Persona | Scenario |
| --- | --- |
| Developer on Linux/macOS | "A vendor shipped `ProductDocs.mshc`; I want to read and search it today." |
| Windows developer without VS | "I need offline help for a product whose docs are .mshc." |
| Application developer | "I ship context-sensitive help: `lucid --topic <id>` / `lucid --search <term>` from my app, and `lucid-forge build` to make the package." |
| Documentation author | "I have a folder of generated HTML with Help meta tags; pack it into a valid .mshc." |
| Power user | "I keep 40 books in a content-store folder; open the whole store, favourite pages, jump by keyboard." |

## 4. Functional requirements

### 4.1 Opening content
- **FR-1.1** Open a single `.mshc` file (path argument, GUI open dialog, drag & drop).
- **FR-1.2** Open a directory as a content set: every `*.mshc` inside is loaded and merged
  (TOC subtrees per package, keywords merged, search across all).
- **FR-1.3** Open a `.msha` *setup* document: read its `div.package` entries and load the
  referenced `.mshc` packages relative to it.
- **FR-1.4** Detect `.cab` packages and fail with an actionable message (no silent zip error).
- **FR-1.5** Tolerate malformed-but-common packages: missing `[Content_Types].xml`,
  missing manifest, upper/lower-case path mismatches, BOMs, `windows-1252`/UTF-8 topics,
  duplicate ids, topics without `TocParent`.
- **FR-1.6** Report load progress; the UI must stay responsive for large books.
- **FR-1.7** Recent files list (last 10) with one-click reopen.

### 4.2 Navigation
- **FR-2.1** Contents pane: hierarchical TOC built from the manifest when present, else
  from `Microsoft.Help.TocParent`/`TocOrder` meta tags (algorithm in DESIGN §2.4).
- **FR-2.2** Type-to-filter in the TOC that prunes to matches while preserving ancestors.
- **FR-2.3** The active topic is auto-revealed and highlighted; expansion state persists.
- **FR-2.4** Index pane: keyword list from manifest index entries, `Microsoft.Help.Keywords`
  and `Microsoft.Help.F1`; instant filter; click to open; multi-select → open all.
- **FR-2.5** Tabs for open topics with close/close-all, plus per-content-set history.
- **FR-2.6** Quick-open (Ctrl+P): fuzzy match over topic titles across the content set.
- **FR-2.7** Breadcrumb of the active topic from the TOC; clicking a segment opens it.
- **FR-2.8** "On this page" outline built from headings h1–h6, click to scroll.

### 4.3 Search
- **FR-3.1** Live search while typing (debounced ≤ 150 ms), prefix-aware for the last term.
- **FR-3.2** Query language per DESIGN §4.1: AND, `"phrase"`, `term*`, `term~`, `-exclude`,
  `OR`, and `field:` filters (`title: body: code: kw: id: path: heading:`).
- **FR-3.3** BM25F ranking with the field weights and boosts in DESIGN §4.2; deterministic
  tie-breaking.
- **FR-3.4** Results show title, breadcrumb, highlighted snippet, matched-field chips.
- **FR-3.5** Filter chips (All / Title / Code / Keywords), scope selector (per package / all),
  sort selector (Relevance / Title / Path).
- **FR-3.6** Empty results → fallback chain: prefix → fuzzy → spelling suggestions ("did you
  mean"), never a bare empty list.
- **FR-3.7** Index is built in the background with progress, then persisted; reopening the
  same content set must not rebuild unless the fingerprint changed.
- **FR-3.8** Selecting a result opens the topic and highlights the matched terms in the
  document; "find in topic" (Ctrl+F) supports next/previous and a match counter.

### 4.4 Reading
- **FR-4.1** Render the XHTML subset natively: headings, paragraphs, emphasis, lists
  (incl. nested), tables, `<pre>`/`<code>`, blockquotes, horizontal rules, images, anchors.
- **FR-4.2** Render MSHC behaviours: `CollapsibleArea`, `CodeSnippet` (with copy button and
  language tabs), `LanguageSpecificText`, `div.title`, `div.alert` + `span.label`,
  `Inherited Member`.
- **FR-4.3** Resolve `ms-xhelp:///?Id=<id>` links, relative links, `#anchors`, and links
  into sibling packages; open external `http(s)` links in the system browser.
- **FR-4.4** Images are loaded from package assets case-insensitively and cached in memory.
- **FR-4.5** Zoom (Ctrl+±/Ctrl+0/Ctrl+scroll) and adjustable UI font size; dark and light
  themes; the choice persists.
- **FR-4.6** Text selection + copy works in the document; code blocks have a copy button;
  "copy link" for the current topic.
- **FR-4.7** Export the current topic (self-contained HTML with assets) and the whole book
  (folder of HTML) via GUI action and CLI.

### 4.5 Automation (CLI)
- **FR-5.1** `lucid [path]`, `lucid --topic <id>`, `lucid --search <term>` open the GUI at
  the requested state (context-sensitive help integration).
- **FR-5.2** Headless: `lucid info`, `lucid toc`, `lucid search`, `lucid extract`,
  `lucid demo`, `lucid --version` (contract and exit codes in DESIGN §6).
- **FR-5.3** Headless output is stable, pipe-friendly text (no ANSI when not a TTY).

### 4.6 Authoring
- **FR-6.1** `lucid-forge build <dir> -o out.mshc` packs topics + assets into a package with
  `[Content_Types].xml`, `_rels/.rels` and a `HelpContentSet` manifest built from meta tags.
- **FR-6.2** `lucid-forge demo <out.mshc>` writes a self-authored demo package that
  exercises every rendering and search feature (also committed to `testdata/`).
- **FR-6.3** `lucid-forge validate <file.mshc>` reports structural problems (dangling TOC
  URLs, duplicate ids, unreachable parents, zero-byte topics).

## 5. Non-functional requirements

- **NFR-1** Startup to interactive window < 300 ms; with a persisted index, opening a known
  content set < 500 ms; first-time open of a 1 000-topic package < 3 s on a laptop.
- **NFR-2** Query latency: p95 < 5 ms for a 10 000-topic corpus (measured by a bench/CLI
  timer shown in the UI as "N results in X ms").
- **NFR-3** Memory: < 400 MB resident for a 100 000-topic content set (topics are read
  lazily; only metadata + rendered topic stay in memory).
- **NFR-4** Pure Rust safe code (`#![forbid(unsafe_code)]`) except third-party crates; no
  runtime dependency on .NET, Java, a browser/WebView, or a database server.
- **NFR-5** Cross-platform: Linux (Wayland/X11), Windows, macOS. Linux build must not
  require pkg-config/GTK.
- **NFR-6** `cargo fmt` clean, `cargo clippy -- -D warnings` clean, `cargo test` green.
- **NFR-7** Every parser rule is covered by a test built on generated fixtures (no
  third-party content is redistributed).

## 6. Compatibility matrix (v0.1)

| Source | Status |
| --- | --- |
| DocMounter / 10Tec-generated `.mshc` | Supported (meta-tag + manifest TOC, assets, keywords) |
| Visual Studio Help Viewer 1.0/2.x `.mshc` | Supported (incl. branding-package absence) |
| SHFB / Sandcastle `.mshc` output | Supported (same metadata contract) |
| Content-store folder (`ContentStore/<LOCALE>`) | Supported |
| `.msha` setup document | Supported (package discovery) |
| `.cab` content packages | Detected, clear error; extraction roadmap |
| `.mshi` / `.metadata` sidecars | Read for package metadata (best effort) |
| CHM (`*.chm`), HxS (`*.hxs`) | Out of scope for v0.1 (roadmap) |

## 7. Persistence & data locations

| Data | Location |
| --- | --- |
| Settings, recent files, bookmarks, history, pane state, TOC expansion | `$XDG_CONFIG_HOME/lucid/state.json` |
| Search index cache | `$XDG_CACHE_HOME/lucid/index/<fingerprint>.idx` |
| Exported topics/books | user-chosen directory |

Fingerprint = sorted package paths + sizes + mtimes + index format version; any change
forces a rebuild (silently, in the background).

## 8. Acceptance criteria (v0.1 "done")

1. `cargo build --release` succeeds on this machine (rustc 1.86, Linux, no system dev packages).
2. `cargo test` passes, including: package parsing on generated fixtures, TOC
   reconstruction both ways (manifest / meta-tags only), asset case-insensitive lookup,
   search ranking sanity (exact title beats body mention), phrase/prefix/fuzzy/exclude
   behaviour, snippet ranges pointing at the actual matched text, index round-trip through
   `save`/`load`.
3. `lucid demo testdata/demo-help.mshc && lucid info testdata/demo-help.mshc` shows the
   expected topic/TOC/keyword counts; `lucid toc` prints the tree; `lucid search
   testdata/demo-help.mshc "phrase search"` returns the search topic with a snippet
   containing the terms.
4. `lucid testdata/demo-help.mshc` opens a window with panes as in DESIGN §6; TOC
   navigation, search-as-you-type with highlighting, keyword index, bookmarks, tabs,
   outline, find-in-topic and the theme toggle all work by mouse and keyboard.
5. A package built by `lucid-forge build` from a directory of HTML opens in Lucid with the
   same TOC/keywords/search behaviour, and the same package opens in Microsoft Help Viewer
   (structural validity: OPC parts present, manifest TOC matches meta tags) — validated by
   `lucid-forge validate`.
6. Opening the same content set twice uses the persisted index (status bar shows the cache
   hit) and the second open is measurably faster.
7. README documents install, GUI usage, all keyboard shortcuts, CLI, and the format notes;
   `docs/DESIGN.md` stays the single source of truth for format decisions.

## 9. Assumptions & open questions

- **A-1** Users may legally open packages they have local access to; Lucid redistributes no
  third-party content, only the self-authored demo.
- **A-2** Fonts/emoji come from egui's bundled default fonts (Ubuntu-Light/Hack/NotoEmoji);
  no third-party font files are vendored.
- **A-3** The `.msha` *inside* a package is not standardized publicly; we accept any XML
  node shape carrying `Id`/`Title`/`Url` and always cross-check with meta tags
  (DESIGN §2.4). *Open question:* add a strict XSD once a real-world sample set is
  available.
- **A-4** Semantics of `.mshi` files are undocumented; parsed best-effort, never required.
- **A-5** License: dual MIT OR Apache-2.0 (placeholder until the owner confirms).
- **A-6** Windows/macOS builds are untested in this session (no such machine); the code
  paths are platform-neutral (egui, no OS APIs).
