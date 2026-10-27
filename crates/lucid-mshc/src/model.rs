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
        depth: 0,
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

struct Convert<'a> {
    arena: &'a Arena,
    title: Option<String>,
    anchors: Vec<String>,
    outline: Vec<OutlineEntry>,
    snippet_group: Option<String>,
    depth: usize,
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

fn is_inline_name(name: &str) -> bool {
    matches!(
        name,
        "a" | "b"
            | "strong"
            | "i"
            | "em"
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
        for child in self.arena.children(node).to_vec() {
            if self.arena.is_text(child) {
                let text = normalize_ws(self.arena.text(child).unwrap_or(""));
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
        matches!(
            self.arena.id_attr(id).map(|s| s.to_ascii_lowercase()),
            Some(ref v) if v == "header" || v == "footer"
        )
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
            "table" => Some(vec![self.table(id)]),
            "img" => Some(vec![Block::Image {
                src: self.arena.attr(id, "src").unwrap_or("").to_owned(),
                alt: self
                    .arena
                    .attr(id, "alt")
                    .map(crate::html::decode_entities)
                    .unwrap_or_default(),
            }]),
            "codesnippet" => Some(vec![self.code_block(id, "codesnippet")]),
            "collapsiblearea" => Some(vec![self.collapsible(id, None)]),
            "inheritedmember" => Some(vec![self.collapsible(id, Some("Inherited Members"))]),
            "a" => {
                // Anchor-only `<a name="x">` / `<a id="x"></a>`
                let anchor = self
                    .arena
                    .attr(id, "name")
                    .or_else(|| self.arena.id_attr(id))
                    .map(|s| s.to_owned())
                    .filter(|s| !s.is_empty());
                let inlines = self.inlines(id);
                let text = inline_text(&inlines);
                if let Some(anchor) = anchor {
                    self.note_anchor(&anchor);
                    if text.trim().is_empty() {
                        return Some(vec![Block::Anchor(anchor)]);
                    }
                }
                if text.trim().is_empty() {
                    return Some(Vec::new());
                }
                Some(vec![Block::Paragraph { inlines }])
            }
            "div" | "center" | "section" | "article" | "aside" | "main" | "figure"
            | "figcaption" | "address" | "fieldset" | "details" | "span" => {
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
        self.depth += 1;
        let blocks = self.blocks(id);
        self.depth -= 1;
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
        let label = self
            .arena
            .find_first(id, &|a, n| {
                a.name(n) == Some("span") && a.has_class(n, "label")
            })
            .map(|n| normalize_ws(&self.arena.subtree_text(n, true)))
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.arena
                    .find_first(id, &|a, n| a.name(n) == Some("th"))
                    .map(|n| normalize_ws(&self.arena.subtree_text(n, true)))
                    .filter(|s| !s.is_empty())
            });

        // Content: the body cell of the inner table when it is a callout table.
        let body_cell = self.arena.find_first(id, &|a, n| {
            a.name(n) == Some("td") && !a.subtree_text(n, true).trim().is_empty()
        });
        let blocks = match body_cell {
            Some(cell) if self.arena.name(id) != Some("td") => self.nested(cell),
            _ => self.nested(id),
        };
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
            match name.as_str() {
                "br" => out.push(Inline::LineBreak),
                "img" => out.push(Inline::Image {
                    src: self.arena.attr(child, "src").unwrap_or("").to_owned(),
                    alt: self
                        .arena
                        .attr(child, "alt")
                        .map(crate::html::decode_entities)
                        .unwrap_or_default(),
                }),
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
    fn slugify_cases() {
        assert_eq!(slugify("1.0 Introduction!"), "1-0-introduction");
        assert_eq!(slugify("  "), "section");
        assert_eq!(slugify("Über—Alles"), "über-alles");
    }
}
