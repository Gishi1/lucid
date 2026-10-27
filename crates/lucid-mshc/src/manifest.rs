//! Reading the optional `.msha` manifest part inside a package.
//!
//! The schema of the manifest *inside* a package is not publicly standardized, so this
//! reader is deliberately shape-agnostic (`docs/DESIGN.md` §2.4): any element carrying
//! `Id`/`Title`/`Url` (or `Href`/`Topic`/`Name`, in any case, with or without namespaces)
//! is treated as a TOC node, anything carrying `Keyword` is treated as an index entry, and
//! everything else is descended into. A flat manifest that links nodes by parent id is
//! rebuilt into a tree. The TOC is always cross-checked against topic meta tags by
//! [`crate::package`] and [`crate::docset`], so an unreadable manifest never costs content.

use crate::error::{Error, Result};

/// One TOC node as written in a manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawTocNode {
    pub id: Option<String>,
    pub title: Option<String>,
    /// Target part path (`Url`, `Href` or `Topic`).
    pub url: Option<String>,
    /// Parent node id, when the manifest is written flat.
    pub parent: Option<String>,
    pub children: Vec<RawTocNode>,
}

impl RawTocNode {
    /// Number of nodes in this subtree (including itself).
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(|c| c.count()).sum::<usize>()
    }
}

/// A parsed manifest.
#[derive(Debug, Clone, Default)]
pub struct RawManifest {
    /// Part path inside the package.
    pub part: String,
    pub title: Option<String>,
    pub locale: Option<String>,
    pub toc: Vec<RawTocNode>,
    /// `(keyword, url or topic id)` pairs.
    pub index: Vec<(String, String)>,
}

impl RawManifest {
    /// Total TOC nodes.
    pub fn toc_size(&self) -> usize {
        self.toc.iter().map(|n| n.count()).sum()
    }
}

/// Cheap check used while classifying package parts.
pub fn looks_like_manifest(norm_path: &str) -> bool {
    let base = norm_path.rsplit('/').next().unwrap_or(norm_path);
    if base.ends_with(".msha") {
        return true;
    }
    if !base.ends_with(".xml") {
        return false;
    }
    // OPC plumbing parts are not manifests.
    if base == "[content_types].xml" || base == ".rels" || norm_path.contains("_rels/") {
        return false;
    }
    base.contains("help")
        || base.contains("manifest")
        || base.contains("content")
        || base.contains("toc")
        || base.contains("package")
}

/// Parse a manifest part.
pub fn parse(part: &str, xml: &str) -> Result<RawManifest> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Xml {
        part: part.to_owned(),
        reason: e.to_string(),
    })?;
    let root = doc.root_element();

    let mut manifest = RawManifest {
        part: part.to_owned(),
        title: attr_ci(root, "Title").or_else(|| attr_ci(root, "Name")),
        locale: attr_ci(root, "Language")
            .or_else(|| attr_ci(root, "Locale"))
            .or_else(|| attr_ci(root, "Lang")),
        toc: Vec::new(),
        index: Vec::new(),
    };

    // Some manifests have the root element *be* the single TOC node.
    if attr_ci(root, "Url").is_some() && attr_ci(root, "Title").is_some() {
        let mut index = Vec::new();
        let node = build_node(root, &mut index);
        manifest.index.extend(index);
        manifest.toc = node.children;
        if manifest.title.is_none() {
            manifest.title = node.title;
        }
        return Ok(manifest);
    }

    let mut toc = Vec::new();
    let mut index = Vec::new();
    walk(root, Mode::Neutral, &mut toc, &mut index);
    manifest.toc = rebuild_by_parent(toc);
    manifest.index = index;
    Ok(manifest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Neutral,
    Toc,
    Index,
}

fn walk(
    node: roxmltree::Node<'_, '_>,
    mode: Mode,
    toc: &mut Vec<RawTocNode>,
    index: &mut Vec<(String, String)>,
) {
    for child in node.children().filter(|c| c.is_element()) {
        let name = child.tag_name().name().to_ascii_lowercase();
        let child_mode = if name.contains("index") && !name.contains("node") {
            Mode::Index
        } else if name.contains("toc") {
            Mode::Toc
        } else {
            mode
        };

        let keyword = attr_ci(child, "Keyword")
            .or_else(|| attr_ci(child, "Key"))
            .or_else(|| attr_ci(child, "Term"));
        let url = attr_ci(child, "Url")
            .or_else(|| attr_ci(child, "Href"))
            .or_else(|| attr_ci(child, "Topic"))
            .or_else(|| attr_ci(child, "Target"));
        let title = attr_ci(child, "Title").or_else(|| attr_ci(child, "Name"));
        let id = attr_ci(child, "Id").or_else(|| attr_ci(child, "Idref"));

        let is_index_entry = keyword.is_some()
            && (child_mode == Mode::Index
                || name.contains("index")
                || (title.is_none() && url.is_some()));
        if is_index_entry {
            if let Some(keyword) = keyword {
                let target = url
                    .clone()
                    .or_else(|| id.clone())
                    .unwrap_or_default();
                if !keyword.trim().is_empty() {
                    index.push((keyword.trim().to_owned(), target));
                }
            }
            // Index entries can have children (nested keywords) — keep descending.
            walk(child, Mode::Index, toc, index);
            continue;
        }

        let is_toc_node = url.is_some() || title.is_some() || id.is_some();
        let name_is_node = matches!(
            name.as_str(),
            "helptocnode" | "tocnode" | "helpnode" | "topic" | "node"
        );
        if (is_toc_node && (name_is_node || title.is_some() || url.is_some())) && !name.contains("index")
        {
            let mut collected_index = Vec::new();
            let built = build_node(child, &mut collected_index);
            index.extend(collected_index);
            if built.title.is_some() || built.url.is_some() {
                toc.push(built);
            } else {
                // Grouping element with no label of its own: hoist the children.
                toc.extend(built.children);
            }
        } else {
            walk(child, child_mode, toc, index);
        }
    }
}

fn build_node(node: roxmltree::Node<'_, '_>, index: &mut Vec<(String, String)>) -> RawTocNode {
    let mut children = Vec::new();
    walk(node, Mode::Toc, &mut children, index);
    RawTocNode {
        id: attr_ci(node, "Id").or_else(|| attr_ci(node, "Idref")),
        title: attr_ci(node, "Title").or_else(|| attr_ci(node, "Name")),
        url: attr_ci(node, "Url")
            .or_else(|| attr_ci(node, "Href"))
            .or_else(|| attr_ci(node, "Topic"))
            .or_else(|| attr_ci(node, "Target")),
        parent: attr_ci(node, "Parent")
            .or_else(|| attr_ci(node, "ParentId"))
            .or_else(|| attr_ci(node, "TocParent")),
        children,
    }
}

/// Rebuild a flat node list into a tree when nodes carry parent ids.
fn rebuild_by_parent(nodes: Vec<RawTocNode>) -> Vec<RawTocNode> {
    let has_parents = nodes.iter().any(|n| n.parent.is_some());
    if !has_parents || nodes.is_empty() {
        return nodes;
    }
    let keys: Vec<String> = nodes
        .iter()
        .map(|n| {
            n.id.as_deref()
                .or(n.url.as_deref())
                .unwrap_or_default()
                .to_ascii_lowercase()
        })
        .collect();
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (idx, node) in nodes.iter().enumerate() {
        match node
            .parent
            .as_deref()
            .map(|p| p.trim().to_ascii_lowercase())
        {
            Some(parent)
                if !parent.is_empty()
                    && !matches!(parent.as_str(), "-1" | "0" | "root")
                    && keys.iter().position(|k| *k == parent).is_some() =>
            {
                let parent_idx = keys
                    .iter()
                    .position(|k| *k == parent)
                    .expect("checked above");
                if parent_idx == idx {
                    roots.push(idx);
                } else {
                    children_of[parent_idx].push(idx);
                }
            }
            _ => roots.push(idx),
        }
    }
    // Guard against cycles: any node not reachable from a root is promoted to a root.
    let mut reachable = vec![false; nodes.len()];
    let mut stack = roots.clone();
    while let Some(idx) = stack.pop() {
        if reachable[idx] {
            continue;
        }
        reachable[idx] = true;
        for child in &children_of[idx] {
            stack.push(*child);
        }
    }
    for (idx, seen) in reachable.iter().enumerate() {
        if !seen {
            roots.push(idx);
        }
    }
    roots.sort_unstable();
    roots.dedup();

    let mut slots: Vec<Option<RawTocNode>> = nodes
        .into_iter()
        .map(|mut n| {
            n.children.clear();
            Some(n)
        })
        .collect();
    // Attach children top-down, skipping anything already placed (a malformed manifest can
    // contain a cycle even after nodes were promoted to roots).
    fn build(
        idx: usize,
        children_of: &[Vec<usize>],
        slots: &mut Vec<Option<RawTocNode>>,
        placed: &mut Vec<bool>,
    ) -> RawTocNode {
        placed[idx] = true;
        let mut kids: Vec<RawTocNode> = Vec::new();
        for child in children_of[idx].iter().copied() {
            if !placed[child] {
                kids.push(build(child, children_of, slots, placed));
            }
        }
        let mut node = slots[idx].take().unwrap_or_default();
        node.children = kids;
        node
    }
    let mut placed = vec![false; slots.len()];
    let mut out = Vec::with_capacity(roots.len());
    for idx in roots {
        if !placed[idx] {
            out.push(build(idx, &children_of, &mut slots, &mut placed));
        }
    }
    for idx in 0..placed.len() {
        if !placed[idx] {
            out.push(build(idx, &children_of, &mut slots, &mut placed));
        }
    }
    out
}

/// Case-insensitive attribute lookup on an XML element.
fn attr_ci(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.attributes()
        .find(|a| a.name().eq_ignore_ascii_case(name))
        .map(|a| a.value().trim().to_owned())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<HelpContentSet xmlns="http://schemas.microsoft.com/help/2010/06/" Language="en-us" Title="Contoso Help">
  <HelpTOC>
    <HelpTOCNode Id="root" Title="Welcome" Url="welcome.htm">
      <HelpTOCNode Id="gs" Title="Getting started" Url="topics/getting-started.htm">
        <HelpTOCNode Id="gs2" Title="Install" Url="topics/install.htm"/>
      </HelpTOCNode>
      <HelpTOCNode Id="grp" Title="Reference"/>
    </HelpTOCNode>
  </HelpTOC>
  <HelpIndex>
    <HelpIndexEntry Keyword="install" Url="topics/install.htm"/>
    <HelpIndexEntry Keyword="welcome" Url="welcome.htm"/>
  </HelpIndex>
</HelpContentSet>"#;

    #[test]
    fn parses_the_documented_shape() {
        let m = parse("pkg.msha", MANIFEST).expect("parse");
        assert_eq!(m.title.as_deref(), Some("Contoso Help"));
        assert_eq!(m.locale.as_deref(), Some("en-us"));
        assert_eq!(m.toc.len(), 1);
        assert_eq!(m.toc[0].title.as_deref(), Some("Welcome"));
        assert_eq!(m.toc[0].children.len(), 2);
        assert_eq!(m.toc_size(), 4);
        assert_eq!(m.index.len(), 2);
        assert_eq!(m.index[0].0, "install");
        assert_eq!(m.index[0].1, "topics/install.htm");
    }

    #[test]
    fn accepts_lowercase_and_other_attribute_names() {
        let xml = r#"<content-set title="T">
            <toc><node id="a" name="Alpha" href="a.htm"><node id="b" title="Beta" topic="b.htm"/></node></toc>
            <keyword-index><index-entry key="alpha" target="a.htm"/></keyword-index>
        </content-set>"#;
        let m = parse("m.msha", xml).expect("parse");
        assert_eq!(m.title.as_deref(), Some("T"));
        assert_eq!(m.toc[0].title.as_deref(), Some("Alpha"));
        assert_eq!(m.toc[0].url.as_deref(), Some("a.htm"));
        assert_eq!(m.toc[0].children.len(), 1);
        assert_eq!(m.index.len(), 1);
    }

    #[test]
    fn flat_manifest_with_parent_ids_is_rebuilt() {
        let xml = r#"<HelpContentSet Title="Flat">
            <HelpTOC>
                <HelpTOCNode Id="1" Title="One" Url="1.htm" ParentId="-1"/>
                <HelpTOCNode Id="2" Title="Two" Url="2.htm" Parent="1"/>
                <HelpTOCNode Id="3" Title="Three" Url="3.htm" Parent="1"/>
                <HelpTOCNode Id="4" Title="Four" Url="4.htm" Parent="2"/>
            </HelpTOC>
        </HelpContentSet>"#;
        let m = parse("m.msha", xml).expect("parse");
        assert_eq!(m.toc.len(), 1);
        assert_eq!(m.toc[0].children.len(), 2);
        assert_eq!(m.toc[0].children[0].children.len(), 1);
    }

    #[test]
    fn grouping_nodes_without_labels_are_hoisted() {
        let xml = r#"<HelpContentSet><toc><group>
            <HelpTOCNode Id="a" Title="A" Url="a.htm"/>
            <HelpTOCNode Id="b" Title="B" Url="b.htm"/>
        </group></toc></HelpContentSet>"#;
        let m = parse("m.msha", xml).expect("parse");
        assert_eq!(m.toc.len(), 2);
    }

    #[test]
    fn malformed_xml_is_an_error_not_a_panic() {
        let err = parse("bad.msha", "<HelpContentSet><HelpTOC>").unwrap_err();
        assert!(matches!(err, Error::Xml { .. }));
    }

    #[test]
    fn cycle_in_parent_ids_does_not_hang() {
        let xml = r#"<HelpContentSet><toc>
            <HelpTOCNode Id="a" Title="A" Parent="b"/>
            <HelpTOCNode Id="b" Title="B" Parent="a"/>
        </toc></HelpContentSet>"#;
        let m = parse("m.msha", xml).expect("parse");
        assert_eq!(m.toc_size(), 2);
    }

    #[test]
    fn manifest_detection() {
        assert!(looks_like_manifest("demo.msha"));
        assert!(looks_like_manifest("[content_types].xml") == false);
        assert!(looks_like_manifest("helpcontentsetup.xml"));
        assert!(looks_like_manifest("image.png") == false);
    }
}
