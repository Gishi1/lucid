//! The built-in demo package: self-authored topics about Lucid itself.
//!
//! The demo exists so a fresh clone has something to open *and* so the viewer, the search index
//! and the parser have fixtures for every documented behaviour: a three-level TOC, an orphan
//! topic, deliberately non-alphabetical `TocOrder` values, note callouts, collapsible areas,
//! C#/VB/F# code snippets sharing a snippet group, `LanguageSpecificText`, `ms-xhelp://` and
//! anchor links, a wide table, nested lists, `<pre>`, a blockquote, an image referenced with an
//! uppercase path while the stored part is lowercase, a stylesheet, F1 keywords, and one topic
//! encoded in windows-1252. No third-party content is involved.

use crate::bmp;
use crate::manifest::escape_xml;
use crate::meta::encode_cp1252;

/// The stylesheet shared by all demo topics.
const STYLESHEET: &str = "\
/* Lucid demo topic stylesheet (self-authored, hand-written). */
body {
  font-family: system-ui, \"Segoe UI\", Roboto, sans-serif;
  line-height: 1.55;
  margin: 2rem auto;
  max-width: 52rem;
  color: #1f2430;
  background: #ffffff;
}
div.title {
  font-size: 1.6rem;
  font-weight: 600;
  margin: 0 0 1rem 0;
  padding-bottom: .4rem;
  border-bottom: 1px solid #d5dae2;
}
div.alert {
  border-left: 4px solid #3f7fbf;
  background: #eef4fb;
  padding: .75rem 1rem;
  border-radius: 6px;
  margin: 1rem 0;
}
div.alert.tip { border-left-color: #3f9f7f; background: #eefaf4; }
div.alert span.label {
  display: block;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: .04em;
  font-size: .78rem;
  margin-bottom: .25rem;
}
table { border-collapse: collapse; width: 100%; margin: 1rem 0; }
th, td { border: 1px solid #d5dae2; padding: .35rem .6rem; text-align: left; vertical-align: top; }
th { background: #f2f5f9; }
pre, CodeSnippet {
  display: block;
  background: #f6f8fa;
  border: 1px solid #e3e8ee;
  border-radius: 6px;
  padding: .75rem 1rem;
  overflow-x: auto;
  font-family: \"Hack\", ui-monospace, monospace;
  font-size: .92rem;
}
blockquote {
  margin: 1rem 0;
  padding-left: 1rem;
  border-left: 3px solid #cfd6df;
  color: #46506a;
}
img { max-width: 100%; }
";

struct DemoTopic {
    path: &'static str,
    title: &'static str,
    id: &'static str,
    parent: &'static str,
    order: i32,
    keywords: &'static str,
    f1: &'static str,
    description: &'static str,
    /// Written as windows-1252 with a `Content-Type` meta tag declaring it.
    cp1252: bool,
    body: &'static str,
}

const TOPICS: &[DemoTopic] = &[
    DemoTopic {
        path: "topics/intro.htm",
        title: "Introduction to Lucid",
        id: "Lucid.Intro",
        parent: "-1",
        order: 10,
        keywords: "Lucid, overview, introduction",
        f1: "Lucid overview",
        description: "What Lucid is, which panes it has and where to start reading.",
        cp1252: false,
        body: "\
<p>Lucid is a native viewer for Microsoft Help Container (<code>.mshc</code>) documentation. It
opens the packages that ship with Visual Studio content as well as any book packed by
<code>lucid-forge</code>, and it needs no .NET runtime, no browser engine and no database.</p>
<h2 id=\"panes\">The four panes</h2>
<ul>
  <li><b>Contents</b> — the table of contents, built from the manifest when the package has one
      and from <code>Microsoft.Help.TocParent</code> tags otherwise.</li>
  <li><b>Index</b> — keywords, including the F1 keywords an application can send when the user
      presses F1.</li>
  <li><b>Search</b> — ranked full-text search with phrases, prefixes, filters and highlighting.
      See <a href=\"#details\">the details below</a>.</li>
  <li><b>Favorites</b> — bookmarks and history.</li>
</ul>
<h2>Where the data comes from</h2>
<table>
  <tr><th>Source</th><th>What Lucid reads</th></tr>
  <tr><td>Topic documents</td><td><code>Microsoft.Help.Id</code>, <code>TocParent</code>,
      <code>TocOrder</code>, keywords and the body itself</td></tr>
  <tr><td>Manifest (<code>*.msha</code>)</td><td>An optional explicit TOC and index</td></tr>
  <tr><td>Assets</td><td>Images and styles, resolved case-insensitively</td></tr>
</table>
<h2 id=\"details\">Reading order</h2>
<p>If you are new here, read <a href=\"./guides/getting-started/index.htm\">Getting started</a>
first; authors should jump to <a href=\"ms-xhelp:///?Id=Lucid.Reference.Markup\">Topic
markup</a>.</p>
<div class=\"alert\"><span class=\"label\">Note</span> Lucid never rewrites the package on disk.
Bookmarks, history and the search index live in your configuration directory instead.</div>
",
    },
    DemoTopic {
        path: "topics/guides/index.htm",
        title: "Guides",
        id: "Lucid.Guides",
        parent: "-1",
        order: 20,
        keywords: "guides, walkthrough",
        f1: "",
        description: "Task-oriented guides: installing, packing a book, searching and reading.",
        cp1252: false,
        body: "\
<p>These guides walk through the workflows Lucid was built for. Each one is a plain XHTML file
with <code>Microsoft.Help.*</code> meta tags, so you can unpack this package and see exactly what
the viewer reads.</p>
<ul>
  <li><a href=\"./getting-started/index.htm\">Getting started</a> — install Lucid, pack a first
      book, understand the layout of a package.</li>
  <li><a href=\"./search/index.htm\">Search</a> — the query language, phrase search and the fuzzy
      fallback.</li>
  <li><a href=\"./reader/index.htm\">Reading</a> — themes, zoom, images and keyboard shortcuts.</li>
  <li><a href=\"./cli.htm\">Command line</a> — headless <code>info</code>, <code>toc</code>,
      <code>search</code> and <code>extract</code>.</li>
</ul>
<div class=\"alert\"><span class=\"label\">Tip</span> Every guide ends with relative links, so you
can walk the whole book with the <code>Alt+Left</code> and <code>Alt+Right</code> history
keys.</div>
<blockquote>Authors keep the HTML; Lucid keeps the index. Nothing in between is
generated.</blockquote>
<p>The reference section — starting at <a href=\"../reference/index.htm\">Reference</a> — documents
the metadata contract and the markup the renderer understands.</p>
",
    },
    DemoTopic {
        path: "topics/guides/getting-started/index.htm",
        title: "Getting started",
        id: "Lucid.Guides.GettingStarted",
        parent: "Lucid.Guides",
        order: 10,
        keywords: "getting started, first steps, install",
        f1: "Lucid getting started",
        description: "The short path from an empty machine to a book you can read and search.",
        cp1252: false,
        body: "\
<p>Three steps take you from nothing to a searchable book.</p>
<ol>
  <li>Install the binaries (<a href=\"./install.htm\">Installing Lucid</a>).</li>
  <li>Pack a folder of HTML into a package
      (<a href=\"./first-package.htm\">Your first package</a>).</li>
  <li>Open it, or open a content store with dozens of packages at once.</li>
</ol>
<h2>What a package contains</h2>
<ul>
  <li>Topic documents — <code>.htm</code>, <code>.html</code> or <code>.xhtml</code>.</li>
  <li>Assets — images, stylesheets, scripts. Referenced by URL, stored by path, matched
      case-insensitively.</li>
  <li>An optional <code>*.msha</code> manifest with an explicit table of contents.</li>
</ul>
<div class=\"alert\"><span class=\"label\">Note</span> Topic metadata beats the manifest: when the
two disagree, the TOC is rebuilt from the meta tags and the manifest is used as a cross-check.</div>
<h2>Next</h2>
<p>When the package opens, try the search box with <code>title:install</code> — the field filters
are documented in <a href=\"../search/index.htm\">Searching the index</a>.</p>
",
    },
    DemoTopic {
        path: "topics/guides/getting-started/install.htm",
        title: "Installing Lucid",
        id: "Lucid.Guides.GettingStarted.Install",
        parent: "Lucid.Guides.GettingStarted",
        order: 10,
        keywords: "install, setup, cargo",
        f1: "lucid install, install",
        description: "Install Lucid from a release archive or build it with cargo.",
        cp1252: false,
        body: "\
<p>Lucid ships as a single binary plus the optional packer, <code>lucid-forge</code>. Pick the
installation that matches your machine.</p>
<CollapsibleArea Expanded=\"True\" Title=\"Install with cargo (all platforms)\">
<CodeSnippet Language=\"C#\" DisplayLanguage=\"C#\" EnableCopyCode=\"true\" snippetGroup=\"install\">
cargo install --path crates/lucid
cargo install --path crates/lucid-forge</CodeSnippet>
<CodeSnippet Language=\"Visual Basic\" DisplayLanguage=\"Visual Basic\" EnableCopyCode=\"true\" snippetGroup=\"install\">
dotnet tool install --global lucid
lucid --version</CodeSnippet>
<CodeSnippet Language=\"F#\" DisplayLanguage=\"F#\" EnableCopyCode=\"true\" snippetGroup=\"install\">
nix develop
cargo run -p lucid -- --version</CodeSnippet>
</CollapsibleArea>
<p>Package managers use their own commands, and the help engine substitutes the right snippet for
the code language you selected — <LanguageSpecificText devLangcs=\"C#\" devLangvb=\"Visual Basic\"
devLangcpp=\"C++\" devLangnu=\"your language\" /> above.</p>
<h2>Verify the installation</h2>
<pre>lucid-forge --version
lucid --help</pre>
<h2>Requirements</h2>
<table>
  <tr><th>Platform</th><th>Runtime needed</th><th>Notes</th></tr>
  <tr><td>Linux</td><td>X11 or Wayland</td><td>No GTK, no WebKit</td></tr>
  <tr><td>Windows</td><td>None</td><td>GPU driver for the window</td></tr>
  <tr><td>macOS</td><td>None</td><td>Metal through the window backend</td></tr>
</table>
<p>Continue with <a href=\"./first-package.htm\">Your first package</a>, or read the command line
reference at <a href=\"ms-xhelp:///?Id=Lucid.Guides.Cli\">Command line</a>.</p>
",
    },
    DemoTopic {
        path: "topics/guides/getting-started/first-package.htm",
        title: "Your first package",
        id: "Lucid.Guides.GettingStarted.FirstPackage",
        parent: "Lucid.Guides.GettingStarted",
        order: 20,
        keywords: "package, mshc, forge, publishing",
        f1: "lucid forge",
        description: "Turn a folder of topic HTML into a package you can open and publish.",
        cp1252: false,
        body: "\
<p>The packer reads a directory, copies it verbatim and adds the two parts that make it a real
Microsoft Help Container: an Open Packaging Conventions content-type map and a manifest with the
table of contents and the index.</p>
<pre>docs-src/
  topics/intro.htm
  topics/reference/*.htm
  assets/logo.bmp
  assets/theme.css</pre>
<p>Packing is one command:</p>
<CodeSnippet Language=\"C#\" DisplayLanguage=\"C#\" EnableCopyCode=\"true\">
lucid-forge build docs-src -o docs.mshc --title \"My Product\"</CodeSnippet>
<CollapsibleArea Expanded=\"True\" Title=\"What the packer writes\">
<ul>
  <li><code>[Content_Types].xml</code> — one default per extension, plus an override for the
      manifest.</li>
  <li><code>_rels/.rels</code> — the relationship that points at the manifest part.</li>
  <li><code>my-product.msha</code> — the <code>HelpContentSet</code> document.</li>
  <li>Every topic and asset, unchanged.</li>
</ul>
</CollapsibleArea>
<blockquote>A package is a zip archive with a different extension. If you can read a zip, you can
inspect a package — and so can Lucid's validator.</blockquote>
<div class=\"alert\"><span class=\"label\">Note</span> Two builds from the same folder produce
byte-identical archives: entry order and timestamps are fixed. That makes packages easy to
diff and to publish.</div>
<p>When the package validates, open it or hand it to Help Viewer. The rules the validator applies
are listed in <a href=\"../../reference/validation.htm\">Validating packages</a>.</p>
",
    },
    DemoTopic {
        path: "topics/guides/search/index.htm",
        title: "Searching the index",
        id: "Lucid.Guides.Search",
        parent: "Lucid.Guides",
        order: 20,
        keywords: "search, index, bm25, ranking",
        f1: "searching the index",
        description: "How the search pane ranks results and how to narrow them down.",
        cp1252: false,
        body: "\
<p>The index is built once per package set and cached, so re-opening a book is instant. Ranking is
BM25F with per-field weights, which means a term in the title always beats a term buried in a
paragraph.</p>
<h2>Field weights</h2>
<table>
  <tr><th>Field</th><th>Weight</th><th>What lands there</th></tr>
  <tr><td>Title</td><td>8.0</td><td>The topic title</td></tr>
  <tr><td>Keywords</td><td>5.0</td><td>Index keywords and F1 keywords</td></tr>
  <tr><td>Headings</td><td>3.0</td><td>Consecutive <code>h1</code>–<code>h6</code> text</td></tr>
  <tr><td>Body</td><td>1.0</td><td>Visible text</td></tr>
  <tr><td>Code</td><td>0.6</td><td><code>pre</code> and code snippets</td></tr>
</table>
<h2>Operators</h2>
<ul>
  <li>Terms
      <ul>
        <li><code>widget factory</code> — both terms, anywhere in the document.</li>
        <li><code>\"widget factory\"</code> — the exact phrase.</li>
        <li><code>widget*</code> — prefix match, used automatically while you type.</li>
        <li><code>widget~</code> — fuzzy match, ranked below an exact hit.</li>
        <li><code>-widget</code> — exclude.</li>
      </ul>
  </li>
  <li>Fields
      <ul>
        <li><code>title:</code>, <code>kw:</code>, <code>body:</code>, <code>code:</code>,
            <code>id:</code> and <code>path:</code> restrict a term to one field.</li>
      </ul>
  </li>
</ul>
<div class=\"alert\"><span class=\"label\">Tip</span> Press <code>Ctrl+K</code> from anywhere to
jump into the search box, and <code>Ctrl+P</code> to fuzzy-match topic titles when you already know
where you want to go.</div>
",
    },
    DemoTopic {
        path: "topics/guides/search/phrase.htm",
        title: "Phrase search",
        id: "Lucid.Guides.Search.Phrase",
        parent: "Lucid.Guides.Search",
        order: 10,
        keywords: "phrase search, search/phrase, quoted queries, ranking",
        f1: "phrase search",
        description: "Quoted queries match adjacent words in one field; ranking prefers titles.",
        cp1252: false,
        body: "\
<p>Wrap a query in double quotes to search for adjacent words. A phrase matches inside a single
field, so <code>\"help viewer\"</code> does not match a document that mentions <i>help</i> in one
paragraph and <i>viewer</i> in another.</p>
<h2 id=\"ranking\">Ranking a phrase</h2>
<p>Phrases are scored twice: once as the sum of their terms, and again as a bonus when they appear
where the reader is looking. The bonuses are listed below.</p>
<table>
  <tr><th>Where the phrase matches</th><th>Multiplier</th><th>Example query</th><th>Typical result
      </th><th>Field</th><th>Tie-break</th></tr>
  <tr><td>Title, exactly</td><td>&#215;3</td><td><code>\"phrase search\"</code></td>
      <td>this topic</td><td>title</td><td>shorter title wins</td></tr>
  <tr><td>Title, as part of</td><td>&#215;2</td><td><code>\"phrase\"</code></td>
      <td>this topic</td><td>title</td><td>title length, then title</td></tr>
  <tr><td>Body</td><td>&#215;1.35</td><td><code>\"adjacent words\"</code></td><td>this
      topic</td><td>body</td><td>document id</td></tr>
  <tr><td>Headings</td><td>&#215;1.2</td><td><code>\"ranking a phrase\"</code></td>
      <td>this topic</td><td>headings</td><td>document id</td></tr>
  <tr><td>Keywords</td><td>&#215;1.0</td><td><code>\"quoted queries\"</code></td>
      <td>this topic</td><td>keywords</td><td>document id</td></tr>
</table>
<blockquote>Ties are broken deterministically — score, then title length, then title, then
document id — so results never jitter between two identical queries.</blockquote>
<h2>Mixing phrases and terms</h2>
<pre>title:\"phrase search\" -chm
kw:search/phrase code:\"IndexBuilder::new\"</pre>
<p>See <a href=\"#ranking\">how a phrase is ranked</a> above, continue with
<a href=\"./fuzzy.htm\">Fuzzy matching</a>, or read the markup reference in
<a href=\"ms-xhelp:///?Id=Lucid.Reference.Markup\">Topic markup</a>.</p>
",
    },
    DemoTopic {
        path: "topics/guides/search/fuzzy.htm",
        title: "Fuzzy matching",
        id: "Lucid.Guides.Search.Fuzzy",
        parent: "Lucid.Guides.Search",
        order: 20,
        keywords: "fuzzy, typo, suggestions, did you mean",
        f1: "fuzzy match",
        description: "Misspelled queries fall back to prefix, then fuzzy, then spelling tips.",
        cp1252: false,
        body: "\
<p>A query that matches nothing is not a dead end. Lucid walks a fallback chain and tells you what
it did, so a typo costs one keystroke instead of a rewrite.</p>
<ol>
  <li>Exact terms, phrases and filters.</li>
  <li>Prefix expansion of the last term.</li>
  <li>Fuzzy match within an edit distance of two, ranked at half weight.</li>
  <li>Spelling suggestions drawn from the term dictionary — the \"did you mean\" line under the
      result list.</li>
</ol>
<table>
  <tr><th>You typed</th><th>Lucid tries</th><th>Result list</th></tr>
  <tr><td><code>serch</code></td><td><code>serch~</code></td><td>Search topics, ranked by
      similarity</td></tr>
  <tr><td><code>instal</code></td><td><code>instal*</code></td><td>Install topics, prefix
      hits first</td></tr>
  <tr><td><code>asjkdl</code></td><td>nothing plausible</td><td>Empty list plus suggestions</td></tr>
</table>
<div class=\"alert\"><span class=\"label\">Note</span> Fuzzy hits are deliberately demoted: a
one-letter typo finds the topic, but a document that matches exactly always stays on top.</div>
<p>Phrase behaviour is documented in <a href=\"ms-xhelp:///?Id=Lucid.Guides.Search.Phrase\">Phrase
search</a>; typo tolerance for whole titles lives in quick-open (<code>Ctrl+P</code>).</p>
",
    },
    DemoTopic {
        path: "topics/guides/reader/index.htm",
        title: "Reading topics",
        id: "Lucid.Guides.Reader",
        parent: "Lucid.Guides",
        order: 30,
        keywords: "reader, rendering, images, tables",
        f1: "reading topics",
        description: "How the topic view renders headings, tables, images, notes and code.",
        cp1252: false,
        body: "\
<p>The topic view renders the XHTML subset that help content actually uses, plus the
<code>Microsoft.Help.*</code> behaviours. Scripts are ignored; nothing is ever executed.</p>
<img src=\"ASSETS/LOGO.BMP\" alt=\"The Lucid logo\" width=\"320\" height=\"120\" />
<h2>Supported blocks</h2>
<ul>
  <li>Headings, paragraphs, emphasis and inline code.</li>
  <li>Lists
      <ul>
        <li>Ordered and unordered.</li>
        <li>Nested to any depth, with the indentation you would expect.</li>
      </ul>
  </li>
  <li>Tables with wrapped cells and zebra striping.</li>
  <li>Code and blockquotes, as on this page.</li>
</ul>
<h2>Behaviours</h2>
<table>
  <tr><th>Markup</th><th>Rendered as</th></tr>
  <tr><td><code>div.title</code></td><td>The topic heading at the top of the page</td></tr>
  <tr><td><code>div.alert</code> + <code>span.label</code></td><td>A callout box</td></tr>
  <tr><td><code>CollapsibleArea</code></td><td>A section you can fold away</td></tr>
  <tr><td><code>CodeSnippet</code></td><td>A code block with a copy button and language tabs</td></tr>
  <tr><td><code>LanguageSpecificText</code></td><td>Text substituted for your code language</td></tr>
</table>
<div class=\"alert tip\"><span class=\"label\">Tip</span> The image above is referenced as
<code>ASSETS/LOGO.BMP</code> while the stored part is <code>assets/logo.bmp</code> — exactly the
case mismatch real packages contain. Asset lookup is always case-insensitive.</div>
<p>Continue with <a href=\"./themes.htm\">Themes and zoom</a> or
<a href=\"./keyboard.htm\">Keyboard shortcuts</a>.</p>
",
    },
    DemoTopic {
        path: "topics/guides/reader/themes.htm",
        title: "Themes and zoom",
        id: "Lucid.Guides.Reader.Themes",
        parent: "Lucid.Guides.Reader",
        order: 10,
        keywords: "theme, dark mode, light mode, zoom, font size",
        f1: "theme",
        description: "Switch between the dark and the light theme, and scale the document.",
        cp1252: false,
        body: "\
<p>Lucid ships one theme with two palettes. The document surface is deliberately a little
different from the chrome, so the page you are reading stands out from the panes around it.</p>
<table>
  <tr><th>Token</th><th>Dark</th><th>Light</th></tr>
  <tr><td>Surface</td><td>#1b1f27</td><td>#f7f8fa</td></tr>
  <tr><td>Document</td><td>#22262f</td><td>#ffffff</td></tr>
  <tr><td>Accent</td><td>#5aa9e6</td><td>#2b6cb0</td></tr>
  <tr><td>Hairline</td><td>#2f3542</td><td>#dfe3ea</td></tr>
</table>
<h2>Keyboard control</h2>
<pre>Ctrl+=    zoom in
Ctrl+-    zoom out
Ctrl+0    reset to 100%
Ctrl+scroll  zoom around the pointer</pre>
<p>Font size, theme and pane widths are stored in the configuration directory, so a new session
starts where the last one ended. Publisher stylesheets are read for their colours where they map
onto the tokens above; the rest of the cascade is Lucid's.</p>
<div class=\"alert\"><span class=\"label\">Note</span> Branding packages are not applied — Lucid
always renders with its own theme, which keeps every book looking consistent.</div>
<p>Next: <a href=\"./keyboard.htm\">Keyboard shortcuts</a>.</p>
",
    },
    DemoTopic {
        path: "topics/guides/reader/keyboard.htm",
        title: "Keyboard shortcuts",
        id: "Lucid.Guides.Reader.Keyboard",
        parent: "Lucid.Guides.Reader",
        order: 20,
        keywords: "keyboard, shortcuts, accessibility",
        f1: "keyboard shortcuts",
        description: "Every shortcut the viewer responds to, in one table.",
        cp1252: false,
        body: "\
<p>Lucid is keyboard-first: every pane can be reached and every action can be triggered without
touching the mouse.</p>
<table>
  <tr><th>Key</th><th>Action</th><th>Pane</th><th>Notes</th><th>Repeatable</th></tr>
  <tr><td><code>Ctrl+O</code></td><td>Open a file, folder or setup document</td><td>Any</td>
      <td>Drag and drop works too</td><td>Yes</td></tr>
  <tr><td><code>Ctrl+K</code></td><td>Focus the search box</td><td>Search</td>
      <td>Results update while you type</td><td>Yes</td></tr>
  <tr><td><code>Ctrl+P</code></td><td>Quick-open by topic title</td><td>Any</td>
      <td>Fuzzy match over every package</td><td>Yes</td></tr>
  <tr><td><code>Ctrl+F</code></td><td>Find inside the topic</td><td>Document</td>
      <td><code>n</code> and <code>N</code> step through matches</td><td>Yes</td></tr>
  <tr><td><code>Ctrl+D</code></td><td>Bookmark the topic</td><td>Favorites</td>
      <td>Folders keep long lists tidy</td><td>No</td></tr>
  <tr><td><code>Alt+Left</code></td><td>Back</td><td>Any</td><td>Per content set</td><td>Yes</td></tr>
  <tr><td><code>F5</code></td><td>Rebuild the index</td><td>Any</td>
      <td>Normally automatic</td><td>No</td></tr>
</table>
<CollapsibleArea Expanded=\"False\" Title=\"Mouse and trackpad equivalents\">
<ul>
  <li>Middle-click a TOC entry to open it in a new tab.</li>
  <li>Ctrl+scroll zooms the document around the pointer.</li>
  <li>Double-click a search result to open it and close the overlay.</li>
</ul>
</CollapsibleArea>
<div class=\"alert\"><span class=\"label\">Tip</span> Press <code>?</code> in an empty search box
to list the shortcuts without leaving the document.</div>
",
    },
    DemoTopic {
        path: "topics/guides/cli.htm",
        title: "Command line",
        id: "Lucid.Guides.Cli",
        parent: "Lucid.Guides",
        order: 40,
        keywords: "cli, headless, automation, scripting",
        f1: "lucid command line",
        description: "Headless commands for scripts, editors and build systems.",
        cp1252: false,
        body: "\
<p>The viewer binary doubles as a headless tool, and the packer is its own binary. Output is plain
text with stable ordering, so it pipes into anything.</p>
<CodeSnippet Language=\"C#\" DisplayLanguage=\"C#\" EnableCopyCode=\"true\">
lucid info docs.mshc
lucid toc docs.mshc --depth 3
lucid search docs.mshc \"phrase search\" --limit 10
lucid extract docs.mshc -o out/</CodeSnippet>
<table>
  <tr><th>Command</th><th>Purpose</th><th>Exit code on success</th></tr>
  <tr><td><code>lucid info</code></td><td>Packages, topics, TOC and index statistics</td><td>0</td>
  </tr>
  <tr><td><code>lucid toc</code></td><td>ASCII table of contents</td><td>0</td></tr>
  <tr><td><code>lucid search</code></td><td>Ranked results with snippets</td><td>0</td></tr>
  <tr><td><code>lucid extract</code></td><td>Unpack topics and assets</td><td>0</td></tr>
  <tr><td><code>lucid-forge validate</code></td><td>Structure and metadata report</td><td>0</td></tr>
</table>
<h2>Context-sensitive help</h2>
<pre>lucid --topic Lucid.Guides.Cli
lucid --search \"command line\"</pre>
<p>The first form opens the window straight at a topic id, which is what an application should do
when the user presses F1. The ids are the ones in
<a href=\"ms-xhelp:///?Id=Lucid.Reference.Metadata\">Metadata reference</a>.</p>
",
    },
    DemoTopic {
        path: "topics/reference/index.htm",
        title: "Reference",
        id: "Lucid.Reference",
        parent: "-1",
        order: 30,
        keywords: "reference, contract, format",
        f1: "",
        description: "The format contract: topic markup, metadata, packing and validation.",
        cp1252: false,
        body: "\
<p>The reference documents the contract between authoring tools and the viewer. Everything here is
read from the package itself — no side-car files, no build step at open time.</p>
<ul>
  <li><a href=\"./markup.htm\">Topic markup</a> — every behaviour the renderer implements.</li>
  <li><a href=\"./metadata.htm\">Metadata reference</a> — the <code>Microsoft.Help.*</code> tags
      and what Lucid does with each one.</li>
  <li><a href=\"./locales.htm\">Locales and text encodings</a> — UTF-8, windows-1252 and locale
      matching.</li>
  <li><a href=\"./forge.htm\">Packing with lucid-forge</a> — what the packer writes, part by
      part.</li>
  <li><a href=\"./validation.htm\">Validating packages</a> — the rules and their severity.</li>
</ul>
<blockquote>The table of contents is always reconstructible from the topic metadata alone. The
manifest is an optimisation, not the source of truth.</blockquote>
",
    },
    DemoTopic {
        path: "topics/reference/markup.htm",
        title: "Topic markup",
        id: "Lucid.Reference.Markup",
        parent: "Lucid.Reference",
        order: 10,
        keywords: "markup, behaviours, codesnippet, collapsible area, alert",
        f1: "topic markup",
        description: "Every element the topic renderer understands, with a live example of each.",
        cp1252: false,
        body: "\
<p>This page is a fixture: it uses every behaviour the renderer implements, so a rendering bug
shows up as a broken page instead of a silent fallback.</p>
<h2 id=\"callouts\">Callouts</h2>
<div class=\"alert\"><span class=\"label\">Note</span> A <code>div.alert</code> with a
<code>span.label</code> renders as a note.</div>
<div class=\"alert tip\"><span class=\"label\">Tip</span> Add a second class — <code>tip</code>,
<code>warning</code> or <code>caution</code> — to change the accent colour.</div>
<h2>Collapsible sections</h2>
<CollapsibleArea Expanded=\"True\" Title=\"Why collapsible areas exist\">
<p>Help files are read in two modes: skimming and hunting. A collapsible area keeps the detail on
the page without shouting at the skimmer.</p>
<ul>
  <li>Expanded state is a hint, not a promise.
      <ul>
        <li>The reader's choice wins.
            <ul>
              <li>Which is why the state is stored per topic.</li>
            </ul>
        </li>
      </ul>
  </li>
</ul>
</CollapsibleArea>
<h2>Code</h2>
<p>Three snippets share a snippet group, so the renderer turns them into language tabs; the
copy button is controlled by <code>EnableCopyCode</code>.</p>
<CodeSnippet Language=\"C#\" DisplayLanguage=\"C#\" EnableCopyCode=\"true\" snippetGroup=\"render\">
var renderer = new TopicRenderer(theme);
renderer.Load(\"topics/intro.htm\");
renderer.Render();</CodeSnippet>
<CodeSnippet Language=\"Visual Basic\" DisplayLanguage=\"Visual Basic\" EnableCopyCode=\"true\" snippetGroup=\"render\">
Dim renderer As New TopicRenderer(theme)
renderer.Load(\"topics/intro.htm\")
renderer.Render()</CodeSnippet>
<CodeSnippet Language=\"F#\" DisplayLanguage=\"F#\" EnableCopyCode=\"true\" snippetGroup=\"render\">
let renderer = TopicRenderer theme
renderer.Load \"topics/intro.htm\"
renderer.Render()</CodeSnippet>
<pre>&lt;CodeSnippet Language=\"C#\" EnableCopyCode=\"true\" snippetGroup=\"render\"&gt;</pre>
<p>Inline text can follow the reader's language as well: today that is
<LanguageSpecificText devLangcs=\"C#\" devLangvb=\"Visual Basic\" devLangcpp=\"C++\"
devLangnu=\"plain text\" />, and the same substitution works inside a table cell, like
<LanguageSpecificText devLangcs=\"Console.WriteLine\" devLangvb=\"Console.WriteLine\"
devLangcpp=\"std::cout &amp;lt;&amp;lt;\" devLangnu=\"print\" /> in the table below.</p>
<table>
  <tr><th>Behaviour</th><th>Attribute</th><th>Values</th><th>Where it shows</th></tr>
  <tr><td rowspan=\"2\"><code>CodeSnippet</code></td><td><code>Language</code></td><td>C#, Visual
      Basic, F#</td><td>Language tab label</td></tr>
  <tr><td><code>snippetGroup</code></td><td>any string</td><td>Groups tabs together</td></tr>
  <tr><td><code>LanguageSpecificText</code></td><td><code>devLangcs</code></td>
      <td><LanguageSpecificText devLangcs=\"C# code\" devLangvb=\"VB code\" devLangcpp=\"C++ code\"
      devLangnu=\"neutral code\" /></td><td>Inline substitution</td></tr>
  <tr><td><code>CollapsibleArea</code></td><td><code>Expanded</code></td><td>True, False</td>
      <td>Initial fold state</td></tr>
</table>
<h2>Images and quotes</h2>
<img src=\"ASSETS/LOGO.BMP\" alt=\"Lucid logo\" width=\"320\" height=\"120\" />
<blockquote>Images are resolved case-insensitively from the package, decoded lazily and cached per
topic; the alt text is used when a part is missing.</blockquote>
<h2>Anchors and links</h2>
<p>Jump back to <a href=\"#callouts\">the callouts</a>, follow a relative link to
<a href=\"./metadata.htm\">Metadata reference</a>, or cross-link by id to
<a href=\"ms-xhelp:///?Id=Lucid.Guides.Search.Phrase\">Phrase search</a>.</p>
<CollapsibleArea Expanded=\"False\" Title=\"Inherited members\">
<div class=\"InheritedMember\">Inherited members: <a href=\"ms-xhelp:///?Id=Lucid.Reference.Metadata\">
Microsoft.Help metadata</a>, <a href=\"ms-xhelp:///?Id=Lucid.Reference.Validation\">validation
rules</a></div>
</CollapsibleArea>
",
    },
    DemoTopic {
        path: "topics/reference/metadata.htm",
        title: "Metadata reference",
        id: "Lucid.Reference.Metadata",
        parent: "Lucid.Reference",
        order: 20,
        keywords: "metadata, meta tags, tocparent, tocorder, f1",
        f1: "help metadata, metadata",
        description: "The Microsoft.Help.* meta tags Lucid reads, and the fallbacks for each.",
        cp1252: false,
        body: "\
<p>Topic metadata is the contract: everything the Contents and Index panes show can be rebuilt
from these tags. Names are matched case-insensitively and without namespaces.</p>
<table>
  <tr><th>Meta name</th><th>Required</th><th>Meaning</th><th>Fallback when missing</th></tr>
  <tr><td><code>Microsoft.Help.Id</code></td><td>Yes</td><td>Topic identity</td>
      <td>The file name, when the packer is told to allow it</td></tr>
  <tr><td><code>Microsoft.Help.TocParent</code></td><td>No</td><td>Parent topic id</td>
      <td>A TOC root</td></tr>
  <tr><td><code>Microsoft.Help.TocOrder</code></td><td>No</td><td>Sort key among siblings</td>
      <td>Zero, then title order</td></tr>
  <tr><td><code>Microsoft.Help.Keywords</code></td><td>No</td><td>Index keywords, comma
      separated</td><td>The Index pane simply skips the topic</td></tr>
  <tr><td><code>Microsoft.Help.F1</code></td><td>No</td><td>F1 keywords</td>
      <td>F1 falls back to the id</td></tr>
  <tr><td><code>Microsoft.Help.Locale</code></td><td>No</td><td>Language of the topic</td>
      <td>The generic <code>Language</code> meta, then the package locale</td></tr>
  <tr><td><code>Description</code></td><td>No</td><td>Summary for tooltips and indexing</td>
      <td>Nothing is shown</td></tr>
</table>
<h2>A complete head</h2>
<pre>&lt;head&gt;
  &lt;title&gt;Installing Lucid&lt;/title&gt;
  &lt;meta name=\"Microsoft.Help.Id\" content=\"Lucid.Guides.GettingStarted.Install\" /&gt;
  &lt;meta name=\"Microsoft.Help.TocParent\" content=\"Lucid.Guides.GettingStarted\" /&gt;
  &lt;meta name=\"Microsoft.Help.TocOrder\" content=\"10\" /&gt;
  &lt;meta name=\"Microsoft.Help.Keywords\" content=\"install, setup\" /&gt;
  &lt;meta name=\"Microsoft.Help.F1\" content=\"lucid install\" /&gt;
&lt;/head&gt;</pre>
<div class=\"alert\"><span class=\"label\">Note</span> A title may come from
<code>&lt;meta name=\"Title\"&gt;</code> instead of the <code>title</code> element; some authoring
tools emit both, and the meta tag wins.</div>
<p>Articles that carry no keywords simply do not appear in the index — see
<a href=\"ms-xhelp:///?Id=Lucid.Reference.Validation\">Validating packages</a> for the checks that
catch the interesting mistakes.</p>
",
    },
    DemoTopic {
        path: "topics/reference/locales.htm",
        title: "Locales and text encodings",
        id: "Lucid.Reference.Locales",
        parent: "Lucid.Reference",
        order: 25,
        keywords: "locale, encoding, windows-1252, utf-8",
        f1: "text encoding",
        description: "Déjà vu: this topic is stored as windows-1252, with quotes “ ”, an em dash — \
                     and the € sign in it.",
        cp1252: true,
        body: "\
<p>Real packages are not all UTF-8. This topic is stored as windows-1252 — the code page the
Microsoft Help tools wrote for years — and declares it in a <code>Content-Type</code> meta tag. Its
accents (café, naïve, déjà vu), curly quotes “like these”, the em dash — and the € sign all survive
because the viewer decodes the declared code page instead of guessing.</p>
<h2>What the decoder does</h2>
<table>
  <tr><th>Signal</th><th>Result</th></tr>
  <tr><td>A UTF-8 or UTF-16 byte order mark</td><td>The mark wins over everything else</td></tr>
  <tr><td><code>charset=windows-1252</code></td><td>Windows-1252 decoding</td></tr>
  <tr><td><code>charset=utf-8</code></td><td>UTF-8, with replacement characters for broken
      bytes</td></tr>
  <tr><td>Nothing declared, bytes are valid UTF-8</td><td>UTF-8</td></tr>
  <tr><td>Nothing declared, bytes are not UTF-8</td><td>Windows-1252 — a superset of
      ISO-8859-1 that cannot fail</td></tr>
</table>
<div class=\"alert\"><span class=\"label\">Note</span> A package may mix encodings from topic to
topic; each document is decoded on its own.</div>
<p>Quotes: “curly”, ‘single’, «guillemets». Accents: á à â ä å ç è é ê ë í î ï ñ ó ô ö ø ú ü ß. Signs:
— – … • ™ € £ ¥.</p>
",
    },
    DemoTopic {
        path: "topics/reference/forge.htm",
        title: "Packing with lucid-forge",
        id: "Lucid.Reference.Forge",
        parent: "Lucid.Reference",
        order: 30,
        keywords: "forge, pack, mshc, build, deterministic",
        f1: "lucid-forge",
        description: "The packer's command line, the parts it writes and why output is stable.",
        cp1252: false,
        body: "\
<p><code>lucid-forge</code> is a library and a small command line tool. <code>build</code> packs a
directory, <code>demo</code> writes this package, and <code>validate</code> and <code>outline</code>
inspect a package without opening the window.</p>
<CodeSnippet Language=\"C#\" DisplayLanguage=\"C#\" EnableCopyCode=\"true\">
lucid-forge build docs-src -o docs.mshc --title \"My Product\" --locale en-us
lucid-forge validate docs.mshc
lucid-forge outline docs.mshc</CodeSnippet>
<CollapsibleArea Expanded=\"True\" Title=\"Part order in the archive\">
<ol>
  <li>The manifest (<code>&lt;name&gt;.msha</code>).</li>
  <li><code>[Content_Types].xml</code>.</li>
  <li><code>_rels/.rels</code>.</li>
  <li>Topics, sorted by path.</li>
  <li>Assets, sorted by path.</li>
</ol>
</CollapsibleArea>
<h2>Options that change the shape of a package</h2>
<table>
  <tr><th>Option</th><th>Effect</th><th>Why you would use it</th></tr>
  <tr><td><code>--no-manifest</code></td><td>No <code>.msha</code> part</td>
      <td>Testing meta-tag-only TOC reconstruction</td></tr>
  <tr><td><code>--no-opc</code></td><td>No <code>[Content_Types].xml</code>, no
      <code>_rels/.rels</code></td><td>Minimal packages for the parser</td></tr>
  <tr><td><code>--allow-missing-ids</code></td><td>Ids derived from file names</td>
      <td>Legacy HTML that never had help metadata</td></tr>
  <tr><td><code>--preserve-times</code></td><td>Source timestamps instead of 2020-01-01</td>
      <td>Mirroring a vendor's archive byte for byte</td></tr>
</table>
<div class=\"alert\"><span class=\"label\">Note</span> Without <code>--preserve-times</code> every
entry carries the same fixed timestamp. Together with the stable part order that makes two builds
of one folder byte-identical — a test hashes both archives to prove it.</div>
",
    },
    DemoTopic {
        path: "topics/reference/validation.htm",
        title: "Validating packages",
        id: "Lucid.Reference.Validation",
        parent: "Lucid.Reference",
        order: 40,
        keywords: "validate, diagnostics, errors, warnings",
        f1: "validate package",
        description: "The checks the validator runs, and which problems are fatal.",
        cp1252: false,
        body: "\
<p>A package can be broken in ways that the viewer tolerates and Help Viewer does not, so
validation sorts problems into two buckets: errors that make the package invalid, and warnings for
quirks worth knowing about.</p>
<table>
  <tr><th>Check</th><th>Severity</th><th>Why it matters</th></tr>
  <tr><td>The zip opens and the parts can be read</td><td>Error</td>
      <td>Nothing else is possible otherwise</td></tr>
  <tr><td>The package contains at least one topic</td><td>Error</td><td>An empty book</td></tr>
  <tr><td>Every topic has a <code>Microsoft.Help.Id</code></td><td>Error</td>
      <td>Ids are the target of every cross-link</td></tr>
  <tr><td>No two topics share an id</td><td>Error</td>
      <td>Links would jump to the wrong topic</td></tr>
  <tr><td>No topic is zero bytes long</td><td>Error</td><td>A blank page</td></tr>
  <tr><td>Every manifest <code>Url</code> points at an existing part</td><td>Error</td>
      <td>A dangling entry in the Contents pane</td></tr>
  <tr><td>The manifest lists every topic the meta tags describe</td><td>Error</td>
      <td>The two views of the book must agree</td></tr>
  <tr><td>Every <code>TocParent</code> resolves</td><td>Warning</td>
      <td>The topic becomes a TOC root instead of vanishing</td></tr>
  <tr><td>Every topic has a title</td><td>Warning</td>
      <td>The id is used as a label</td></tr>
  <tr><td>Keywords are not empty strings</td><td>Warning</td>
      <td>The Index pane would show a blank row</td></tr>
  <tr><td>The OPC parts are present</td><td>Warning</td>
      <td>Lucid does not need them, Help Viewer does</td></tr>
</table>
<pre>lucid-forge validate docs.mshc
docs.mshc: OK - 19 topics, 2 assets, 48 keywords</pre>
<div class=\"alert\"><span class=\"label\">Note</span> This demo package validates cleanly: the one
warning it reports is its deliberately orphaned topic, which the TOC faithfully promotes to a
root.</div>
<p>Rows in the table above map to the sections in
<a href=\"ms-xhelp:///?Id=Lucid.Reference.Metadata\">Metadata reference</a>.</p>
",
    },
    DemoTopic {
        path: "topics/orphan.htm",
        title: "Unlisted notes",
        id: "Lucid.Orphan",
        parent: "Lucid.DoesNotExist",
        order: 50,
        keywords: "orphan, notes, toc",
        f1: "orphan topic",
        description: "A topic whose TocParent does not exist: the TOC keeps it as a root.",
        cp1252: false,
        body: "\
<p>This topic points at a <code>Microsoft.Help.TocParent</code> that no topic in the package
defines — a real-world quirk of hand-edited and half-regenerated content. Lucid never drops a
topic: unreachable parents are promoted to TOC roots, and the validator reports the promotion as a
warning rather than an error.</p>
<blockquote>A missing parent is a broken promise, not a broken topic. The reader still gets the
page.</blockquote>
<h2>Why keep it in the demo</h2>
<ul>
  <li>The TOC builder has to handle the case every time a vendor ships it.</li>
  <li>The validator has to report it without failing the package.</li>
  <li>The search index has to find it even though nothing links to it.</li>
</ul>
<div class=\"alert\"><span class=\"label\">Tip</span> Search for <code>orphan</code> and this page
comes back, which is exactly the behaviour that makes orphaned content recoverable.</div>
<p>Related: <a href=\"ms-xhelp:///?Id=Lucid.Reference.Validation\">Validating packages</a> and
<a href=\"ms-xhelp:///?Id=Lucid.Reference.Metadata\">Metadata reference</a>.</p>
",
    },
];

/// The payload of the demo package: topics and assets, sorted by part path.
///
/// The manifest and the OPC parts are added by [`crate::demo_package`], so this is exactly the
/// content that would sit in a source folder.
pub fn demo_files() -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = TOPICS
        .iter()
        .map(|topic| (topic.path.to_owned(), render(topic)))
        .collect();
    files.push(("assets/logo.bmp".to_owned(), bmp::logo(320, 120)));
    files.push((
        "assets/theme.css".to_owned(),
        STYLESHEET.as_bytes().to_vec(),
    ));
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// The build options the demo is written with.
pub fn demo_options() -> crate::options::BuildOptions {
    crate::options::BuildOptions {
        title: Some("Lucid Help".to_owned()),
        ..crate::options::BuildOptions::default()
    }
}

fn render(topic: &DemoTopic) -> Vec<u8> {
    let charset = if topic.cp1252 {
        "windows-1252"
    } else {
        "utf-8"
    };
    let title = escape_xml(topic.title);
    let id = escape_xml(topic.id);
    let parent = escape_xml(topic.parent);
    let keywords = escape_xml(topic.keywords);
    let f1 = escape_xml(topic.f1);
    let description = escape_xml(topic.description);
    let order = topic.order;
    let body = topic.body;
    // An empty keyword list is left out rather than declared empty: a meta tag with no content
    // is one of the things the validator warns about.
    let keywords_meta = if topic.keywords.is_empty() {
        String::new()
    } else {
        format!("<meta name=\"Microsoft.Help.Keywords\" content=\"{keywords}\" />\n")
    };
    let f1_meta = if topic.f1.is_empty() {
        String::new()
    } else {
        format!("<meta name=\"Microsoft.Help.F1\" content=\"{f1}\" />\n")
    };
    let html = format!(
        "<?xml version=\"1.0\" encoding=\"{charset}\"?>\n\
<html xmlns=\"http://www.w3.org/1999/xhtml\">\n\
<head>\n\
<meta http-equiv=\"Content-Type\" content=\"text/html; charset={charset}\" />\n\
<title>{title}</title>\n\
<meta name=\"Microsoft.Help.Id\" content=\"{id}\" />\n\
<meta name=\"Microsoft.Help.TocParent\" content=\"{parent}\" />\n\
<meta name=\"Microsoft.Help.TocOrder\" content=\"{order}\" />\n\
{keywords_meta}{f1_meta}<meta name=\"Microsoft.Help.Locale\" content=\"en-us\" />\n\
<meta name=\"Description\" content=\"{description}\" />\n\
<link rel=\"stylesheet\" type=\"text/css\" href=\"ASSETS/THEME.CSS\" />\n\
</head>\n\
<body>\n\
<div class=\"title\">{title}</div>\n\
{body}\
</body>\n\
</html>\n"
    );
    if topic.cp1252 {
        encode_cp1252(&html)
    } else {
        html.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::extract_meta_bytes;

    #[test]
    fn every_demo_topic_parses_and_has_an_id() {
        let files = demo_files();
        let topics: Vec<_> = files
            .iter()
            .filter(|(name, _)| name.ends_with(".htm"))
            .collect();
        assert!(topics.len() >= 14, "the demo needs at least 14 topics");
        for (name, bytes) in topics {
            let meta = extract_meta_bytes(bytes);
            assert!(meta.id.is_some(), "{name} has no id");
            assert!(meta.title.is_some(), "{name} has no title");
            assert!(meta.toc_parent.is_some(), "{name} has no TocParent");
            assert!(meta.toc_order.is_some(), "{name} has no TocOrder");
            assert!(!meta.keywords.is_empty(), "{name} has no keywords");
        }
    }

    #[test]
    fn demo_paths_are_unique_and_assets_are_lowercase() {
        let files = demo_files();
        let mut names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique);
        for (name, _) in &files {
            if !name.ends_with(".htm") {
                assert_eq!(*name, name.to_ascii_lowercase(), "{name} must be lowercase");
            }
        }
    }

    #[test]
    fn the_demo_uses_every_documented_behaviour() {
        let files = demo_files();
        let all: String = files
            .iter()
            .filter(|(name, _)| name.ends_with(".htm"))
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .collect();
        for needle in [
            "<div class=\"title\">",
            "div class=\"alert\"",
            "<span class=\"label\">",
            "<CollapsibleArea Expanded=",
            "<CodeSnippet Language=",
            "DisplayLanguage=",
            "EnableCopyCode=",
            "snippetGroup=",
            "<LanguageSpecificText devLangcs=",
            "devLangvb=",
            "devLangcpp=",
            "devLangnu=",
            "ms-xhelp:///?Id=",
            "href=\"#",
            "<table>",
            "<ul>",
            "<pre>",
            "<blockquote>",
            "<img src=\"ASSETS/LOGO.BMP\"",
            "href=\"ASSETS/THEME.CSS\"",
            "search/phrase",
        ] {
            assert!(all.contains(needle), "the demo never uses {needle}");
        }
    }

    #[test]
    fn the_demo_has_three_toc_levels_and_an_orphan() {
        let files = demo_files();
        let metas: Vec<_> = files
            .iter()
            .filter(|(name, _)| name.ends_with(".htm"))
            .map(|(name, bytes)| (name.clone(), extract_meta_bytes(bytes)))
            .collect();
        let ids: Vec<String> = metas
            .iter()
            .filter_map(|(_, meta)| meta.id.clone())
            .collect();
        // Lucid.Guides.GettingStarted.Install sits below two levels of grouping topics.
        let mut parent = Some("Lucid.Guides.GettingStarted.Install".to_owned());
        let mut depth = 0;
        while let Some(current) = parent {
            parent = metas
                .iter()
                .find(|(_, meta)| meta.id.as_deref() == Some(current.as_str()))
                .and_then(|(_, meta)| meta.toc_parent.clone())
                .filter(|parent| parent != "-1");
            depth += 1;
            assert!(depth < 10, "the demo TOC has a cycle");
        }
        assert_eq!(
            depth, 3,
            "three levels: Guides → Getting started → Installing Lucid"
        );
        assert!(metas
            .iter()
            .any(|(_, meta)| meta
                .toc_parent
                .as_deref()
                .is_some_and(|parent| !parent.is_empty()
                    && parent != "-1"
                    && !ids.contains(&parent.to_owned()))));
    }

    #[test]
    fn one_topic_is_windows_1252_encoded() {
        let files = demo_files();
        let (_, bytes) = files
            .iter()
            .find(|(name, _)| name == "topics/reference/locales.htm")
            .expect("the demo has a locales topic");
        assert!(std::str::from_utf8(bytes).is_err(), "it must not be UTF-8");
        let meta = extract_meta_bytes(bytes);
        assert_eq!(meta.charset.as_deref(), Some("windows-1252"));
        let description = meta.description.unwrap_or_default();
        assert!(description.contains('€'), "{description}");
        assert!(description.contains('é'), "{description}");
    }
}
