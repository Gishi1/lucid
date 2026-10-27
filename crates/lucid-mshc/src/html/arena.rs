//! A permissive mini-DOM built from [`super::lexer`] events.
//!
//! Structure rules implemented (the subset of HTML5 tree construction that matters for
//! help topics): void elements, implicit end tags (`<li>` closing `<li>`, `<p>` closed by
//! block starters, table cell/row handling), unmatched end tags ignored, everything else
//! nesting as written. Nothing here validates — a topic is never rejected because of
//! markup.

use super::lexer::{Event, Lexer};

pub type NodeId = u32;

/// Arena-allocated node.
#[derive(Debug, Clone)]
pub struct ANode {
    pub parent: Option<NodeId>,
    pub kind: AKind,
    pub children: Vec<NodeId>,
}

/// Node payload.
#[derive(Debug, Clone)]
pub enum AKind {
    Element {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Text(String),
}

/// Parsed document as a flat arena; `root` is a synthetic element named `#document`.
#[derive(Debug, Clone)]
pub struct Arena {
    pub nodes: Vec<ANode>,
    pub root: NodeId,
}

/// Elements that never have children / are written as `<br>`.
fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "basefont"
            | "br"
            | "col"
            | "embed"
            | "frame"
            | "hr"
            | "img"
            | "input"
            | "isindex"
            | "keygen"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// Block-level starters that implicitly close an open `<p>`.
fn closes_p(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "center"
            | "collapsiblearea"
            | "codesnippet"
            | "dd"
            | "details"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "hr"
            | "li"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
    )
}

/// Does an open element `open` get implicitly closed when `new` starts?
fn implicitly_closes(open: &str, new: &str) -> bool {
    match open {
        "p" => closes_p(new),
        "li" => new == "li",
        "dt" | "dd" => matches!(new, "dt" | "dd"),
        "td" | "th" => matches!(new, "td" | "th" | "tr" | "tbody" | "thead" | "tfoot"),
        "tr" => matches!(new, "tr" | "tbody" | "thead" | "tfoot"),
        "thead" => matches!(new, "tbody" | "tfoot"),
        "option" => new == "option",
        _ => false,
    }
}

impl Arena {
    /// Parse a document.
    pub fn parse(src: &str) -> Arena {
        let mut arena = Arena {
            nodes: Vec::with_capacity(64),
            root: 0,
        };
        arena.nodes.push(ANode {
            parent: None,
            kind: AKind::Element {
                name: "#document".to_owned(),
                attrs: Vec::new(),
            },
            children: Vec::new(),
        });

        let mut stack: Vec<NodeId> = vec![0];
        // Set while inside script/style/textarea/title: ignore everything until its end.
        let mut skip: Option<usize> = None;

        let mut lexer = Lexer::new(src);
        while let Some(ev) = lexer.next_event() {
            match ev {
                Event::Meta => {}
                Event::Text(text) => {
                    if skip.is_some() || text.is_empty() {
                        continue;
                    }
                    let id = arena.push_text(*stack.last().unwrap(), text);
                    let _ = id;
                }
                Event::Start {
                    name,
                    attrs,
                    self_closing,
                } => {
                    if skip.is_some() {
                        continue;
                    }
                    // Implicit end tags.
                    while stack.len() > 1 {
                        let open_name = arena
                            .name(*stack.last().unwrap())
                            .unwrap_or("")
                            .to_owned();
                        if open_name == "#document" {
                            break;
                        }
                        if implicitly_closes(&open_name, &name) {
                            stack.pop();
                        } else {
                            break;
                        }
                    }
                    let parent = *stack.last().unwrap();
                    let id = arena.push_element(parent, name.clone(), attrs);
                    if is_void(&name) || self_closing {
                        continue;
                    }
                    stack.push(id);
                    if matches!(name.as_str(), "script" | "style" | "textarea") {
                        skip = Some(stack.len());
                    }
                }
                Event::End { name } => {
                    if let Some(depth) = skip {
                        if stack.len() == depth && arena.name(*stack.last().unwrap()) == Some(&name)
                        {
                            stack.pop();
                            skip = None;
                        } else if stack.len() <= depth {
                            skip = None;
                        }
                        continue;
                    }
                    // Find the matching open element, drop everything above it.
                    let mut found = None;
                    for idx in (1..stack.len()).rev() {
                        if arena.name(stack[idx]) == Some(name.as_str()) {
                            found = Some(idx);
                            break;
                        }
                    }
                    if let Some(idx) = found {
                        stack.truncate(idx);
                    }
                }
            }
        }
        arena
    }

    fn push_element(
        &mut self,
        parent: NodeId,
        name: String,
        attrs: Vec<(String, String)>,
    ) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(ANode {
            parent: Some(parent),
            kind: AKind::Element { name, attrs },
            children: Vec::new(),
        });
        self.nodes[parent as usize].children.push(id);
        id
    }

    fn push_text(&mut self, parent: NodeId, text: String) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(ANode {
            parent: Some(parent),
            kind: AKind::Text(text),
            children: Vec::new(),
        });
        self.nodes[parent as usize].children.push(id);
        id
    }

    /// Element name of `id` (lowercased for real elements).
    pub fn name(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id as usize].kind {
            AKind::Element { name, .. } => Some(name.as_str()),
            AKind::Text(_) => None,
        }
    }

    /// Attributes of an element node.
    pub fn attrs(&self, id: NodeId) -> &[(String, String)] {
        match &self.nodes[id as usize].kind {
            AKind::Element { attrs, .. } => attrs,
            AKind::Text(_) => &[],
        }
    }

    /// Case-insensitive attribute lookup.
    pub fn attr(&self, id: NodeId, name: &str) -> Option<&str> {
        Event::attr(self.attrs(id), name)
    }

    /// Attribute value lowercased (useful for flags such as `expanded="1"`).
    pub fn attr_lc(&self, id: NodeId, name: &str) -> Option<String> {
        self.attr(id, name).map(|v| v.trim().to_ascii_lowercase())
    }

    /// True when the attribute is present and not `false`/`0`/`no`.
    pub fn attr_flag(&self, id: NodeId, name: &str) -> bool {
        match self.attr_lc(id, name) {
            None => false,
            Some(v) => !matches!(v.as_str(), "" | "0" | "false" | "no" | "off"),
        }
    }

    /// Text payload of a text node.
    pub fn text(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id as usize].kind {
            AKind::Text(t) => Some(t.as_str()),
            AKind::Element { .. } => None,
        }
    }

    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.nodes[id as usize].kind, AKind::Element { .. })
    }

    pub fn is_text(&self, id: NodeId) -> bool {
        matches!(self.nodes[id as usize].kind, AKind::Text(_))
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id as usize].children
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id as usize].parent
    }

    /// `id` attribute of an element, if non-empty.
    pub fn id_attr(&self, id: NodeId) -> Option<&str> {
        self.attr(id, "id").filter(|v| !v.trim().is_empty())
    }

    /// Class list, lowercased and split on whitespace.
    pub fn classes(&self, id: NodeId) -> Vec<String> {
        self.attr(id, "class")
            .map(|c| {
                c.split_whitespace()
                    .map(|s| s.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True when any class matches `needle` (substring, case-insensitive).
    pub fn has_class(&self, id: NodeId, needle: &str) -> bool {
        let needle = needle.to_ascii_lowercase();
        self.classes(id).iter().any(|c| c.contains(&needle))
    }

    /// Depth-first search for the first element matching `pred` (self excluded unless it matches).
    pub fn find_first<F>(&self, id: NodeId, pred: &F) -> Option<NodeId>
    where
        F: Fn(&Arena, NodeId) -> bool,
    {
        if self.is_element(id) && pred(self, id) {
            return Some(id);
        }
        for child in self.children(id) {
            if let Some(found) = self.find_first(*child, pred) {
                return Some(found);
            }
        }
        None
    }

    /// Concatenated text of a subtree (whitespace collapsed, no entity work — the lexer did it).
    pub fn subtree_text(&self, id: NodeId, collapse: bool) -> String {
        let mut out = String::new();
        self.collect_text(id, collapse, &mut out);
        if collapse {
            out.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            out
        }
    }

    fn collect_text(&self, id: NodeId, collapse: bool, out: &mut String) {
        if let Some(text) = self.text(id) {
            out.push_str(text);
            if collapse {
                out.push(' ');
            }
            return;
        }
        for child in self.children(id) {
            self.collect_text(*child, collapse, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(arena: &Arena, name: &str) -> NodeId {
        arena
            .find_first(arena.root, &|a, id| a.name(id) == Some(name))
            .unwrap_or_else(|| panic!("no <{name}> found"))
    }

    #[test]
    fn nests_and_closes() {
        let arena = Arena::parse("<div><p>a<b>b</b></p><p>c</p></div>");
        let div = find(&arena, "div");
        let paragraphs: Vec<NodeId> = arena
            .children(div)
            .iter()
            .copied()
            .filter(|id| arena.name(*id) == Some("p"))
            .collect();
        assert_eq!(paragraphs.len(), 2, "second <p> must be closed by <div> end");
        let div_text = arena.subtree_text(div, true);
        assert_eq!(div_text, "a b c");
    }

    #[test]
    fn implicit_li_close() {
        let arena = Arena::parse("<ul><li>one<li>two</ul>");
        let ul = find(&arena, "ul");
        let items = arena
            .children(ul)
            .iter()
            .filter(|id| arena.name(**id) == Some("li"))
            .count();
        assert_eq!(items, 2);
    }

    #[test]
    fn table_cells_close() {
        let arena = Arena::parse("<table><tr><td>a<td>b<tr><td>c</table>");
        let table = find(&arena, "table");
        let rows = arena
            .children(table)
            .iter()
            .filter(|id| arena.name(**id) == Some("tr"))
            .count();
        assert_eq!(rows, 2);
        let first_row = arena
            .children(table)
            .iter()
            .copied()
            .find(|id| arena.name(*id) == Some("tr"))
            .unwrap();
        let cells = arena
            .children(first_row)
            .iter()
            .filter(|id| arena.name(**id) == Some("td"))
            .count();
        assert_eq!(cells, 2);
    }

    #[test]
    fn unclosed_tags_are_auto_closed() {
        let arena = Arena::parse("<div><p>text");
        assert_eq!(arena.subtree_text(arena.root, true), "text");
    }

    #[test]
    fn unmatched_end_tags_are_ignored() {
        let arena = Arena::parse("</div><p>x</p></span>");
        assert_eq!(arena.subtree_text(arena.root, true), "x");
    }

    #[test]
    fn void_elements_have_no_children() {
        let arena = Arena::parse("<p>a<br>b<img src=x.png>c</p>");
        let p = find(&arena, "p");
        let br = arena
            .children(p)
            .iter()
            .copied()
            .find(|id| arena.name(*id) == Some("br"))
            .unwrap();
        assert!(arena.children(br).is_empty());
    }

    #[test]
    fn script_content_is_not_markup() {
        let arena = Arena::parse("<p>a</p><script>var x = '</p>';</script><p>b</p>");
        let text = arena.subtree_text(arena.root, true);
        assert!(text.contains("a b"), "got {text}");
        assert!(!text.contains("var x"));
    }

    #[test]
    fn attributes_and_classes() {
        let arena = Arena::parse(r#"<div CLASS="Alert Note" id="x">hi</div>"#);
        let div = find(&arena, "div");
        assert!(arena.has_class(div, "alert"));
        assert_eq!(arena.id_attr(div), Some("x"));
        assert_eq!(arena.attr_lc(div, "class").unwrap(), "alert note");
    }
}
