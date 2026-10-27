//! Topic HTML → block model.
//!
//! One parse is used twice: the viewer renders the model natively (see
//! `crates/lucid/src/view/render.rs`) and the search index consumes the plain-text views
//! produced here, so what you can search is exactly what you can see.
//!
//! Supported subset (see `docs/DESIGN.md` §2.3): headings, paragraphs, inline emphasis,
//! nested lists, tables, `<pre>`/`<code>`, blockquotes, rules, images, anchors, and the
//! Microsoft Help behaviours `CollapsibleArea`, `CodeSnippet`, `LanguageSpecificText`,
//! `div.title`, `div.alert` + `span.label` and `InheritedMember`.
//!
//! The older Sandcastle/Help Viewer 1.x markup that ships in real `.mshc` packages is
//! recognised too: `div.pageHeader`/`div.pageFooter` chrome, `table.titleTable`,
//! `div.codeSnippetContainer` language tabs, `div.collapsibleAreaRegion` +
//! `div.collapsibleSection`, `h1.heading` + `div[name=collapseableSection]`, and the
//! `AddLanguageSpecificTextSet("id?cs=…|nu=…")` separators written into `<script>` tags.

use std::collections::BTreeMap;

use crate::html::arena::{Arena, NodeId};

/// Maximum nesting depth we convert before falling back to flattened text (guards against
/// pathological input blowing the stack).
const MAX_DEPTH: usize = 200;

/// Inline content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Underline(Vec<Inline>),
    Strike(Vec<Inline>),
    Sub(Vec<Inline>),
    Sup(Vec<Inline>),
    Code(String),
    /// `[text](target)` — `target` is the raw `href` value.
    Link { target: String, inlines: Vec<Inline> },
    Image { src: String, alt: String },
    LineBreak,
    /// `<LanguageSpecificText devLangcs=… devLangvb=… devLangcpp=… devLangnu=…>`
    LanguageSpecific {
        cs: String,
        vb: String,
        cpp: String,
        nu: String,
    },
    /// `<span class="label">`.
    Label(Vec<Inline>),
}

/// A table cell.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub blocks: Vec<Block>,
    pub colspan: u32,
    pub rowspan: u32,
    pub align: Option<String>,
    pub is_header: bool,
}

/// A table row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub is_header: bool,
}

/// A table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub rows: Vec<Row>,
    pub header_rows: usize,
    pub caption: Option<String>,
}

/// A code block (`<pre>` or `CodeSnippet`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodeBlock {
    /// Machine language name from `Language="CSharp"`.
    pub language: Option<String>,
    /// Label shown on the tab, e.g. `DisplayLanguage="C#"`.
    pub display_language: Option<String>,
    /// Id of the enclosing snippet group, so the viewer can show language tabs.
    pub group: Option<String>,
    pub lines: Vec<String>,
    /// `EnableCopyCode="true"` (absent means "assume copyable" for `<pre>`).
    pub copyable: bool,
}

/// Block-level content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
        anchor: Option<String>,
    },
    Paragraph {
        inlines: Vec<Inline>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Table(Table),
    Code(CodeBlock),
    Note {
        label: Option<String>,
        blocks: Vec<Block>,
    },
    Collapsible {
        title: String,
        expanded: bool,
        blocks: Vec<Block>,
    },
    Quote {
        blocks: Vec<Block>,
    },
    Rule,
    Image {
        src: String,
        alt: String,
    },
    /// Position marker for `<a name="…">` / `<a id="…">` without text.
    Anchor(String),
}

/// A heading, as listed in the "On this page" outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    pub level: u8,
    pub text: String,
    pub anchor: String,
}

/// A parsed topic body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    /// Title found in the body (`div.title` or the first `<h1>`).
    pub title: Option<String>,
    pub blocks: Vec<Block>,
    pub outline: Vec<OutlineEntry>,
    pub anchors: Vec<String>,
    /// Visible text, for the search index.
    pub text: String,
    /// Headings + collapsible titles, for the search index.
    pub headings_text: String,
    /// Code text, for the search index.
    pub code_text: String,
    /// `lang` attribute of `<html>`.
    pub language: Option<String>,
}

/// Parse a topic body into the block model.
pub fn parse_document(html: &str) -> Document {
    let arena = Arena::parse(html);
    let mut conv = Convert {
        arena: &arena,
        title: None,
        anchors: Vec::new(),
        outline: Vec::new(),
        snippet_group: None,
        language_specific: collect_language_specific(html),
        depth: 0,
        skip_child: None,
    };
    let language = arena
        .find_first(arena.root, &|a, id| a.name(id) == Some("html"))
        .and_then(|id| arena.attr(id, "lang"))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let blocks = conv.blocks(arena.root);
    let mut doc = Document {
        title: conv.title,
        blocks,
        outline: conv.outline,
        anchors: conv.anchors,
        text: String::new(),
        headings_text: String::new(),
        code_text: String::new(),
        language,
    };
    let (text, headings, code) = collect_texts(&doc.blocks);
    doc.text = text;
    doc.headings_text = headings;
    doc.code_text = code;
    doc
}

/// Plain text of inline content.
pub fn inline_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    push_inline_text(inlines, &mut out);
    out
}

fn push_inline_text(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Bold(v)
            | Inline::Italic(v)
            | Inline::Underline(v)
            | Inline::Strike(v)
            | Inline::Sub(v)
            | Inline::Sup(v)
            | Inline::Label(v) => push_inline_text(v, out),
            Inline::Link { inlines, .. } => push_inline_text(inlines, out),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::LineBreak => out.push('\n'),
            Inline::LanguageSpecific { cs, nu, .. } => {
                out.push_str(if cs.is_empty() { nu } else { cs })
            }
        }
    }
}

/// Plain text of a block list (used by the search index and by `lucid info`).
pub fn blocks_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    push_blocks_text(blocks, &mut out);
    out
}

fn push_blocks_text(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Heading { inlines, .. } | Block::Paragraph { inlines } => {
                out.push_str(&inline_text(inlines));
                out.push('\n');
            }
            Block::List { items, .. } => {
                for item in items {
                    push_blocks_text(item, out);
                }
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        push_blocks_text(&cell.blocks, out);
                    }
                }
            }
            Block::Code(code) => {
                for line in &code.lines {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            Block::Note { blocks, .. } | Block::Quote { blocks } => push_blocks_text(blocks, out),
            Block::Collapsible { blocks, title, .. } => {
                out.push_str(title);
                out.push('\n');
                push_blocks_text(blocks, out);
            }
            Block::Image { alt, .. } => {
                if !alt.is_empty() {
                    out.push_str(alt);
                    out.push('\n');
                }
            }
            Block::Rule | Block::Anchor(_) => {}
        }
    }
}

fn collect_texts(blocks: &[Block]) -> (String, String, String) {
    let mut text = String::new();
    push_blocks_text(blocks, &mut text);
    let mut headings = String::new();
    collect_headings(blocks, &mut headings);
    let mut code = String::new();
    collect_code(blocks, &mut code);
    (
        tidy(&text),
        tidy(&headings),
        tidy_code(&code),
    )
}

fn collect_headings(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Heading { inlines, .. } => {
                out.push_str(&inline_text(inlines));
                out.push('\n');
            }
            Block::Collapsible { title, blocks, .. } => {
                out.push_str(title);
                out.push('\n');
                collect_headings(blocks, out);
            }
            Block::Note { blocks, .. } | Block::Quote { blocks } => collect_headings(blocks, out),
            Block::List { items, .. } => {
                for item in items {
                    collect_headings(item, out);
                }
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_headings(&cell.blocks, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_code(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Code(code) => {
                for line in &code.lines {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            Block::Note { blocks, .. } | Block::Quote { blocks } => collect_code(blocks, out),
            Block::Collapsible { blocks, .. } => collect_code(blocks, out),
            Block::List { items, .. } => {
                for item in items {
                    collect_code(item, out);
                }
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_code(&cell.blocks, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn tidy(s: &str) -> String {
    s.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tidy_code(s: &str) -> String {
    s.trim_matches('\n').to_owned()
}

/// Collapse whitespace runs into single spaces (viewer text uses this).
pub fn normalize_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Collapse whitespace runs but keep one leading/trailing space.
///
/// Inline text runs are concatenated (`Hello <b>world</b>`), so dropping the separator
/// between a text node and a formatting element would glue words together. The edges are
/// trimmed later, once per paragraph, by [`trim_inlines`].
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    if pending_space {
        out.push(' ');
    }
    out
}

/// Anchor id for a heading without an `id` attribute.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_dash = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for c in ch.to_lowercase() {
                out.push(c);
            }
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "section".to_owned()
    } else {
        out
    }
}

// ---------------------------------------------------------------------------------------
// Arena → model
// ---------------------------------------------------------------------------------------

/// Language-specific punctuation written by Help Viewer/Sandcastle.
///
/// The topic contains an empty placeholder and a script call:
///
/// ```html
/// <span id="LSTD3D7B158_0"></span>
/// <script>AddLanguageSpecificTextSet("LSTD3D7B158_0?cpp=::|nu=.");</script>
/// ```
///
/// Script bodies are not part of the DOM (the lexer keeps them out of the arena), so the
/// separators have to be recovered from the raw source before conversion.
fn collect_language_specific(html: &str) -> BTreeMap<String, Inline> {
    const CALL: &str = "AddLanguageSpecificTextSet(";
    let mut out = BTreeMap::new();
    let mut rest = html;
    while let Some(pos) = rest.find(CALL) {
        rest = &rest[pos + CALL.len()..];
        let quoted = rest.trim_start();
        let quote = match quoted.chars().next() {
            Some(quote @ ('"' | '\'')) => quote,
            _ => continue,
        };
        let body = &quoted[1..];
        let Some(end) = body.find(quote) else {
            continue;
        };
        let spec = &body[..end];
        // Only the `id?key=value|…` form belongs to a placeholder.
        let Some((id, values)) = spec.split_once('?') else {
            continue;
        };
        if id.trim().is_empty() {
            continue;
        }
        let (mut cs, mut vb, mut cpp, mut nu) = (String::new(), String::new(), String::new(), String::new());
        for pair in values.split('|') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let value = crate::html::decode_entities(value);
            match key.trim().to_ascii_lowercase().as_str() {
                "cs" => cs = value,
                "vb" => vb = value,
                "cpp" => cpp = value,
                "nu" => nu = value,
                _ => {}
            }
        }
        out.insert(
            id.trim().to_owned(),
            Inline::LanguageSpecific { cs, vb, cpp, nu },
        );
    }
    out
}

struct Convert<'a> {
    arena: &'a Arena,
    title: Option<String>,
    anchors: Vec<String>,
    outline: Vec<OutlineEntry>,
    snippet_group: Option<String>,
    /// Separators for `<span id="LST…">` placeholders (see [`collect_language_specific`]).
    language_specific: BTreeMap<String, Inline>,
    depth: usize,
    /// A child to skip while converting (see [`Convert::nested_except`]).
    skip_child: Option<NodeId>,
}

/// Elements that are never rendered from a topic body.
fn is_skipped(name: &str) -> bool {
    matches!(
        name,
        "head"
            | "script"
            | "style"
            | "noscript"
            | "iframe"
            | "object"
            | "embed"
            | "applet"
            | "form"
            | "input"
            | "button"
            | "select"
            | "textarea"
            | "map"
            | "area"
            | "link"
            | "meta"
            | "base"
            | "param"
            | "source"
            | "track"
            | "svg"
            | "video"
            | "audio"
            | "canvas"
            | "template"
            | "colgroup"
            | "col"
    )
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn is_inline_name(name: &str) -> bool {
    matches!(
        name,
        "a" | "b"
            | "strong"
            | "i"            | "em"
            | "cite"
            | "var"
            | "dfn"
            | "u"
            | "ins"
            | "s"
            | "strike"
            | "del"
            | "sub"
            | "sup"
            | "code"
            | "tt"
            | "kbd"
            | "samp"
            | "span"
            | "font"
            | "small"
            | "big"
            | "label"
            | "abbr"
            | "acronym"
            | "q"
            | "mark"
            | "time"
            | "nobr"
            | "wbr"
            | "img"
            | "br"
            | "languagespecifictext"
            | "languageelement"
            | "text"
    )
}

impl<'a> Convert<'a> {
    fn blocks(&mut self, node: NodeId) -> Vec<Block> {
        if self.depth > MAX_DEPTH {
            let text = self.arena.subtree_text(node, true);
            return if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![Block::Paragraph {
                    inlines: vec![Inline::Text(text)],
                }]
            };
        }
        let mut out: Vec<Block> = Vec::new();
        let mut pending: Vec<Inline> = Vec::new();
        let children = self.arena.children(node).to_vec();
        let mut index = 0;
        while index < children.len() {
            let child = children[index];
            index += 1;
            if Some(child) == self.skip_child {
                continue;
            }
            if self.arena.is_text(child) {
                // Edge spaces are kept: `</b> — the…` and `from <code>x</code> tags` must not
                // glue together. `flush` trims the paragraph's own edges afterwards.
                let text = collapse_ws(self.arena.text(child).unwrap_or(""));
                if !text.is_empty() {
                    pending.push(Inline::Text(text));
                }
                continue;
            }
            let name = match self.arena.name(child) {
                Some(n) => n.to_owned(),
                None => continue,
            };
            if is_skipped(&name) || self.is_chrome(child, &name) {
                continue;
            }
            // `<br>` can be a direct child of a block container (`<dd>`, `<td>`, `<div>`),
            // not only of a paragraph. Dropping it glued `…ICustomScan` to `object…`.
            if name == "br" {
                pending.push(Inline::LineBreak);
                continue;
            }

            // `div.collapsibleAreaRegion` is a *title*; the body lives in the immediately
            // following `div.collapsibleSection`. Pairing them keeps real reference pages
            // (Syntax / Remarks / Example / See Also) collapsible instead of printing the
            // title as loose text.
            if name == "div" && self.class_is(child, "collapsiblearearegion") {
                if let Some((section, offset)) = self.following_section(&children[index..]) {
                    flush(&mut out, &mut pending);
                    let title = self.region_title(child);
                    self.note_collapsible(&title);
                    let blocks = self.nested(section);
                    out.push(Block::Collapsible {
                        title,
                        expanded: true,
                        blocks,
                    });
                    index += offset + 1;
                    continue;
                }
            }

            // The Sandcastle flavour uses `<h1 class="heading">…</h1>` followed by
            // `<div name="collapseableSection">…</div>`.
            if is_heading(&name) && self.class_is(child, "heading") {
                if let Some((section, offset)) = self.following_named_section(&children[index..]) {
                    flush(&mut out, &mut pending);
                    let title = normalize_ws(&self.arena.subtree_text(child, true));
                    self.note_collapsible(&title);
                    let blocks = self.nested(section);
                    out.push(Block::Collapsible {
                        title,
                        expanded: true,
                        blocks,
                    });
                    index += offset + 1;
                    continue;
                }
            }

            // A language-specific separator placeholder (`<span id="LST…">`).
            if let Some(placeholder) = self.placeholder(child) {
                pending.push(placeholder);
                continue;
            }

            if let Some(mut blocks) = self.try_block_element(child, &name) {
                flush(&mut out, &mut pending);
                out.append(&mut blocks);
            } else {
                pending.extend(self.inlines(child));
            }
        }
        flush(&mut out, &mut pending);
        out
    }

    /// Template chrome that MSHC packages carry in every topic (running header/footer).
    fn is_chrome(&self, id: NodeId, name: &str) -> bool {
        if name != "div" {
            return false;
        }
        if let Some(value) = self.arena.id_attr(id) {
            let value = value.to_ascii_lowercase();
            if matches!(value.as_str(), "header" | "footer" | "pageheader" | "pagefooter") {
                return true;
            }
        }
        self.class_is(id, "pageheader") || self.class_is(id, "pagefooter")
    }

    /// Exact class-token match (`has_class` is a substring test).
    fn class_is(&self, id: NodeId, class: &str) -> bool {
        self.arena.classes(id).iter().any(|c| c == class)
    }

    /// The next element sibling, ignoring text nodes and ignored elements.
    fn next_element(&self, siblings: &[NodeId]) -> Option<(NodeId, usize)> {
        for (offset, sibling) in siblings.iter().copied().enumerate() {
            if self.arena.is_text(sibling) {
                continue;
            }
            let name = match self.arena.name(sibling) {
                Some(n) => n,
                None => continue,
            };
            if is_skipped(name) {
                continue;
            }
            return Some((sibling, offset));
        }
        None
    }

    /// A `div.collapsibleSection` directly after a `div.collapsibleAreaRegion`.
    fn following_section(&self, siblings: &[NodeId]) -> Option<(NodeId, usize)> {
        let (sibling, offset) = self.next_element(siblings)?;
        if self.arena.name(sibling) == Some("div")
            && self.class_is(sibling, "collapsiblesection")
        {
            Some((sibling, offset))
        } else {
            None
        }
    }

    /// A `div[name="collapseableSection"]` directly after a Sandcastle `h1.heading`.
    fn following_named_section(&self, siblings: &[NodeId]) -> Option<(NodeId, usize)> {
        let (sibling, offset) = self.next_element(siblings)?;
        if self.arena.name(sibling) == Some("div")
            && self.arena.attr_lc(sibling, "name").as_deref() == Some("collapseablesection")
        {
            Some((sibling, offset))
        } else {
            None
        }
    }

    /// The visible title of a `div.collapsibleAreaRegion`.
    fn region_title(&self, id: NodeId) -> String {
        self.arena
            .find_first(id, &|a, n| {
                a.name(n) == Some("span") && a.has_class(n, "collapsibleregiontitle")
            })
            .map(|n| normalize_ws(&self.arena.subtree_text(n, true)))
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.arena
                    .find_first(id, &|a, n| a.name(n) == Some("span"))
                    .map(|n| normalize_ws(&self.arena.subtree_text(n, true)))
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| "Details".to_owned())
    }

    /// The separator for an empty language-specific placeholder, if this node is one.
    fn placeholder(&self, id: NodeId) -> Option<Inline> {
        let id_attr = self.arena.id_attr(id)?;
        let inline = self.language_specific.get(id_attr)?;
        if self.arena.subtree_text(id, true).trim().is_empty() {
            Some(inline.clone())
        } else {
            None
        }
    }

    /// A collapsible section title takes part in the "On this page" outline.
    fn note_collapsible(&mut self, title: &str) {
        if title.is_empty() {
            return;
        }
        let anchor = slugify(title);
        if self.anchors.iter().any(|existing| existing == &anchor) {
            return;
        }
        self.note_anchor(&anchor);
        self.outline.push(OutlineEntry {
            level: 2,
            text: title.to_owned(),
            anchor,
        });
    }

    /// `span.languageSpecificText` with one child per language (`span.cs`, `span.vb`, …).
    fn language_specific_spans(&self, id: NodeId) -> Inline {
        let (mut cs, mut vb, mut cpp, mut nu) = (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        );
        for child in self.arena.children(id) {
            let value = normalize_ws(&self.arena.subtree_text(*child, true));
            if value.is_empty() {
                continue;
            }
            let classes = self.arena.classes(*child);
            if classes.iter().any(|c| c == "cs") {
                cs = value;
            } else if classes.iter().any(|c| c == "vb") {
                vb = value;
            } else if classes.iter().any(|c| c == "cpp") {
                cpp = value;
            } else if classes.iter().any(|c| c == "nu") {
                nu = value;
            }
        }
        if cs.is_empty() && vb.is_empty() && cpp.is_empty() && nu.is_empty() {
            nu = normalize_ws(&self.arena.subtree_text(id, true));
        }
        Inline::LanguageSpecific { cs, vb, cpp, nu }
    }

    /// Images that exist only to draw the expand/collapse or copy controls.
    fn is_decorative_image(&self, id: NodeId) -> bool {
        if self.arena.has_class(id, "toggle") {
            return true;
        }
        if self.arena.attr_lc(id, "name").as_deref() == Some("toggleswitch") {
            return true;
        }
        if let Some(style) = self.arena.attr_lc(id, "style") {
            let compact: String = style.chars().filter(|c| !c.is_whitespace()).collect();
            if compact.contains("display:none") {
                return true;
            }
        }
        false
    }

    /// Convert an element as a block, or `None` when it is inline content.
    fn try_block_element(&mut self, id: NodeId, name: &str) -> Option<Vec<Block>> {
        let classes = self.arena.classes(id);
        let has_class = |needle: &str| classes.iter().any(|c| c.contains(needle));

        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name[1..].parse::<u8>().unwrap_or(2);
                Some(vec![self.heading(id, level)])
            }
            "p" => Some(vec![Block::Paragraph {
                inlines: self.inlines(id),
            }]),
            "hr" => Some(vec![Block::Rule]),
            "pre" => Some(vec![self.code_block(id, "pre")]),
            "blockquote" => Some(vec![Block::Quote {
                blocks: self.nested(id),
            }]),
            "ul" | "ol" | "menu" | "dir" => Some(vec![self.list(id, name != "ul" && name != "menu" && name != "dir")]),
            "dl" => Some(vec![self.definition_list(id)]),
            "table" => {
                if self.class_is(id, "titletable") {
                    // Help Viewer 1.x page title: `IScans<span/>SetCustomScan Method`.
                    Some(vec![self.title_table(id)])
                } else {
                    Some(vec![self.table(id)])
                }
            }
            "img" => {
                if self.is_decorative_image(id) {
                    return Some(Vec::new());
                }
                Some(vec![Block::Image {
                    src: self.arena.attr(id, "src").unwrap_or("").to_owned(),
                    alt: self
                        .arena
                        .attr(id, "alt")
                        .map(crate::html::decode_entities)
                        .unwrap_or_default(),
                }])
            }
            "codesnippet" => Some(vec![self.code_block(id, "codesnippet")]),
            "collapsiblearea" => Some(vec![self.collapsible(id, None)]),
            "inheritedmember" => Some(vec![self.collapsible(id, Some("Inherited Members"))]),
            "a" => {
                // Only the two *positional* forms are blocks here: `<a name="x">` /
                // `<a id="x"></a>` markers. A link with text belongs in the running text —
                // returning a paragraph for those used to split sentences in three.
                let anchor = self
                    .arena
                    .attr(id, "name")
                    .or_else(|| self.arena.id_attr(id))
                    .map(|s| s.to_owned())
                    .filter(|s| !s.is_empty());
                let text = normalize_ws(&self.arena.subtree_text(id, true));
                match anchor {
                    Some(anchor) => {
                        self.note_anchor(&anchor);
                        if text.trim().is_empty() {
                            Some(vec![Block::Anchor(anchor)])
                        } else {
                            None
                        }
                    }
                    None if text.trim().is_empty() => Some(Vec::new()),
                    None => None,
                }
            }
            "div" | "center" | "section" | "article" | "aside" | "main" | "figure"
            | "figcaption" | "address" | "fieldset" | "details" | "span" => {
                // Help Viewer 1.x code snippet with language tabs. The tabs and the Copy
                // toolbar must not be printed as text; the code divs become one group.
                if name == "div" && self.class_is(id, "codesnippetcontainer") {
                    return Some(self.shfb_snippet_container(id));
                }
                // Sandcastle 1.x: `div.code` with one `span[codeLanguage]` per language.
                if name == "div" && self.class_is(id, "code") {
                    if let Some(blocks) = self.sandcastle_snippets(id) {
                        return Some(blocks);
                    }
                }
                // Microsoft Help behaviours carried in class/id attributes.
                if has_class("alert") || has_class("note") {
                    return Some(vec![self.note(id)]);
                }
                if has_class("title") && name == "div" {
                    return Some(vec![self.heading(id, 1)]);
                }
                if has_class("inheritedmember") {
                    return Some(vec![self.collapsible(id, Some("Inherited Members"))]);
                }
                if self.arena.attr(id, "expanded").is_some() && self.arena.attr(id, "title").is_some()
                {
                    // A CollapsibleArea written as a plain div.
                    return Some(vec![self.collapsible(id, None)]);
                }
                let group = self
                    .arena
                    .id_attr(id)
                    .or_else(|| self.arena.attr(id, "class"))
                    .map(|s| s.to_ascii_lowercase())
                    .filter(|s| s.contains("snippetgroup"));
                match group {
                    // Inside a snippet group every CodeSnippet knows the group id, which is
                    // how the viewer builds its language tabs.
                    Some(new_group) => {
                        let saved = std::mem::replace(&mut self.snippet_group, Some(new_group));
                        let result = if self.has_block_descendant(id) {
                            Some(self.nested(id))
                        } else {
                            None
                        };
                        self.snippet_group = saved;
                        result
                    }
                    None => {
                        if self.has_block_descendant(id) {
                            Some(self.nested(id))
                        } else {
                            None
                        }
                    }
                }
            }
            _ => {
                if is_inline_name(name) {
                    None
                } else if self.has_block_descendant(id) {
                    Some(self.nested(id))
                } else if self.arena.subtree_text(id, true).trim().is_empty() {
                    Some(Vec::new())
                } else {
                    None
                }
            }
        }
    }

    fn nested(&mut self, id: NodeId) -> Vec<Block> {
        self.nested_except(id, None)
    }

    /// [`Convert::nested`] while skipping one child (used to drop a note's label node).
    fn nested_except(&mut self, id: NodeId, skip: Option<NodeId>) -> Vec<Block> {
        let previous = std::mem::replace(&mut self.skip_child, skip);
        self.depth += 1;
        let blocks = self.blocks(id);
        self.depth -= 1;
        self.skip_child = previous;
        blocks
    }

    fn heading(&mut self, id: NodeId, level: u8) -> Block {
        let inlines = self.inlines(id);
        let mut text = normalize_ws(&inline_text(&inlines));
        if text.is_empty() {
            text = normalize_ws(&self.arena.subtree_text(id, true));
        }
        let anchor = self
            .arena
            .id_attr(id)
            .map(|s| s.to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slugify(&text));
        if !text.is_empty() {
            self.note_anchor(&anchor);
            self.outline.push(OutlineEntry {
                level,
                text: text.clone(),
                anchor: anchor.clone(),
            });
            if self.title.is_none() {
                self.title = Some(text.clone());
            }
        }
        let anchor = if text.is_empty() { None } else { Some(anchor) };
        Block::Heading {
            level,
            inlines,
            anchor,
        }
    }

    fn note_anchor(&mut self, anchor: &str) {
        if !self.anchors.iter().any(|a| a == anchor) {
            self.anchors.push(anchor.to_owned());
        }
    }

    fn list(&mut self, id: NodeId, ordered: bool) -> Block {
        let mut items: Vec<Vec<Block>> = Vec::new();
        for child in self.arena.children(id).to_vec() {
            match self.arena.name(child) {
                Some("li") => {
                    let blocks = self.nested(child);
                    items.push(blocks);
                }
                Some("ul") | Some("ol") | Some("menu") | Some("dir") => {
                    // Malformed: a nested list as a direct child.
                    items.push(vec![self.list(child, false)]);
                }
                _ => {
                    let text = self.arena.subtree_text(child, true);
                    if !text.trim().is_empty() {
                        items.push(vec![Block::Paragraph {
                            inlines: vec![Inline::Text(normalize_ws(&text))],
                        }]);
                    }
                }
            }
        }
        if items.is_empty() {
            let text = normalize_ws(&self.arena.subtree_text(id, true));
            if !text.is_empty() {
                items.push(vec![Block::Paragraph {
                    inlines: vec![Inline::Text(text)],
                }]);
            }
        }
        Block::List { ordered, items }
    }

    fn definition_list(&mut self, id: NodeId) -> Block {
        let mut items: Vec<Vec<Block>> = Vec::new();
        for child in self.arena.children(id).to_vec() {
            match self.arena.name(child) {
                Some("dt") => {
                    let inlines = self.inlines(child);
                    items.push(vec![Block::Paragraph {
                        inlines: vec![Inline::Bold(inlines)],
                    }]);
                }
                Some("dd") => items.push(self.nested(child)),
                _ => {}
            }
        }
        Block::List {
            ordered: false,
            items,
        }
    }

    /// The Help Viewer 1.x `table.titleTable` page title.
    fn title_table(&mut self, id: NodeId) -> Block {
        let cell = self
            .arena
            .find_first(id, &|a, n| matches!(a.name(n), Some("td") | Some("th")));
        match cell {
            Some(cell) => self.heading(cell, 1),
            None => self.heading(id, 1),
        }
    }

    fn table(&mut self, id: NodeId) -> Block {
        let caption = self
            .arena
            .find_first(id, &|a, n| a.name(n) == Some("caption"))
            .map(|n| normalize_ws(&self.arena.subtree_text(n, true)))
            .filter(|s| !s.is_empty());

        let mut rows: Vec<Row> = Vec::new();
        self.collect_rows(id, &mut rows, 0);

        let mut header_rows = 0usize;
        for row in &rows {
            if !row.cells.is_empty() && row.cells.iter().all(|c| c.is_header) {
                header_rows += 1;
            } else {
                break;
            }
        }
        let table = Table {
            rows,
            header_rows,
            caption,
        };
        let empty = table.rows.is_empty();
        if empty {
            // A "table" used purely for layout still has text worth showing.
            return Block::Paragraph {
                inlines: vec![Inline::Text(normalize_ws(&self.arena.subtree_text(id, true)))],
            };
        }
        Block::Table(table)
    }

    fn collect_rows(&mut self, id: NodeId, rows: &mut Vec<Row>, _depth: usize) {
        for child in self.arena.children(id).to_vec() {
            match self.arena.name(child) {
                Some("tr") => {
                    let mut row = Row::default();
                    for cell in self.arena.children(child).to_vec() {
                        let name = match self.arena.name(cell) {
                            Some(n) => n.to_owned(),
                            None => continue,
                        };
                        if name != "td" && name != "th" {
                            continue;
                        }
                        let blocks = self.nested(cell);
                        let colspan = self
                            .arena
                            .attr(cell, "colspan")
                            .and_then(|v| v.trim().parse::<u32>().ok())
                            .filter(|v| *v > 0)
                            .unwrap_or(1);
                        let rowspan = self
                            .arena
                            .attr(cell, "rowspan")
                            .and_then(|v| v.trim().parse::<u32>().ok())
                            .filter(|v| *v > 0)
                            .unwrap_or(1);
                        row.cells.push(Cell {
                            blocks,
                            colspan,
                            rowspan,
                            align: self.arena.attr(cell, "align").map(|s| s.to_owned()),
                            is_header: name == "th",
                        });
                    }
                    if !row.cells.is_empty() {
                        row.is_header = row.cells.iter().all(|c| c.is_header);
                        rows.push(row);
                    }
                }
                Some("thead") | Some("tbody") | Some("tfoot") => {
                    self.collect_rows(child, rows, _depth + 1)
                }
                Some("caption") | None | Some(_) => {}
            }
        }
    }

    fn note(&mut self, id: NodeId) -> Block {
        // Label: `<span class="label">Note</span>` or the header cell of the inner table.
        let label_node = self.arena.find_first(id, &|a, n| {
            (a.name(n) == Some("span") && a.has_class(n, "label"))
                || (a.name(n) == Some("th") && !a.subtree_text(n, true).trim().is_empty())
        });
        let label = label_node
            .map(|n| normalize_ws(&self.arena.subtree_text(n, true)))
            .filter(|s| !s.is_empty());

        // Content: the body cell of a callout table, else the element itself. The node that
        // supplied the label is skipped, so `Note` is not printed twice.
        let body_cell = self.arena.find_first(id, &|a, n| {
            a.name(n) == Some("td") && !a.subtree_text(n, true).trim().is_empty()
        });
        let (content_node, skip) = match body_cell {
            Some(cell) if self.arena.name(id) != Some("td") => (cell, None),
            _ => (id, label_node),
        };
        let blocks = self.nested_except(content_node, skip);
        Block::Note { label, blocks }
    }

    fn collapsible(&mut self, id: NodeId, default_title: Option<&str>) -> Block {
        let title = self
            .arena
            .attr(id, "title")
            .map(normalize_ws)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.arena
                    .find_first(id, &|a, n| {
                        matches!(
                            a.name(n),
                            Some("summary") | Some("h1") | Some("h2") | Some("h3") | Some("h4")
                        )
                    })
                    .map(|n| normalize_ws(&self.arena.subtree_text(n, true)))
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| default_title.map(str::to_owned))
            .unwrap_or_else(|| "Details".to_owned());
        let expanded = match self.arena.attr_lc(id, "expanded") {
            Some(v) => !matches!(v.as_str(), "0" | "false" | "no" | "off"),
            None => true,
        };
        let blocks = self.nested(id);
        Block::Collapsible {
            title,
            expanded,
            blocks,
        }
    }

    /// `div.codeSnippetContainer`: language tabs, a Copy toolbar and one code div per
    /// language. The tabs/toolbar are chrome; the code divs become a single snippet group.
    fn shfb_snippet_container(&mut self, id: NodeId) -> Vec<Block> {
        let mut tab_nodes = Vec::new();
        collect_by_class(self.arena, id, "codesnippetcontainertab", &mut tab_nodes);
        let mut code_nodes = Vec::new();
        collect_by_class(self.arena, id, "codesnippetcontainercode", &mut code_nodes);

        // (position, language code, tab label)
        let mut tabs: Vec<(usize, String, String)> = Vec::new();
        for (fallback, tab) in tab_nodes.iter().copied().enumerate() {
            let label = normalize_ws(&self.arena.subtree_text(tab, true));
            let onclick = self
                .arena
                .find_first(tab, &|a, n| a.attr(n, "onclick").is_some())
                .and_then(|n| self.arena.attr(n, "onclick"))
                .unwrap_or("");
            let args = call_args(onclick, "ChangeTab");
            let position = args
                .as_ref()
                .and_then(|args| args.get(2))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(fallback + 1);
            let language = args
                .as_ref()
                .and_then(|args| args.get(1))
                .cloned()
                .unwrap_or_default();
            tabs.push((position, language, label));
        }

        if code_nodes.is_empty() {
            // Not the tabbed shape after all: keep the snippet text if there is any.
            if let Some(pre) = self
                .arena
                .find_first(id, &|a, n| a.name(n) == Some("pre"))
            {
                return vec![self.code_block(pre, "pre")];
            }
            return Vec::new();
        }

        let group = code_nodes.iter().find_map(|node| {
            self.arena
                .id_attr(*node)
                .and_then(|id| id.split_once("_code_"))
                .map(|(prefix, _)| prefix.to_owned())
        });

        let mut blocks = Vec::with_capacity(code_nodes.len());
        for (position, node) in code_nodes.iter().copied().enumerate() {
            let pre = self
                .arena
                .find_first(node, &|a, n| a.name(n) == Some("pre"))
                .unwrap_or(node);
            let mut block = self.code_block(pre, "pre");
            if let Block::Code(code) = &mut block {
                let tab = tabs
                    .iter()
                    .find(|(tab_position, ..)| *tab_position == position + 1)
                    .or_else(|| tabs.get(position));
                if let Some((_, language, label)) = tab {
                    code.language = snippet_language(language);
                    if !label.is_empty() {
                        code.display_language = Some(label.clone());
                    }
                }
                code.group = group.clone();
                code.copyable = true;
            }
            blocks.push(block);
        }
        blocks
    }

    /// Sandcastle 1.x snippets: `div.code` → `span[codeLanguage]` → `table` (the header
    /// cell is the tab label) → `pre`. All the spans become one snippet group.
    fn sandcastle_snippets(&mut self, id: NodeId) -> Option<Vec<Block>> {
        let mut spans = Vec::new();
        collect_with_attr(self.arena, id, "codelanguage", &mut spans);
        if spans.is_empty() {
            return None;
        }
        let group = Some(match self.arena.id_attr(id) {
            Some(id) => format!("code:{id}"),
            None => format!("code:node{id}"),
        });
        let mut blocks = Vec::with_capacity(spans.len());
        for span in spans {
            let language = self
                .arena
                .attr(span, "codelanguage")
                .unwrap_or("")
                .to_owned();
            let pre = self
                .arena
                .find_first(span, &|a, n| a.name(n) == Some("pre"))
                .unwrap_or(span);
            let label = self
                .arena
                .find_first(span, &|a, n| a.name(n) == Some("th"))
                .map(|n| normalize_ws(&self.arena.subtree_text(n, true)))
                .filter(|s| !s.is_empty());
            let mut block = self.code_block(pre, "pre");
            if let Block::Code(code) = &mut block {
                code.language = snippet_language(&language);
                code.display_language = label.or_else(|| snippet_language(&language));
                code.group = group.clone();
                code.copyable = true;
            }
            blocks.push(block);
        }
        Some(blocks)
    }

    fn code_block(&mut self, id: NodeId, kind: &str) -> Block {
        let language = self
            .arena
            .attr(id, "language")
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
            .or_else(|| self.class_language(id));
        let display_language = self
            .arena
            .attr(id, "displaylanguage")
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
            .or_else(|| language.clone());
        let copyable = match self.arena.attr_lc(id, "enablecopycode") {
            Some(v) => !matches!(v.as_str(), "0" | "false" | "no" | "off"),
            None => true,
        };
        let raw = self.arena.subtree_text(id, false);
        let raw = raw
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\u{A0}', " ")
            // `ContainsMarkup="true"` snippets arrive with their own markup encoded; the
            // lexer already decoded one level, which is exactly what we want to display.
            .replace("<!---->", "");
        let trimmed = raw.trim_matches('\n');
        let mut lines: Vec<String> = trimmed
            .split('\n')
            .map(|l| l.trim_end().to_owned())
            .collect();
        while lines.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
            lines.remove(0);
        }
        while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            lines.pop();
        }
        let _ = kind;
        Block::Code(CodeBlock {
            language,
            display_language,
            group: self.snippet_group.clone(),
            lines,
            copyable,
        })
    }

    fn class_language(&self, id: NodeId) -> Option<String> {
        for class in self.arena.classes(id) {
            for prefix in ["lang-", "language-"] {
                if let Some(rest) = class.strip_prefix(prefix) {
                    if !rest.is_empty() {
                        return Some(rest.to_owned());
                    }
                }
            }
        }
        // `<pre><code class="lang-x">`
        let code_child = self
            .arena
            .find_first(id, &|a, n| a.name(n) == Some("code"));
        code_child.and_then(|c| {
            self.arena.attr(c, "class").and_then(|class| {
                class
                    .split_whitespace()
                    .find_map(|c| {
                        c.strip_prefix("lang-")
                            .or_else(|| c.strip_prefix("language-"))
                            .map(|s| s.to_ascii_lowercase())
                    })
            })
        })
    }

    fn inlines(&mut self, node: NodeId) -> Vec<Inline> {
        let mut out = Vec::new();
        for child in self.arena.children(node).to_vec() {
            if self.arena.is_text(child) {
                // Code/pre keep raw text; here we collapse whitespace (keeping the edges so
                // inline runs concatenate correctly).
                let text = collapse_ws(self.arena.text(child).unwrap_or(""));
                if !text.is_empty() {
                    push_text(&mut out, text);
                }
                continue;
            }
            let name = match self.arena.name(child) {
                Some(n) => n.to_owned(),
                None => continue,
            };
            if is_skipped(&name) {
                continue;
            }
            if let Some(placeholder) = self.placeholder(child) {
                out.push(placeholder);
                continue;
            }
            match name.as_str() {
                "br" => out.push(Inline::LineBreak),
                "img" => {
                    if self.is_decorative_image(child) {
                        continue;
                    }
                    out.push(Inline::Image {
                        src: self.arena.attr(child, "src").unwrap_or("").to_owned(),
                        alt: self
                            .arena
                            .attr(child, "alt")
                            .map(crate::html::decode_entities)
                            .unwrap_or_default(),
                    });
                }
                "b" | "strong" => out.push(Inline::Bold(self.inlines(child))),
                "i" | "em" | "cite" | "var" | "dfn" => out.push(Inline::Italic(self.inlines(child))),
                "u" | "ins" => out.push(Inline::Underline(self.inlines(child))),
                "s" | "strike" | "del" => out.push(Inline::Strike(self.inlines(child))),
                "sub" => out.push(Inline::Sub(self.inlines(child))),
                "sup" => out.push(Inline::Sup(self.inlines(child))),
                "code" | "tt" | "kbd" | "samp" | "text" => {
                    // Keep the edge spaces: `<code>title:</code>, <code>kw:</code>` must not
                    // glue into `title:,kw:`.
                    let text = collapse_ws(&self.arena.subtree_text(child, false));
                    out.push(Inline::Code(text));
                }
                "a" => {
                    let target = self
                        .arena
                        .attr(child, "href")
                        .map(|s| s.to_owned())
                        .unwrap_or_default();
                    if target.is_empty() {
                        if let Some(anchor) = self
                            .arena
                            .attr(child, "name")
                            .or_else(|| self.arena.id_attr(child))
                        {
                            self.note_anchor(anchor);
                        }
                    }
                    let inlines = self.inlines(child);
                    if inlines.is_empty() {
                        // `<a name="x"></a>` marker inside text.
                        continue;
                    }
                    out.push(Inline::Link { target, inlines });
                }
                "span" if self.arena.has_class(child, "languagespecifictext") => {
                    out.push(self.language_specific_spans(child));
                }
                "span" | "font" | "small" | "big" | "label" | "abbr" | "acronym" | "q" | "mark"
                | "time" | "nobr" | "wbr" => {
                    let inlines = self.inlines(child);
                    if self.arena.has_class(child, "label") {
                        out.push(Inline::Label(inlines));
                    } else {
                        out.extend(inlines);
                    }
                }
                "languagespecifictext" | "languageelement" => {
                    out.push(self.language_specific(child));
                }
                "codesnippet" | "pre" => {
                    let code = self.code_block(child, "codesnippet");
                    if let Block::Code(code) = code {
                        out.push(Inline::Code(code.lines.join("\n")));
                    }
                }
                "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "table"
                | "blockquote" | "li" | "td" | "th" | "tr" | "dl" | "dt" | "dd" => {
                    // Block content inside an inline context: keep the words, separated.
                    let inlines = self.inlines(child);
                    if !inlines.is_empty() {
                        out.push(Inline::LineBreak);
                        out.extend(inlines);
                    }
                }
                _ => {
                    let inlines = self.inlines(child);
                    out.extend(inlines);
                }
            }
        }
        trim_inlines(out)
    }

    fn language_specific(&self, id: NodeId) -> Inline {
        let get = |name: &str| {
            self.arena
                .attr(id, name)
                .map(normalize_ws)
                .unwrap_or_default()
        };
        let cs = get("devlangcs");
        let vb = get("devlangvb");
        let cpp = get("devlangcpp");
        let mut nu = get("devlangnu");
        if nu.is_empty() {
            // Any other `devLang*` attribute is a language name (F#, Fortran, …).
            if let Some((_, value)) = self
                .arena
                .attrs(id)
                .iter()
                .find(|(k, _)| k.len() > 7 && k.to_ascii_lowercase().starts_with("devlang"))
            {
                nu = normalize_ws(value);
            }
        }
        if nu.is_empty() && cs.is_empty() && vb.is_empty() && cpp.is_empty() {
            nu = normalize_ws(&self.arena.subtree_text(id, true));
        }
        Inline::LanguageSpecific { cs, vb, cpp, nu }
    }

    fn has_block_descendant(&self, id: NodeId) -> bool {
        self.arena.find_first(id, &|a, n| {
            matches!(
                a.name(n),
                Some("p")
                    | Some("div")
                    | Some("ul")
                    | Some("ol")
                    | Some("dl")
                    | Some("table")
                    | Some("pre")
                    | Some("blockquote")
                    | Some("h1")
                    | Some("h2")
                    | Some("h3")
                    | Some("h4")
                    | Some("h5")
                    | Some("h6")
                    | Some("codesnippet")
                    | Some("collapsiblearea")
                    | Some("hr")
                    | Some("li")
            )
        })
        .is_some()
    }
}

/// Collect descendants carrying exactly `class` (case-insensitive token match).
fn collect_by_class(arena: &Arena, node: NodeId, class: &str, out: &mut Vec<NodeId>) {
    for child in arena.children(node) {
        if arena.is_text(*child) {
            continue;
        }
        if arena.classes(*child).iter().any(|c| c == class) {
            out.push(*child);
        }
        collect_by_class(arena, *child, class, out);
    }
}

/// Collect descendants carrying `attr` (stopping at the first match on a branch).
fn collect_with_attr(arena: &Arena, node: NodeId, attr: &str, out: &mut Vec<NodeId>) {
    for child in arena.children(node) {
        if arena.is_text(*child) {
            continue;
        }
        if arena.attr(*child, attr).is_some() {
            out.push(*child);
            continue;
        }
        collect_with_attr(arena, *child, attr, out);
    }
}

/// Arguments of `name('a', 'b', …)` inside an inline event handler.
fn call_args(script: &str, name: &str) -> Option<Vec<String>> {
    let start = script.find(name)? + name.len();
    let rest = script.get(start..)?.trim_start();
    let rest = rest.strip_prefix('(')?;
    let end = rest.find(')')?;
    Some(
        rest[..end]
            .split(',')
            .map(|arg| arg.trim().trim_matches(['\'', '"']).trim().to_owned())
            .collect(),
    )
}

/// Machine language name for a Help Viewer tab code (`cs`, `vb`, `cpp`, `fs`, …).
fn snippet_language(code: &str) -> Option<String> {
    let code = code.trim();
    if code.is_empty() {
        return None;
    }
    Some(match code.to_ascii_lowercase().as_str() {
        "cs" | "csharp" => "CSharp".to_owned(),
        "vb" | "visualbasic" | "visualbasicdeclaration" => "VisualBasic".to_owned(),
        "cpp" | "managedcplusplus" | "visualcplusplus" => "CPlusPlus".to_owned(),
        "fs" | "fsharp" => "FSharp".to_owned(),
        _ => code.to_owned(),
    })
}

fn flush(out: &mut Vec<Block>, pending: &mut Vec<Inline>) {
    if pending.is_empty() {
        return;
    }
    let inlines = trim_inlines(std::mem::take(pending));
    if !inlines.is_empty() {
        out.push(Block::Paragraph { inlines });
    }
}

/// Merge adjacent text runs and drop empty containers left by the conversion.
fn trim_inlines(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        let inline = match inline {
            Inline::Bold(v) => Inline::Bold(trim_inlines(v)),
            Inline::Italic(v) => Inline::Italic(trim_inlines(v)),
            Inline::Underline(v) => Inline::Underline(trim_inlines(v)),
            Inline::Strike(v) => Inline::Strike(trim_inlines(v)),
            Inline::Sub(v) => Inline::Sub(trim_inlines(v)),
            Inline::Sup(v) => Inline::Sup(trim_inlines(v)),
            Inline::Label(v) => Inline::Label(trim_inlines(v)),
            Inline::Link { target, inlines } => Inline::Link {
                target,
                inlines: trim_inlines(inlines),
            },
            other => other,
        };
        match &inline {
            Inline::Text(t) if t.is_empty() => continue,
            Inline::Bold(v)
            | Inline::Italic(v)
            | Inline::Underline(v)
            | Inline::Strike(v)
            | Inline::Sub(v)
            | Inline::Sup(v)
            | Inline::Label(v)
                if v.is_empty() =>
            {
                continue
            }
            Inline::Link { inlines, .. } if inlines.is_empty() => continue,
            _ => {}
        }
        // Merge adjacent plain text.
        if let (Some(Inline::Text(prev)), Inline::Text(next)) = (out.last_mut(), &inline) {
            prev.push_str(next);
            continue;
        }
        out.push(inline);
    }
    // Trim leading/trailing spaces of the whole run.
    if let Some(Inline::Text(first)) = out.first_mut() {
        let trimmed = first.trim_start().to_owned();
        *first = trimmed;
    }
    if let Some(Inline::Text(last)) = out.last_mut() {
        let trimmed = last.trim_end().to_owned();
        *last = trimmed;
    }
    out.retain(|inline| !matches!(inline, Inline::Text(t) if t.is_empty()));
    out
}

fn push_text(out: &mut Vec<Inline>, text: String) {
    match out.last_mut() {
        // Concatenate verbatim: the text node already carries its own separators.
        Some(Inline::Text(prev)) => prev.push_str(&text),
        _ => out.push(Inline::Text(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_runs_keep_their_separating_spaces_at_block_level() {
        // Regression: block-level text used to be trimmed, glueing `Contents— the…` and
        // `fromMicrosoft.Help.TocParenttags` together in the reader.
        let doc = parse_document(
            "<ul><li><b>Contents</b> — the table of contents, built from <code>Microsoft.Help.TocParent</code> tags.</li></ul>",
        );
        let text = blocks_text(&doc.blocks);
        assert!(text.contains("Contents — the table of contents"), "{text}");
        assert!(text.contains("from Microsoft.Help.TocParent tags"), "{text}");
        assert!(!text.contains("Contents—"));
        assert!(!text.contains("fromMicrosoft"));
    }

    #[test]
    fn block_level_br_breaks_the_line() {
        // `iapi.mshc` writes the signature separator as a block-level `<br>`:
        // `<b>Type:</b> <a>ICustomScan</a><br/>object containing …`.
        let doc = parse_document(
            r#"<div class="summary"><b>Type:</b> <a href="x.htm">ICustomScan</a><br/>object containing the scan definition.</div>"#,
        );
        let paragraph = match &doc.blocks[0] {
            Block::Paragraph { inlines } => inlines,
            other => panic!("expected a paragraph, got {other:?}"),
        };
        assert!(
            paragraph.iter().any(|inline| matches!(inline, Inline::LineBreak)),
            "{paragraph:?}"
        );
        let text = blocks_text(&doc.blocks);
        assert!(
            text.contains("ICustomScan\nobject containing"),
            "the break must survive as a newline: {text:?}"
        );
        assert!(!text.contains("ICustomScanobject"), "{text:?}");
    }

    #[test]
    fn paragraphs_still_trim_their_edges() {
        let doc = parse_document("<p>\n   Hello <b>there</b>.\n</p>");
        assert_eq!(blocks_text(&doc.blocks).trim(), "Hello there.");
    }

    #[test]
    fn basic_structure() {
        let doc = parse_document(
            r#"<html><body>
            <div class="title">Lucid Overview</div>
            <p>Hello <b>world</b>, see <a href="other.htm">other</a>.</p>
            <h2>Section</h2>
            <ul><li>one</li><li>two<ul><li>nested</li></ul></li></ul>
            </body></html>"#,
        );
        assert_eq!(doc.title.as_deref(), Some("Lucid Overview"));
        assert!(matches!(doc.blocks[0], Block::Heading { level: 1, .. }));
        assert_eq!(doc.outline.len(), 2);
        assert_eq!(doc.outline[1].text, "Section");
        assert_eq!(doc.outline[1].anchor, "section");
        let list = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::List { items, ordered } => Some((items, ordered)),
                _ => None,
            })
            .expect("list");
        assert_eq!(list.0.len(), 2);
        assert!(!list.1);
        assert!(doc.text.contains("Hello world, see other."));
    }

    #[test]
    fn code_snippet_and_language_specific() {
        let doc = parse_document(
            r#"<div id="snippetGroup">
                 <CodeSnippet EnableCopyCode="true" Language="CSharp" DisplayLanguage="C#" >
private void F() { }
</CodeSnippet>
                 <CodeSnippet Language="VisualBasic" DisplayLanguage="Visual Basic">Sub F()
End Sub</CodeSnippet>
               </div>
               <p>Language = <LanguageSpecificText devLangcs="CS" devLangvb="VB" devLangcpp="C++" devLangnu="F#" /></p>"#,
        );
        let codes: Vec<&CodeBlock> = doc
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Code(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(codes.len(), 2);
        assert_eq!(codes[0].language.as_deref(), Some("CSharp"));
        assert_eq!(codes[0].display_language.as_deref(), Some("C#"));
        assert!(codes[0].group.is_some(), "snippet group must be propagated");
        assert_eq!(codes[0].lines[0], "private void F() { }");
        assert!(codes[1].lines.contains(&"End Sub".to_owned()));
        assert!(doc.code_text.contains("private void F()"));
        // LanguageSpecific resolves to the C# form by default.
        assert!(doc.text.contains("Language = CS"));
    }

    #[test]
    fn collapsible_area_and_note() {
        let doc = parse_document(
            r#"<div class="alert"><table><tr><th><strong>Note </strong></th></tr>
                 <tr><td><p>Mind the gap.</p></td></tr></table></div>
               <CollapsibleArea Expanded="1" Title="More details"><p>hidden text</p></CollapsibleArea>"#,
        );
        match &doc.blocks[0] {
            Block::Note { label, blocks } => {
                assert_eq!(label.as_deref(), Some("Note"));
                assert!(blocks_text(blocks).contains("Mind the gap."));
            }
            other => panic!("expected note, got {other:?}"),
        }
        match &doc.blocks[1] {
            Block::Collapsible {
                title,
                expanded,
                blocks,
            } => {
                assert_eq!(title, "More details");
                assert!(expanded);
                assert!(blocks_text(blocks).contains("hidden text"));
            }
            other => panic!("expected collapsible, got {other:?}"),
        }
    }

    #[test]
    fn table_with_header_row() {
        let doc = parse_document(
            r#"<table><caption>Options</caption>
                 <tr><th>Flag</th><th>Meaning</th></tr>
                 <tr><td><code>--force</code></td><td>Overwrite</td></tr>
               </table>"#,
        );
        match &doc.blocks[0] {
            Block::Table(t) => {
                assert_eq!(t.rows.len(), 2);
                assert_eq!(t.header_rows, 1);
                assert_eq!(t.caption.as_deref(), Some("Options"));
                assert!(t.rows[0].is_header);
                assert!(blocks_text(&t.rows[1].cells[0].blocks).contains("--force"));
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn chrome_and_scripts_are_dropped() {
        let doc = parse_document(
            r#"<html><head><title>T</title><script>var x = 1;</script></head>
               <body><div id="header"><span id="runningHeaderText">Product Help</span></div>
               <p>Real content</p>
               <div id="footer">copyright</div></body></html>"#,
        );
        assert!(!doc.text.contains("Product Help"));
        assert!(!doc.text.contains("copyright"));
        assert!(!doc.text.contains("var x"));
        assert!(doc.text.contains("Real content"));
    }

    #[test]
    fn a_link_with_text_stays_inside_its_sentence() {
        // Regression: the `<a>` used to become its own paragraph, splitting
        // "…highlighting. See" / "the details below" / "." into three lines.
        let doc = parse_document(
            "<ul><li><b>Search</b> — ranked full-text search with highlighting.\n      See <a href=\"#details\">the details below</a>.</li></ul>",
        );
        let item = match &doc.blocks[0] {
            Block::List { items, .. } => items[0].clone(),
            other => panic!("expected a list, got {other:?}"),
        };
        assert_eq!(item.len(), 1, "one paragraph, not three: {item:?}");
        let text = blocks_text(&item);
        assert!(text.contains("See the details below."), "{text}");
    }

    #[test]
    fn note_labels_are_not_repeated_in_the_body() {
        // Inline callout (Help Viewer writes `<span class="label">Note</span>` in the body).
        let doc = parse_document(
            "<div class=\"alert\"><span class=\"label\">Note</span> Mind the gap.</div>",
        );
        match &doc.blocks[0] {
            Block::Note { label, blocks } => {
                assert_eq!(label.as_deref(), Some("Note"));
                let text = blocks_text(blocks);
                assert_eq!(text.trim(), "Mind the gap.");
            }
            other => panic!("expected a note, got {other:?}"),
        }
        // Table callout: the header cell carries the label, the body cell the text.
        let doc = parse_document(
            "<div class=\"alert\"><table><tr><th><strong>Tip </strong></th></tr>\
             <tr><td><p>Use Ctrl+K.</p></td></tr></table></div>",
        );
        match &doc.blocks[0] {
            Block::Note { label, blocks } => {
                assert_eq!(label.as_deref(), Some("Tip"));
                let text = blocks_text(blocks);
                assert_eq!(text.trim(), "Use Ctrl+K.");
                assert!(!text.contains("Tip"));
            }
            other => panic!("expected a note, got {other:?}"),
        }
    }

    #[test]
    fn anchors_and_outline_ids() {
        let doc = parse_document(
            r#"<h2 id="custom-id">Custom</h2><a name="marker"></a><p>text</p>"#,
        );
        assert_eq!(doc.outline[0].anchor, "custom-id");
        assert!(doc.anchors.contains(&"marker".to_owned()));
        assert!(matches!(doc.blocks[1], Block::Anchor(ref a) if a == "marker"));
    }

    #[test]
    fn windows_entities_and_nbsp_in_code() {
        let doc = parse_document("<p>caf&#233; &amp; t&#233;</p><pre>a\u{A0}b</pre>");
        assert!(doc.text.contains("café & té"));
        assert_eq!(doc.code_text, "a b");
    }

    #[test]
    fn pathological_nesting_does_not_overflow() {
        let mut html = String::new();
        for _ in 0..3000 {
            html.push_str("<div>");
        }
        html.push_str("deep");
        let doc = parse_document(&html);
        assert!(doc.text.contains("deep"));
    }

    #[test]
    fn shfb_title_table_becomes_a_heading_with_its_separator() {
        // Copied from Thermo Fisher's `iapi.mshc` (`IScans.SetCustomScan Method`).
        let doc = parse_document(
            r#"<div class="pageHeader" id="PageHeader">Thermo Fisher Instrument API Documentation</div>
               <div class="pageBody"><div class="topicContent">
               <table class="titleTable"><tr><td class="titleColumn">IScans<span id="LSTF80A9584_0">
               </span><script type="text/javascript">AddLanguageSpecificTextSet("LSTF80A9584_0?cpp=::|nu=.");</script>SetCustomScan Method </td></tr></table>
               <p>Body text.</p>
               </div></div>
               <div id="pageFooter" class="pageFooter">Send comments about this topic</div>"#,
        );
        assert_eq!(doc.title.as_deref(), Some("IScans.SetCustomScan Method"));
        assert!(
            matches!(doc.blocks[0], Block::Heading { level: 1, .. }),
            "expected the title table to become an H1: {:?}",
            doc.blocks
        );
        assert!(doc.text.contains("IScans.SetCustomScan Method"), "{}", doc.text);
        assert!(!doc.text.contains("Thermo Fisher"), "{}", doc.text);
        assert!(!doc.text.contains("Send comments"), "{}", doc.text);
        assert!(doc.text.contains("Body text."));
    }

    #[test]
    fn shfb_snippet_container_becomes_one_group() {
        let doc = parse_document(
            r##"<div class="collapsibleAreaRegion">
                 <span class="collapsibleRegionTitle"><img class="collapseToggle" src="icons/SectionExpanded.png" />Syntax</span>
               </div>
               <div class="collapsibleSection">
               <div class="codeSnippetContainer">
                 <div class="codeSnippetContainerTabs">
                   <div id="ID0EDCA_tab1" class="codeSnippetContainerTab"><a href="#" onclick="javascript:ChangeTab('ID0EDCA','cs','1','4');return false;">C#</a></div>
                   <div id="ID0EDCA_tab2" class="codeSnippetContainerTab"><a href="#" onclick="javascript:ChangeTab('ID0EDCA','vb','2','4');return false;">VB</a></div>
                   <div id="ID0EDCA_tab3" class="codeSnippetContainerTab"><a href="#" onclick="javascript:ChangeTab('ID0EDCA','cpp','3','4');return false;">C++</a></div>
                   <div id="ID0EDCA_tab4" class="codeSnippetContainerTab"><a href="#" onclick="javascript:ChangeTab('ID0EDCA','fs','4','4');return false;">F#</a></div>
                 </div>
                 <div class="codeSnippetContainerCodeContainer">
                   <div class="codeSnippetToolBar"><div class="codeSnippetToolBarText"><a class="copyCodeSnippet" onclick="javascript:CopyToClipboard('ID0EDCA');return false;" title="Copy">Copy</a></div></div>
                   <div id="ID0EDCA_code_Div1" class="codeSnippetContainerCode" style="display: block"><pre xml:space="preserve">event <span class="identifier">EventHandler</span> <span class="identifier">CanAcceptNextCustomScan</span></pre></div>
                   <div id="ID0EDCA_code_Div2" class="codeSnippetContainerCode" style="display: none"><pre xml:space="preserve">Event CanAcceptNextCustomScan As EventHandler</pre></div>
                   <div id="ID0EDCA_code_Div3" class="codeSnippetContainerCode" style="display: none"><pre xml:space="preserve">event EventHandler^ CanAcceptNextCustomScan;</pre></div>
                   <div id="ID0EDCA_code_Div4" class="codeSnippetContainerCode" style="display: none"><pre xml:space="preserve">abstract CanAcceptNextCustomScan : IEvent&lt;EventHandler, EventArgs&gt;</pre></div>
                 </div>
               </div>
               </div>"##,
        );
        let codes: Vec<&CodeBlock> = doc
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Collapsible { blocks, .. } => Some(blocks),
                _ => None,
            })
            .expect("collapsible syntax region")
            .iter()
            .filter_map(|block| match block {
                Block::Code(code) => Some(code),
                _ => None,
            })
            .collect();
        assert_eq!(codes.len(), 4, "one code block per language: {codes:?}");
        assert!(codes
            .iter()
            .all(|code| code.group.as_deref() == Some("ID0EDCA")));
        assert_eq!(
            codes
                .iter()
                .map(|code| code.display_language.as_deref().unwrap_or(""))
                .collect::<Vec<_>>(),
            vec!["C#", "VB", "C++", "F#"]
        );
        assert_eq!(
            codes[0].lines,
            vec!["event EventHandler CanAcceptNextCustomScan"]
        );
        assert!(codes[3].lines[0].contains("IEvent<EventHandler"));
        assert!(codes.iter().all(|code| code.copyable));
        // Tabs and the Copy toolbar are chrome, never body text.
        assert!(!doc.text.contains("Copy"), "{}", doc.text);
        assert!(!doc.text.contains("C#"), "{}", doc.text);
    }

    #[test]
    fn shfb_collapsible_region_pairs_with_its_section() {
        let doc = parse_document(
            r#"<div class="collapsibleAreaRegion" id="syntaxSection">
                 <span class="collapsibleRegionTitle" onclick="SectionExpandCollapse('ID1RB')" tabindex="0">
                   <img id="ID1RBToggle" class="collapseToggle" alt="Section image" src="icons/SectionExpanded.png" />Syntax</span>
               </div>
               <div id="ID1RBSection" class="collapsibleSection"><p>snippet text</p></div>
               <p>after</p>"#,
        );
        match &doc.blocks[0] {
            Block::Collapsible {
                title,
                expanded,
                blocks,
            } => {
                assert_eq!(title, "Syntax");
                assert!(expanded);
                assert!(blocks_text(blocks).contains("snippet text"));
            }
            other => panic!("expected a collapsible region, got {other:?}"),
        }
        assert!(doc.text.contains("after"));
        assert!(!doc.text.contains("Section image"), "{}", doc.text);
        assert!(!doc.text.contains("image not available"), "{}", doc.text);
    }

    #[test]
    fn sandcastle_heading_pairs_with_its_collapseable_section() {
        // `genericparsing.mshc` uses `<h1 class="heading">` + `div[name=collapseableSection]`.
        let doc = parse_document(
            r#"<h1 class="heading"><span onclick="ExpandCollapse(propertyTableToggle)">
                 <img id="propertyTableToggle" class="toggle" name="toggleSwitch" src="../icons/collapse_all.gif" />Properties</span></h1>
               <div id="propertyTableSection" class="section" name="collapseableSection">
                 <table><tr><th>Name</th></tr><tr><td>ColumnCount</td></tr></table>
               </div>
               <p>tail</p>"#,
        );
        match &doc.blocks[0] {
            Block::Collapsible { title, blocks, .. } => {
                assert_eq!(title, "Properties");
                assert!(blocks_text(blocks).contains("ColumnCount"));
            }
            other => panic!("expected a collapsible section, got {other:?}"),
        }
        assert!(doc.text.contains("tail"));
    }

    #[test]
    fn sandcastle_code_spans_become_one_group() {
        // `genericparsing.mshc`/`revit.mshc` syntax blocks: `div.code` + `span[codeLanguage]`.
        let doc = parse_document(
            r#"<div id="syntaxCodeBlocks" class="code">
                 <span codeLanguage="CSharp"><table><tr><th>C#</th></tr><tr><td><pre>public class Parser { }</pre></td></tr></table></span>
                 <span codeLanguage="VisualBasicDeclaration"><table><tr><th>Visual Basic (Declaration)</th></tr><tr><td><pre>Public Class Parser
End Class</pre></td></tr></table></span>
                 <span codeLanguage="ManagedCPlusPlus"><table><tr><th>Visual C++</th></tr><tr><td><pre>public ref class Parser {};</pre></td></tr></table></span>
               </div>"#,
        );
        let codes: Vec<&CodeBlock> = doc
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Code(code) => Some(code),
                _ => None,
            })
            .collect();
        assert_eq!(codes.len(), 3, "{codes:?}");
        assert!(codes
            .iter()
            .all(|code| code.group.as_deref() == Some("code:syntaxCodeBlocks")));
        assert_eq!(codes[0].language.as_deref(), Some("CSharp"));
        assert_eq!(codes[1].language.as_deref(), Some("VisualBasic"));
        assert_eq!(codes[0].display_language.as_deref(), Some("C#"));
        assert_eq!(
            codes[1].display_language.as_deref(),
            Some("Visual Basic (Declaration)")
        );
        assert_eq!(codes[2].display_language.as_deref(), Some("Visual C++"));
        assert_eq!(codes[0].lines, vec!["public class Parser { }"]);
        // The header cells are tabs, not body text or tables.
        assert!(!doc.text.contains("Visual C++"), "{}", doc.text);
    }

    #[test]
    fn sandcastle_language_specific_text_prefers_one_language() {
        let doc = parse_document(
            r#"<p>System<span class="languageSpecificText"><span class="cs">.</span><span class="vb">.</span><span class="cpp">::</span><span class="nu">.</span></span>Object</p>"#,
        );
        assert_eq!(doc.text.trim(), "System.Object");
    }

    #[test]
    fn collapsible_sections_join_the_outline() {
        let doc = parse_document(
            r#"<div class="collapsibleAreaRegion"><span class="collapsibleRegionTitle">Syntax</span></div>
               <div class="collapsibleSection"><p>x</p></div>"#,
        );
        assert_eq!(doc.outline.len(), 1, "{:?}", doc.outline);
        assert_eq!(doc.outline[0].text, "Syntax");
        assert_eq!(doc.outline[0].level, 2);
        assert_eq!(doc.outline[0].anchor, "syntax");
    }

    #[test]
    fn language_specific_separators_come_from_the_script() {
        let doc = parse_document(
            r#"<p>List<span id="LST1"></span><script>AddLanguageSpecificTextSet("LST1?cs=&lt;|vb=(Of |cpp=&lt;|nu=(");</script>Item<span id="LST2"></span><script>AddLanguageSpecificTextSet("LST2?cs=&gt;|vb=)|cpp=&gt;|nu=)");</script></p>"#,
        );
        assert_eq!(doc.text.trim(), "List<Item>", "{}", doc.text);
        let paragraph = match &doc.blocks[0] {
            Block::Paragraph { inlines } => inlines,
            other => panic!("expected a paragraph, got {other:?}"),
        };
        assert!(
            paragraph
                .iter()
                .any(|inline| matches!(inline, Inline::LanguageSpecific { cs, .. } if cs == "<")),
            "{paragraph:?}"
        );
        // A placeholder without a script entry contributes nothing.
        let doc = parse_document("<p>a<span id=\"NOPE\"></span>b</p>");
        assert_eq!(doc.text.trim(), "ab");
    }

    #[test]
    fn slugify_cases() {
        assert_eq!(slugify("1.0 Introduction!"), "1-0-introduction");
        assert_eq!(slugify("  "), "section");
        assert_eq!(slugify("Über—Alles"), "über-alles");
    }
}
