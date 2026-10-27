# Lucid — design & format reference

A fast, modern, cross-platform re-make of the 10Tec "Help Viewer for MSHC files"
(`https://10tec.com/help-viewer/`): a viewer for **Microsoft Help Container (.mshc)**
documentation — the help format introduced with Microsoft Help Viewer 1.0 / VS 2010
(a.k.a. "MSHelp3"/"Help 2.x content") — plus a native **packer** so the same format can
be authored without Visual Studio or DocMounter.

Everything is Rust. The viewer is a single native binary (egui/eframe, no web engine, no
runtime dependency beyond the GPU/OS windowing stack), the search engine is written from
scratch (BM25F + phrase/prefix/fuzzy), and the packer emits MSHC packages from a folder of
topic HTML.

---

## 1. Goals / non-goals

**Goals**

1. Open `.mshc` packages and *content sets* (folders of `.mshc`, e.g. a Help Library
   `ContentStore\EN-US` directory) and present them like Microsoft Help Viewer does:
   Contents / Index / Search / Favorites panes + topic browser with a real toolbar.
2. Be *tolerant*: MSHC packages are produced by many authoring tools (DocMounter,
   Sandcastle, SHFB, Word→MSHC converters, VS itself). The parser accepts every layout
   variant we know of and degrades gracefully instead of failing to open a package.
3. Search that is genuinely better than Help Viewer 2.x: instant (typing-as-you-go),
   ranked (BM25F, field-weighted), with exact phrases, field filters, exclusions,
   prefix and fuzzy matching, highlighting, snippets, "did you mean", and a persisted
   index so re-opening a package is instant.
4. Feel modern: dark/light themes, 8-pt spacing, rounded surfaces, resizable panes,
   keyboard-first navigation, tabs, per-topic outline, zoom, copy-code, DRY CLI.
5. Cross-platform, no Windows API, no .NET, no HTML engine needed.

**Non-goals (v0.1)**

- Rendering arbitrary modern web HTML/CSS/JS. Topics are W3C Basic 1.1 XHTML + a small
  set of `Microsoft.Help.*` behaviours; we render that subset *natively* and skip scripts.
- Being an authoring GUI (that is `lucid-forge`, a CLI; a GUI editor is on the roadmap).
- Editing/annotating content on disk (bookmarks/history are stored outside the package).

---

## 2. The MSHC format, as implemented

### 2.1 Container

An `.mshc` is a **zip archive** (Open Packaging Conventions style) whose extension was
changed to `.mshc`. Observed contents:

| Part | Meaning |
| --- | --- |
| `[Content_Types].xml` | OPC content-type map (extension → MIME). Optional for us. |
| `_rels/.rels` | OPC relationships; points at the manifest part. Optional for us. |
| `*.msha` (often a GUID or the product name) | Manifest: optional explicit TOC + index. Namespace seen in the wild: `http://schemas.microsoft.com/help/2010/06/`. |
| `*.htm` / `*.html` / `*.xhtml` | Topic documents (XHTML + `Microsoft.Help.*` meta tags). **The authoritative source of TOC/index data.** |
| everything else (`ic*`, `images/`, `styles/`, `*.css`, `*.js`, `*.gif`, …) | Assets referenced by topics, resolved *case-insensitively* (real packages reference `ICONS/foo.GIF` while the zip entry is `icons/foo.gif`). |

Additional deployment formats we accept:

- **Content set folder** — a directory containing several `.mshc` files plus
  `*.mshi` / `*.metadata` sidecars (the layout of
  `C:\ProgramData\Microsoft\HelpLibrary2\Catalogs\<Catalog>\ContentStore\<LOCALE>`).
- **`.msha` setup file** — the HTML-ish *Help Content Setup* document listing
  `<div class="package"><a class="current-link" href="x.mshc">` entries; we use it to
  discover sibling packages when the user opens it.
- **`.cab`** — VS content packages are cab files containing `.mshc`. v0.1 detects cab
  magic bytes and reports a clear "extract with cabextract/unshield first" error
  instead of a confusing zip error. (In-process cab support is on the roadmap.)

### 2.2 Topic metadata (the contract)

From the *Microsoft Help Viewer SDK* (Content Metadata Reference), with the quirks
found in real files. All names are matched **case-insensitively**.

| Meta name | Meaning in Lucid |
| --- | --- |
| `<title>` | Topic title. Required. |
| `Microsoft.Help.Id` | Topic id; target of `ms-xhelp:///?Id=…` links. Required. |
| `Microsoft.Help.TocParent` | Parent topic id; `-1`/absent = TOC root. |
| `Microsoft.Help.TocOrder` | Sort key among siblings (integer, ascending). |
| `Microsoft.Help.Keywords` | Comma-separated index keywords ("K keywords"). |
| `Microsoft.Help.F1` | F1 keyword(s), comma separated. |
| `Microsoft.Help.Locale` / `Microsoft.Help.TopicLocale` | Language of the topic. |
| `Description` | Short summary (used as a search field + tooltip). |
| `Microsoft.Help.Category`, `Product`, `ProductVersion`, `TopicVersion` | Kept as raw metadata. |
| `SelfBranded`, `Microsoft.Help.TopicVendor` | Recorded; branding packages are not applied (we render with Lucid's own theme). |
| `Title` (as a `<meta name="Title">`) | Some authoring tools emit the title as a meta tag instead of `<title>`; we accept both. |
| `Microsoft.Help.Book`, `Microsoft.Help.TopicPublishDate` | Recorded (older `mshc2devhelp`-style content uses `Book`). |

### 2.3 Content behaviours inside topic bodies

Rendered natively (no JS):

- `<CollapsibleArea Expanded Title>` → collapsible section.
- `<CodeSnippet Language DisplayLanguage EnableCopyCode>` → code block with copy button
  and language tabs when several snippets share a `snippetGroup`.
- `<LanguageSpecificText devLangcs=… devLangvb=… devLangcpp=… devLangnu=…>` → substituted
  from the user's preferred code language.
- `div.title` → big topic heading; `div.alert` + `span.label` → callout box.
- `ms-xhelp:///?Id=<id>` links → internal navigation by topic id.
- `#anchor` links, relative links (`../x.htm`, `sub/y.htm#z`, `./a.htm`) → resolved
  inside the package (and across packages of a content set), case-insensitively.
- `Inherited Member` markup → collapsible "members" block.

Legacy vendor markup (SHFB / Sandcastle 1.x, the shape most real `.mshc` files still
have) is normalised to the model above rather than passed through as raw HTML:

- `div.pageHeader` / `div.pageFooter`, `#header` / `#footer` → package chrome, dropped
  (the topic header and footer belong to the viewer, not the document).
- `table.titleTable > td.titleColumn` → the topic heading. `AddLanguageSpecificTextSet`
  separators (`cs=.|vb=.|cpp=::|nu=.`) are resolved through the preferred language.
- `div.codeSnippetContainer` (SHFB) and `div.code` + `span[codeLanguage]` (Sandcastle) →
  one grouped code block with language tabs and a copy button; a tab-less container
  falls back to a plain code block.
- `div.collapsibleAreaRegion` followed by its `div.collapsibleSection`, and Sandcastle's
  `h1.heading` + `div[name="collapseableSection"]` → a collapsible section, also listed
  in the per-topic outline.
- `span.languageSpecificText > span.cs/.vb/.cpp/.nu` → language-specific text.
- Toggle/hidden images are dropped; a block-level `<br>` becomes a real line break
  (reference signatures put `Type: …` and its description on separate lines).

Reference tables (member lists, parameter and exception tables) are laid out by the
renderer rather than by the browser: each column is measured from its widest line and
widest unbreakable token, every cell is wrapped at its own column width, and a cell's
blocks are stacked vertically. This keeps long summaries inside the reading measure; a
`Grid`-only version let multi-paragraph cells run sideways and stretched rows to
thousands of pixels.

### 2.4 TOC reconstruction algorithm

1. If a manifest part contains TOC nodes (`HelpTOCNode`, `HelpTOC`, `Topic`, `Node` with
   `Id`/`Title`/`Url` in any case) → build the tree from it, resolving `Url` to topics.
2. Otherwise (and always as a cross-check): build the tree from topic meta tags —
   group topics by `TocParent`, order by `TocOrder` then title, roots are `-1`,
   empty/absent parent, or a parent id that does not exist (those become roots, never
   dropped).
3. If a parent id has no topic of its own (typical for pure grouping nodes) → create a
   synthetic (non-clickable) node whose title is `Microsoft.Help.Id`-derived, prettified.
4. Multi-package content sets: each package gets its own subtree; packages are ordered by
   locale match, then package title, then file name. Duplicate ids across packages are
   resolved by preferring the package whose locale matches the active UI locale.

### 2.5 Keywords / F1

The Index pane is fed by, in order: manifest index entries, `Microsoft.Help.Keywords`
(comma-separated, trimmed, empty ignored), and `Microsoft.Help.F1` values (marked as F1).
The pane filters instantly and supports multi-select → "open all matching topics" (a
Help-Viewer feature that is genuinely useful) and copy.

### 2.6 Presentation rules we inherit from the SDK

- Without a branding package, Help Viewer still renders topics; Lucid always renders with
  its own theme and ignores branding packages (documented difference).
- `Microsoft.Help.Locale` beats the generic `Language` attribute.
- Multiple topics may share an id when their `TopicLocale` differs; the locale matching
  the active locale wins for the TOC, all locales remain reachable in search.

---

## 3. Architecture

```
crates/lucid-mshc    format core: zip/OPC → packages → topics, TOC, keywords, assets, doc sets
crates/lucid-search  inverted index (BM25F) + query language + snippets + suggestions + persistence
crates/lucid-forge   MSHC packer/validator CLI (build a package from topic HTML; demo generator)
crates/lucid         the application: egui GUI + headless CLI (`lucid info|search|toc|demo`)
docs/                this document
testdata/            demo packages used by tests and committed for a first-run demo
```

Data flow (GUI):

```
open path ──► DocSet::load (worker thread, progress events)
                │
                ├─► TOC tree      ─┐
                ├─► keyword index ─┤──► UI panes (left dock)
                └─► topics ────────┴──► IndexBuilder (worker thread) ──► SearchIndex (Arc)
                                              │
                        search box ───────────┘  (query on UI thread: sub-ms, no lock held long)
                                              │
                                       Hit{title, path, snippet, ranges} ──► Search pane
```

Threading rules:

- Parsing and index building happen **off the UI thread**; progress is reported through a
  channel and the UI stays interactive (opening a book shows a progress bar in the status
  bar).
- The index is immutable once built (`Arc<SearchIndex>`); queries never block the UI and
  never need `&mut`.
- Persistence: `$XDG_CACHE_HOME/lucid/index/<fingerprint>.idx` (fingerprint = package
  set + sizes + mtimes + index format version). Load is attempted first; a mismatch
  triggers a rebuild.

Application state (persisted as JSON in `$XDG_CONFIG_HOME/lucid/state.json`):
recent files, bookmarks/favorites, TOC expansion state per package, theme, font size,
preferred code language, pane widths, last topic per package, window geometry.

---

## 4. Frozen API — `lucid-search`

The search crate is standalone: it knows nothing about MSHC, only documents and fields.

```rust
pub type DocId = u32;

pub const FIELD_TITLE: u8 = 0;
pub const FIELD_HEADINGS: u8 = 1;
pub const FIELD_KEYWORDS: u8 = 2;
pub const FIELD_BODY: u8 = 3;
pub const FIELD_CODE: u8 = 4;
pub const NUM_FIELDS: usize = 5;

pub struct DocInput<'a> {
    pub title: &'a str,
    pub headings: &'a str,   // concatenated headings + section titles
    pub keywords: &'a str,   // index keywords + F1 keywords
    pub body: &'a str,       // visible body text
    pub code: &'a str,       // <pre>/<code>/<CodeSnippet> content
    pub path: &'a str,       // human breadcrumb ("Guides > Search > Queries")
    pub weight: f32,         // document prior, 1.0 = neutral
}

pub struct IndexOptions { pub stem: bool, pub stopwords: bool, pub max_snippet_chars: usize }
impl Default for IndexOptions { /* stem: true, stopwords: true, max_snippet_chars: 240 */ }

pub struct IndexBuilder { /* … */ }
impl IndexBuilder {
    pub fn new(opts: IndexOptions) -> Self;
    pub fn add(&mut self, doc: DocInput<'_>) -> DocId;   // DocIds are assigned 0,1,2,…
    pub fn build(self) -> Index;
}

pub enum Sort { Relevance, Title, Path }
pub enum FuzzyMode { Never, Auto, Always }
pub enum SuggestionKind { Spelling, Term }

pub struct SearchOptions {
    pub limit: usize,                  // 0 = unlimited
    pub sort: Sort,
    pub fields: u8,                    // bitmask over FIELD_*; default all
    pub fuzzy: FuzzyMode,              // Auto = fuzzy only when no hits and query looks misspelled
    pub prefix_last_term: bool,        // true while the user types
    pub scope: Option<Vec<DocId>>,     // restrict to a subset (package scoping)
    pub snippet: bool,
}
impl Default for SearchOptions { /* limit 200, Relevance, all fields, Auto, false, None, true */ }

pub struct SearchResults {
    pub hits: Vec<Hit>,
    pub total_matched: usize,
    pub took_us: u64,
    pub suggestions: Vec<Suggestion>,
    pub terms: Vec<String>,            // effective query terms (including expansion), for find-in-topic
}

pub struct Hit {
    pub doc: DocId,
    pub score: f32,
    pub title: String,
    pub path: String,
    pub snippet: String,
    pub snippet_ranges: Vec<std::ops::Range<usize>>,  // char indices into `snippet`
    pub title_ranges: Vec<std::ops::Range<usize>>,    // char indices into `title`
    pub fields: u8,                                   // bitmask of fields that matched
}

pub struct Suggestion { pub text: String, pub kind: SuggestionKind }

pub struct IndexStats { pub docs: usize, pub terms: usize, pub postings: usize, pub bytes: usize }

impl Index {
    pub fn search(&self, query: &str, opts: &SearchOptions) -> SearchResults;
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<Suggestion>;
    pub fn doc_meta(&self, id: DocId) -> Option<DocMeta>;   // { title, path }
    pub fn stats(&self) -> IndexStats;
    pub fn to_bytes(&self) -> Vec<u8>;
    pub fn from_bytes(bytes: &[u8]) -> Result<Index, Error>;
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()>;
    pub fn load(path: &std::path::Path) -> Result<Index, Error>;
}
```

### 4.1 Query language

| Syntax | Meaning |
| --- | --- |
| `foo bar` | AND (both must match somewhere in the document) |
| `"foo bar"` | exact phrase (adjacent tokens, same field) |
| `foo*` / last term with `prefix_last_term` | prefix match |
| `foo~` | fuzzy match (edit distance ≤ 2), ranked below exact |
| `-foo` | exclude documents containing `foo` |
| `foo OR bar` | union (case-sensitive `OR`) |
| `title:foo`, `t:`, `body:`, `code:`, `kw:` `keywords:`, `id:`, `path:`, `heading:` | field filters |

Defaults that make it feel "improved" versus Help Viewer: multi-term AND with per-term
highlighting, ranking that respects field proximity, and *zero result* handling that falls
back to prefix → fuzzy → spelling suggestions instead of showing an empty list.

### 4.2 Ranking (BM25F, deterministic)

```
score(doc) = doc.weight · Σ_f w_f · Σ_t idf(t) · tf_norm(t,f)
tf_norm     = tf / (tf + k1·(1 − b + b·len_f/avg_len_f))      k1 = 1.2, b = 0.65
w_f         = title 8.0, headings 3.0, keywords 5.0, body 1.0, code 0.6
idf(t)      = ln(1 + (N − df + 0.5)/(df + 0.5))
```
Boosts: exact title == query ×3, phrase in title ×2, phrase in body ×1.35, all query
terms present ×1.2, prefix match ×0.9, fuzzy match ×0.5. Ties break on
(score desc, title len asc, title asc, doc asc) so results never jitter.

### 4.3 Snippets

Body text is split into sentences/lines; a sliding window of ≤ `max_snippet_chars` is
scored by matched-term density (phrases worth double) and the best window is returned with
`…` ellipses plus `snippet_ranges` for highlighting. If only `code` matched, the snippet
comes from the code line containing the best match and is prefixed with `⟨code⟩ `.

### 4.4 Persistence

`to_bytes` writes a versioned bincode blob: header (`LUCIDIDX` + format version), options,
doc metas, term dictionary (sorted), postings, lengths, averages. Unknown versions are
rejected (rebuild). No unsafe code.

---

## 5. Frozen API — `lucid-forge`

Library + CLI that writes real MSHC packages (and doubles as the test-fixture generator).

```
lucid-forge build <input-dir> -o out.mshc [--title T] [--locale en-us] [--root-id -1]
                                          [--no-manifest] [--no-opc] [--force]
lucid-forge demo <out.mshc> [--force]        # writes the built-in demo package
lucid-forge validate <file.mshc>             # structure + metadata sanity report (exit code 1 on error)
lucid-forge outline <file.mshc>              # prints the TOC derived from meta tags
```

Package layout written by `build` (faithful to real packages):

```
[Content_Types].xml            # Default per extension: htm/html→text/html, css→text/css,
                               # js→application/javascript, gif/png/jpg/bmp→image/*,
                               # plus Override for the manifest part
_rels/.rels                    # Relationship → manifest, type …/help/2010/06/msha
<slug>.msha                    # HelpContentSet manifest (HelpTOC + HelpIndex), see below
<topic paths>                  # input tree copied verbatim (any nesting), meta tags intact
<asset paths>                  # non-HTML files copied verbatim
```

`build` **requires** each topic to carry `Microsoft.Help.Id`; `TocParent`/`TocOrder` are
read to synthesize the manifest TOC (so `build` output is valid for Help Viewer *and* for
Lucid's meta-tag-only mode). Missing `Title` meta falls back to `<title>`; missing
`Microsoft.Help.Id` is an error unless `--allow-missing-ids` (then the file stem is used).
`--no-manifest` produces a package without `.msha` — this is the fixture for testing
meta-tag-only TOC reconstruction. `--no-opc` omits `[Content_Types].xml`/`_rels/.rels`.

`.msha` manifest schema written (and accepted by the parser in all its variants):

```xml
<?xml version="1.0" encoding="utf-8"?>
<HelpContentSet xmlns="http://schemas.microsoft.com/help/2010/06/" Language="en-us" Title="…">
  <HelpTOC>
    <HelpTOCNode Id="…" Title="…" Url="topics/intro.htm">
      <HelpTOCNode Id="…" Title="…" Url="…"/>
    </HelpTOCNode>
  </HelpTOC>
  <HelpIndex>
    <HelpIndexEntry Keyword="…" Url="topics/intro.htm"/>
  </HelpIndex>
</HelpContentSet>
```

`demo` writes a self-authored package (no third-party content) exercising: nested TOC
(3 levels), an orphan topic, a topic with no id in one variant, tables, note callouts,
collapsible areas, code snippets in C#/VB/F#, language-specific text, anchors, an
`ms-xhelp://` cross-link, a generated `.bmp` asset (hand-encoded, no deps) and a `.css`
asset. It is committed as `testdata/demo-help.mshc` so a fresh clone has something to open
(`lucid demo testdata/demo-help.mshc` regenerates it).

Validation rules (`validate`, also used by tests): zip opens; every manifest `Url` points
at an existing part (case-insensitive); duplicate `Microsoft.Help.Id` reported; every
`TocParent` resolvable; keywords non-empty; no zero-byte topics.

---

## 6. Application UX (`crates/lucid`)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ Lucid  [Open] [Reindex] [Go to topic (Ctrl+P)]   100% ⟲ − + Outline [C#] 🌙 │
│ [🔍 Search (Ctrl+K) — "phrase", term*, field:value, -exclude______________] │
├───────────────┬──────────────────────────────────────────────┬───────────────┤
│ Contents │ Index │ Search │ Favorites                          │  On this page │
│  ⏵ Guides     │  Guides › Getting started › Installing       │   • Overview  │
│    · Intro    │  ┌──────────────────────────────────────────┐ │   • Install   │
│    ⏷ Search   │  │ 12 results in 3 ms                       │ │   • Queries   │
│      · Queries│  │ Installing the CLI                       │ │               │
│               │  │ …copy the binary and run lucid info…     │ │               │
│ filter: ______│  └──────────────────────────────────────────┘ │               │
├───────────────┴──────────────────────────────────────────────┴───────────────┤
│ Introduction to Lucid ✖ │ Other topic ✖ │ Close all      bookmark ☆  copy link│
├──────────────────────────────────────────────────────────────────────────────┤
│ demo-help.mshc │ 19 topics │ index 1424 terms (cached) │ single package       │
└──────────────────────────────────────────────────────────────────────────────┘
```

The window opens **maximized** (a help viewer is a full-screen reading surface). The
toolbar is two rows — controls on top, the search box on its own full-width row — because
at the 900 px minimum width a single row either overflowed or squeezed the search box down
to an unusable sliver. The status bar never repeats what another element already says
(the source and topic count on the left, the index state in the middle).

### 6.1 Layout rules

| Rule | Value | Why |
| --- | --- | --- |
| Minimum window | 900 × 600 | Below this the panes stop being usable; the window manager enforces it |
| Outline pane | auto-hidden under 1080 px | The document needs the width more than a 200 px column of headings does; the Outline button stays visible and its tooltip explains why it is off |
| Sidebar | 300 px default, 220–520 px, never over 40% of the window | Resizable; the search results need the room, but at 900 px the document needs it more |
| Reading measure | 1120 px, centred | Un-capped, a 2560 px display gives ~170-character lines |
| Toolbar | two rows, auto height | See above |
| Status bar | one line, ellipsized | Never duplicates the index or pane counters |

### 6.2 Glyph policy

egui bundles three fonts (Ubuntu-Light, NotoEmoji, emoji-icon-font) and a missing glyph is
painted as an empty box — which is how `▾ ▸ ▲ ▼ ✕ ⓘ ⌕ ← →` all shipped in the first cut.
Only characters that were *verified* by rendering them are used in the UI:

- **Works:** `★ ☆ ✔ ✖ ▶ ◀ ⏵ ⏷ ⏶ ⏴ ☀ 🌙 ⚠ 📌 🔍 📁 📄 📦 ⟲ ↺ ⬆ ➡ ⬅ ⬇ ⮨ ⮩ ⏏ ● ■ ▪ › · — – … § ± µ ×`
- **Missing (do not use):** `▾ ▸ ▲ ▼ ✕ ⓘ ⌕ ← → ↑ ↓ ☾ ⏎` and the geometric-shape block
  (`□ ▫`) generally.

If a new icon is needed, render it with the glyph probe described in `PLAN.md` (S3) instead
of trusting a Unicode chart.

Panes (left dock, tabs, resizable, remembers width & tab):

1. **Contents** — TOC tree with type-to-filter (prunes the tree to matches, keeps
   ancestors), expand/collapse all, auto-reveal of the active topic, `Enter` opens,
   `→/←` navigate tree without a mouse.
2. **Index** — keyword list with instant filter, click → topic, `*`-suffix sub-entry
   display for `parent/child` style keywords, "open all matches" and "copy keywords".
3. **Search** — query box (Ctrl+K), live results while typing (debounced, prefix-aware),
   filter chips (All / Title / Code / Keywords), scope selector (all packages or one),
   sort selector, result list = title + breadcrumb + highlighted snippet, `Enter` opens,
   `↑/↓` move, footer shows `N results in X ms` plus "did you mean" suggestions when
   nothing matched.
4. **Favorites** — bookmarks grouped into folders (create/rename/remove), plus a
   "History" section (last 100 topics with timestamps), drag-free reordering.

Header: app menu (Open file…, Open folder/content set…, Recent, Close all, Export topic
to HTML, Reindex, Preferences, About), global quick-open (Ctrl+P) that fuzzy-matches
*titles* across the content set ("go to topic"), search box (Ctrl+K), theme toggle.

Topic view: tabs, breadcrumb, headings outline on the right (auto from h1–h6, highlight
current section on scroll, click to jump), in-topic find bar (Ctrl+F: highlight all,
`n`/`N`, match counter), zoom (Ctrl +/-/0, Ctrl+scroll), copy-code buttons, per-snippet
language tabs, note/callout styling, tables with zebra striping and wrapped cells,
images decoded from package assets (memory-cached, lazy), external links opened in the
system browser, `ms-xhelp://` and relative links resolved inside the content set.

Keyboard: `Ctrl+O` open, `Ctrl+P` quick open, `Ctrl+K` search, `Ctrl+F` find in topic,
`Ctrl+W` close tab, `Alt+←/→` history back/forward, `Ctrl+D` bookmark, `Ctrl+=`/`Ctrl+-`/
`Ctrl+0` zoom, `F5` reindex, `Esc` close overlay, `Ctrl+Q` quit.

Theme ("modern", both dark & light): neutral slate surfaces, one accent colour, 8-pt
grid, 6–8 px corner radius, 1 px hairline separators, no bevels; the document pane gets a
slightly different surface from the chrome so content stands out. Default: dark. Font
size is user-adjustable (0.9×–1.4×), monospace for code.

CLI (same binary): 

```
lucid [path]                              # open GUI (file, folder or .msha setup)
lucid --topic <id> --search <term>        # open GUI at a topic / with a query (context help / F1)
lucid info <path>                         # packages, topics, TOC stats, keywords, assets, ids
lucid toc <path> [--depth N]              # ASCII TOC
lucid search <path> <query> [--limit N]   # ranked results with snippets (pipe-friendly)
lucid extract <path> -o dir               # unpack topics + assets
lucid demo <out.mshc>                     # write the demo package
lucid --version

GUI-only flags:
  --pane <name>        start in contents | index | search | favorites
  --theme <name>       dark | light (overrides the saved preference for this session)
  --size <WxH>         exact window size, e.g. 1280x800 (implies windowed, not maximized)
  --no-maximize        start windowed at the fallback size (1440x900)
  --screenshot <png>   draw the window, write a PNG, exit (layout verification)
  --frames <n>         frames to draw before the screenshot (default 6)
```

`--screenshot` exists so the layout can be checked without a human at the keyboard: the
example below writes five images, one per window size, and the contrast/layout audit in
`PLAN.md` S3 runs over them.

```
for size in 900x600 1280x800 1920x1080; do
  lucid testdata/demo-help.mshc --size $size --screenshot /tmp/lucid-$size.png
done
lucid testdata/demo-help.mshc --screenshot /tmp/lucid-maximized.png
```

Exit codes: 0 ok, 1 IO/format error, 2 bad usage, 3 topic/keyword not found.

---

## 7. Roadmap

- **v0.2** — in-process `.cab` support (MU/VS content packages) and `.mshi` catalogs;
  syntax highlighting for code snippets (syntect, lazy per language); export topic/whole
  book to PDF; print layout; CSS subset (`font-weight`, `color`, `text-align`, borders of
  `table.alert`); SVG assets (resvg).
- **v0.3** — semantic search (optional local embeddings + hybrid rerank), "similar topics",
  per-topic summaries; saved searches; multi-window; localization of the UI.
- **v0.4** — CHM/HxS import (ITSS/CHM decoding) so Help 1.x/2.x content can be opened and
  republished as MSHC; GUI authoring (a real DocMounter alternative).
- Cross-cutting: Nix flake + AppImage/deb packaging, fuzzing of the parser, benchmark
  suite (open 1 000-topic book, 100 k-topic content set) wired to `cargo bench`.
