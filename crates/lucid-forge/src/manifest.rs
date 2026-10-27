//! The `HelpContentSet` manifest (`*.msha`): TOC synthesis plus the writer and the tolerant
//! reader used by [`crate::validate`].
//!
//! The document shape is the one in `docs/DESIGN.md` §5:
//!
//! ```xml
//! <HelpContentSet xmlns="http://schemas.microsoft.com/help/2010/06/" Language="en-us" Title="…">
//!   <HelpTOC>
//!     <HelpTOCNode Id="…" Title="…" Url="topics/intro.htm">
//!       <HelpTOCNode Id="…" Title="…" Url="…"/>
//!     </HelpTOCNode>
//!   </HelpTOC>
//!   <HelpIndex>
//!     <HelpIndexEntry Keyword="…" Url="topics/intro.htm"/>
//!   </HelpIndex>
//! </HelpContentSet>
//! ```

use std::collections::HashMap;

use crate::options::is_root_parent_id;

/// The namespace every real package uses for the manifest.
pub const HELP_NAMESPACE: &str = "http://schemas.microsoft.com/help/2010/06/";

/// One topic as the TOC builder sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicRef {
    /// Path of the topic inside the package (`topics/intro.htm`).
    pub path: String,
    /// `Microsoft.Help.Id` (or the derived id).
    pub id: String,
    /// Title used as the TOC label.
    pub title: String,
    /// Raw `Microsoft.Help.TocParent`.
    pub parent: Option<String>,
    /// Resolved `Microsoft.Help.TocOrder` (absent/garbage is 0).
    pub order: i32,
}

/// A node of the TOC written into the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocNode {
    /// `Id` attribute (the topic id; never empty).
    pub id: String,
    /// `Title` attribute.
    pub title: String,
    /// `Url` attribute: the part path, when the node is backed by a topic.
    pub url: Option<String>,
    /// Nested nodes, already in TOC order.
    pub children: Vec<TocNode>,
}

impl TocNode {
    /// A leaf node for `topic`.
    fn leaf(topic: &TopicRef) -> TocNode {
        TocNode {
            id: topic.id.clone(),
            title: topic.title.clone(),
            url: Some(topic.path.clone()),
            children: Vec::new(),
        }
    }
}

/// Everything [`write_manifest`] needs to emit one `HelpContentSet` part.
#[derive(Debug, Clone)]
pub struct ManifestSpec<'a> {
    /// `<HelpContentSet Title>` value.
    pub title: &'a str,
    /// `<HelpContentSet Language>` value.
    pub locale: &'a str,
    /// TOC roots, in order.
    pub toc: &'a [TocNode],
    /// `(keyword, url)` index entries, in order.
    pub index: &'a [(String, String)],
}

/// One TOC node read back from a manifest part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TocNodeSummary {
    /// `Id` attribute (`""` when absent).
    pub id: String,
    /// `Title` attribute (`""` when absent).
    pub title: String,
    /// `Url` attribute (`None` when absent).
    pub url: Option<String>,
}

/// What validation needs from a manifest part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestSummary {
    /// Every TOC node that carried an id, a title or a url.
    pub toc_nodes: Vec<TocNodeSummary>,
    /// Every index entry as `(keyword, url)`.
    pub index_entries: Vec<(String, String)>,
}

/// Synthesize the TOC from `Microsoft.Help.TocParent` / `Microsoft.Help.TocOrder` (DESIGN §2.4).
///
/// Siblings are ordered by `TocOrder`, then by title (case-insensitively), then by path, so the
/// result is fully deterministic. Topics whose parent does not resolve — and topics that only a
/// cycle connects — become TOC roots instead of being dropped, and a human-readable warning is
/// returned for each.
pub fn toc_from_topics(topics: &[TopicRef], root_id: &str) -> (Vec<TocNode>, Vec<String>) {
    let key = |value: &str| value.trim().to_ascii_lowercase();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (index, topic) in topics.iter().enumerate() {
        by_id.entry(key(&topic.id)).or_insert(index);
    }

    let mut builder = Builder::new(topics);
    let mut parent: Vec<Option<usize>> = vec![None; topics.len()];
    for (index, topic) in topics.iter().enumerate() {
        let raw = topic.parent.as_deref().unwrap_or("").trim();
        if raw.is_empty() || is_root_parent_id(root_id, raw) {
            continue;
        }
        match by_id.get(&key(raw)) {
            Some(&found) if found != index => parent[index] = Some(found),
            Some(_) => builder.warn(format!(
                "topic {} lists itself as Microsoft.Help.TocParent; it becomes a TOC root",
                topic.path
            )),
            None => builder.warn(format!(
                "topic {} has an unresolvable Microsoft.Help.TocParent (\"{raw}\"); it becomes a \
                 TOC root",
                topic.path
            )),
        }
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); topics.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (index, parent) in parent.iter().enumerate() {
        match *parent {
            Some(found) => children[found].push(index),
            None => roots.push(index),
        }
    }
    let order_of = |index: usize| {
        let topic = &topics[index];
        (
            topic.order,
            topic.title.to_lowercase(),
            topic.title.clone(),
            topic.path.clone(),
        )
    };
    for list in children.iter_mut() {
        list.sort_by_key(|index| order_of(*index));
    }
    roots.sort_by_key(|index| order_of(*index));

    let mut nodes: Vec<TocNode> = Vec::new();
    for root in roots {
        nodes.push(builder.render(root, &children));
    }
    // A cycle that no root reaches never gets visited above; promote its members so no topic is
    // lost (DESIGN §2.4: "those become roots, never dropped").
    for (index, topic) in topics.iter().enumerate() {
        if builder.is_pending(index) {
            builder.warn(format!(
                "topic {} is only reachable through a Microsoft.Help.TocParent cycle; it becomes \
                 a TOC root",
                topic.path
            ));
            nodes.push(builder.render(index, &children));
        }
    }
    (nodes, builder.warnings)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    Visiting,
    Done,
}

struct Builder<'a> {
    topics: &'a [TopicRef],
    state: Vec<State>,
    warnings: Vec<String>,
}

impl<'a> Builder<'a> {
    fn new(topics: &'a [TopicRef]) -> Self {
        Builder {
            topics,
            state: vec![State::Pending; topics.len()],
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }

    fn is_pending(&self, index: usize) -> bool {
        self.state.get(index) == Some(&State::Pending)
    }

    /// The manifest node for topic `index` (an empty node when the index is out of range).
    fn leaf(&self, index: usize) -> TocNode {
        match self.topics.get(index) {
            Some(topic) => TocNode::leaf(topic),
            None => TocNode {
                id: String::new(),
                title: String::new(),
                url: None,
                children: Vec::new(),
            },
        }
    }

    /// Render the subtree rooted at `index` without recursion, so a pathological package cannot
    /// overflow the stack.
    fn render(&mut self, root: usize, children: &[Vec<usize>]) -> TocNode {
        let mut stack: Vec<(usize, usize, TocNode)> = vec![(root, 0, self.leaf(root))];
        if let Some(state) = self.state.get_mut(root) {
            *state = State::Visiting;
        }
        while let Some((index, position, node)) = stack.pop() {
            let kids: Vec<usize> = children.get(index).cloned().unwrap_or_default();
            if position < kids.len() {
                stack.push((index, position + 1, node));
                let child = kids[position];
                match self.state.get(child).copied() {
                    Some(State::Pending) => {
                        if let Some(state) = self.state.get_mut(child) {
                            *state = State::Visiting;
                        }
                        let child_node = self.leaf(child);
                        stack.push((child, 0, child_node));
                    }
                    Some(State::Visiting) => {
                        // A back edge: cut it, otherwise the TOC would never terminate.
                        let path = self
                            .topics
                            .get(child)
                            .map(|topic| topic.path.clone())
                            .unwrap_or_default();
                        self.warn(format!(
                            "topic {path} closes a Microsoft.Help.TocParent cycle; the repeating \
                             link was dropped"
                        ));
                    }
                    _ => {}
                }
            } else {
                if let Some(state) = self.state.get_mut(index) {
                    *state = State::Done;
                }
                match stack.last_mut() {
                    Some((_, _, parent)) => parent.children.push(node),
                    None => return node,
                }
            }
        }
        self.leaf(root)
    }
}

/// Build the `.msha` document.
pub fn write_manifest(spec: &ManifestSpec<'_>) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str(&format!(
        "<HelpContentSet xmlns=\"{HELP_NAMESPACE}\" Language=\"{}\" Title=\"{}\">\n",
        escape_xml(spec.locale),
        escape_xml(spec.title)
    ));
    if spec.toc.is_empty() {
        out.push_str("  <HelpTOC />\n");
    } else {
        out.push_str("  <HelpTOC>\n");
        write_nodes(&mut out, spec.toc, 2);
        out.push_str("  </HelpTOC>\n");
    }
    if spec.index.is_empty() {
        out.push_str("  <HelpIndex />\n");
    } else {
        out.push_str("  <HelpIndex>\n");
        for (keyword, url) in spec.index {
            out.push_str(&format!(
                "    <HelpIndexEntry Keyword=\"{}\" Url=\"{}\" />\n",
                escape_xml(keyword),
                escape_xml(url)
            ));
        }
        out.push_str("  </HelpIndex>\n");
    }
    out.push_str("</HelpContentSet>\n");
    out
}

fn write_nodes(out: &mut String, nodes: &[TocNode], depth: usize) {
    let indent = "  ".repeat(depth);
    for node in nodes {
        let mut tag = format!(
            "<HelpTOCNode Id=\"{}\" Title=\"{}\"",
            escape_xml(&node.id),
            escape_xml(&node.title)
        );
        if let Some(url) = &node.url {
            tag.push_str(&format!(" Url=\"{}\"", escape_xml(url)));
        }
        if node.children.is_empty() {
            out.push_str(&format!("{indent}{tag} />\n"));
        } else {
            out.push_str(&format!("{indent}{tag}>\n"));
            write_nodes(out, &node.children, depth + 1);
            out.push_str(&format!("{indent}</HelpTOCNode>\n"));
        }
    }
}

/// Read a manifest part tolerantly: element names are matched case-insensitively and without
/// namespaces, and `Id`/`Title`/`Url`/`Keyword` attributes in any case are accepted
/// (DESIGN §2.4, assumption A-3).
///
/// Returns `None` when the part is not well-formed XML.
pub fn read_manifest(xml: &str) -> Option<ManifestSummary> {
    let document = roxmltree::Document::parse(xml).ok()?;
    let mut summary = ManifestSummary::default();
    for node in document.descendants().filter(|node| node.is_element()) {
        let tag = node.tag_name().name().to_ascii_lowercase();
        let attr = |name: &str| -> Option<String> {
            node.attributes()
                .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
                .map(|attribute| attribute.value().to_owned())
        };
        match tag.as_str() {
            "helptocnode" | "tocnode" | "helptopic" | "topic" | "node" => {
                let id = attr("id").unwrap_or_default();
                let title = attr("title").unwrap_or_default();
                let url = attr("url").or_else(|| attr("link"));
                if id.is_empty() && title.is_empty() && url.is_none() {
                    continue;
                }
                summary.toc_nodes.push(TocNodeSummary { id, title, url });
            }
            "helpindexentry" | "helpkeywordindexentry" | "indexentry" | "keyword" => {
                summary.index_entries.push((
                    attr("keyword").or_else(|| attr("name")).unwrap_or_default(),
                    attr("url").unwrap_or_default(),
                ));
            }
            _ => {}
        }
    }
    Some(summary)
}

/// Escape text for use in XML attributes and text nodes.
///
/// Characters that XML 1.0 cannot represent at all are replaced with a space rather than
/// producing a part no parser will accept.
pub fn escape_xml(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(ch),
            '\u{FFFE}' | '\u{FFFF}' => out.push(' '),
            other if (other as u32) < 0x20 => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(path: &str, id: &str, title: &str, parent: Option<&str>, order: i32) -> TopicRef {
        TopicRef {
            path: path.to_owned(),
            id: id.to_owned(),
            title: title.to_owned(),
            parent: parent.map(str::to_owned),
            order,
        }
    }

    #[test]
    fn orders_by_toc_order_then_title() {
        let topics = vec![
            topic("topics/b.htm", "B", "Zebra", Some("-1"), 20),
            topic("topics/a.htm", "A", "Alpha", None, -5),
            topic("topics/c.htm", "C", "Beta", Some("b"), 10),
            topic("topics/d.htm", "D", "Beta", Some("B"), 5),
        ];
        let (roots, warnings) = toc_from_topics(&topics, "-1");
        assert!(warnings.is_empty());
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].id, "A");
        assert_eq!(roots[1].id, "B");
        let children: Vec<&str> = roots[1]
            .children
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        assert_eq!(children, vec!["D", "C"]); // order 5 before 10
    }

    #[test]
    fn unresolvable_parents_become_roots_with_a_warning() {
        let topics = vec![
            topic("topics/a.htm", "A", "Alpha", Some("Missing"), 1),
            topic("topics/b.htm", "B", "Beta", None, 2),
        ];
        let (roots, warnings) = toc_from_topics(&topics, "-1");
        assert_eq!(roots.len(), 2);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unresolvable"), "{warnings:?}");
        assert!(warnings[0].contains("topics/a.htm"));
    }

    #[test]
    fn cycles_are_cut_and_never_lost() {
        let topics = vec![
            topic("topics/a.htm", "A", "Alpha", Some("B"), 1),
            topic("topics/b.htm", "B", "Beta", Some("A"), 2),
            topic("topics/c.htm", "C", "Gamma", Some("C"), 3),
        ];
        let (roots, warnings) = toc_from_topics(&topics, "-1");
        let ids: Vec<&str> = roots.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, vec!["C", "A"]);
        assert_eq!(roots[1].children.len(), 1);
        assert_eq!(roots[1].children[0].id, "B");
        assert!(roots[1].children[0].children.is_empty());
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("lists itself")));
        assert!(warnings.iter().any(|w| w.contains("cycle")));
    }

    #[test]
    fn the_configured_root_id_is_honoured() {
        let topics = vec![
            topic("topics/a.htm", "A", "Alpha", Some("ROOT"), 1),
            topic("topics/b.htm", "B", "Beta", Some("A"), 2),
        ];
        let (roots, warnings) = toc_from_topics(&topics, "ROOT");
        assert!(warnings.is_empty());
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].children[0].id, "B");
    }

    #[test]
    fn manifest_round_trips_through_the_reader() {
        let topics = vec![
            topic("topics/a.htm", "A", "Alpha & <Beta>", Some("-1"), 1),
            topic("topics/b.htm", "B", "Child", Some("A"), 2),
        ];
        let (toc, _) = toc_from_topics(&topics, "-1");
        let index = vec![("beta & co".to_owned(), "topics/b.htm".to_owned())];
        let xml = write_manifest(&ManifestSpec {
            title: "Fixture \"quoted\"",
            locale: "en-us",
            toc: &toc,
            index: &index,
        });
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n"));
        assert!(xml.contains("Alpha &amp; &lt;Beta&gt;"));
        assert!(xml.contains("Title=\"Fixture &quot;quoted&quot;\""));
        assert!(xml.ends_with("</HelpContentSet>\n"));

        let summary = read_manifest(&xml).expect("the writer emits well-formed XML");
        assert_eq!(summary.toc_nodes.len(), 2);
        assert_eq!(summary.toc_nodes[0].id, "A");
        assert_eq!(summary.toc_nodes[0].title, "Alpha & <Beta>");
        assert_eq!(summary.toc_nodes[1].url.as_deref(), Some("topics/b.htm"));
        assert_eq!(
            summary.index_entries,
            vec![("beta & co".to_owned(), "topics/b.htm".to_owned())]
        );
    }

    #[test]
    fn the_reader_accepts_foreign_shapes() {
        let xml = r#"<?xml version="1.0"?>
<helpContentset xmlns="http://schemas.microsoft.com/help/2010/06/">
  <HelpTOC><Topic ID="a" TITLE="Alpha" URL="a.htm"><node id="b" title="B" url="b.htm"/></Topic></HelpTOC>
  <HelpIndex><HelpKeywordIndexEntry Keyword="k" Url="a.htm"/></HelpIndex>
</helpContentset>"#;
        let summary = read_manifest(xml).unwrap();
        assert_eq!(summary.toc_nodes.len(), 2);
        assert_eq!(summary.toc_nodes[0].id, "a");
        assert_eq!(summary.toc_nodes[1].title, "B");
        assert_eq!(
            summary.index_entries,
            vec![("k".to_owned(), "a.htm".to_owned())]
        );
        assert!(read_manifest("<not-closed").is_none());
    }

    #[test]
    fn escaping_removes_illegal_xml_characters() {
        assert_eq!(escape_xml("a\u{1}b\u{FFFE}"), "a b ");
    }
}
