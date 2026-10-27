//! Table-of-contents reconstruction (`docs/DESIGN.md` §2.4).
//!
//! Two sources feed the tree:
//!
//! * the manifest part inside the package (explicit nesting), and
//! * the `Microsoft.Help.TocParent` / `TocOrder` meta tags on the topics themselves
//!   (the contract that every authoring tool honours).
//!
//! The meta-tag tree is always computed, because it is the only source that cannot lie:
//! it is derived from the topics that actually exist. The manifest tree is merged on top so
//! explicit ordering, grouping nodes and node titles win, and any topic that the manifest
//! forgot is appended instead of being lost.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::manifest::{RawManifest, RawTocNode};
use crate::meta::prettify_id;

/// A node of the reconstructed table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocNode<K> {
    pub title: String,
    /// Target topic; `None` for grouping nodes that only exist in a manifest.
    pub key: Option<K>,
    pub children: Vec<TocNode<K>>,
    /// Manifest node id, when the node came from a manifest.
    pub id: Option<String>,
    /// True when the node exists purely because a parent id was referenced.
    pub synthetic: bool,
    pub order: i32,
}

impl<K> TocNode<K> {
    pub fn new(title: impl Into<String>, key: Option<K>, order: i32) -> Self {
        TocNode {
            title: title.into(),
            key,
            children: Vec::new(),
            id: None,
            synthetic: false,
            order,
        }
    }

    /// Number of nodes in this subtree.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(|c| c.count()).sum::<usize>()
    }

    /// Deepest level below this node (0 = leaf).
    pub fn depth(&self) -> usize {
        self.children.iter().map(|c| c.depth() + 1).max().unwrap_or(0)
    }

    pub fn walk<F: FnMut(&TocNode<K>, usize)>(&self, level: usize, f: &mut F) {
        f(self, level);
        for child in &self.children {
            child.walk(level + 1, f);
        }
    }

    /// Depth-first iteration of the whole forest.
    pub fn walk_forest<F: FnMut(&TocNode<K>, usize)>(forest: &[TocNode<K>], f: &mut F) {
        for node in forest {
            node.walk(0, f);
        }
    }

    /// Find the first node with `key`.
    pub fn find<F: Fn(&K) -> bool>(forest: &[TocNode<K>], pred: F) -> Option<&TocNode<K>> {
        // Recursion goes through `&F` so the compiler instantiates one type, not `&&&…F`.
        fn rec<'a, K, F: Fn(&K) -> bool>(
            forest: &'a [TocNode<K>],
            pred: &F,
        ) -> Option<&'a TocNode<K>> {
            for node in forest {
                if let Some(key) = node.key.as_ref() {
                    if pred(key) {
                        return Some(node);
                    }
                }
                if let Some(found) = rec(&node.children, pred) {
                    return Some(found);
                }
            }
            None
        }
        rec(forest, &pred)
    }

    /// Path of ancestor titles leading to `key` (excluding the node itself).
    pub fn path_to<F: Fn(&K) -> bool>(forest: &[TocNode<K>], pred: F) -> Vec<String> {
        fn rec<K, F: Fn(&K) -> bool>(
            forest: &[TocNode<K>],
            pred: &F,
            stack: &mut Vec<String>,
        ) -> bool {
            for node in forest {
                if node.key.as_ref().is_some_and(|k| pred(k)) {
                    return true;
                }
                if !node.children.is_empty() {
                    stack.push(node.title.clone());
                    if rec(&node.children, pred, stack) {
                        return true;
                    }
                    stack.pop();
                }
            }
            false
        }
        let mut stack = Vec::new();
        if rec(forest, &pred, &mut stack) {
            stack
        } else {
            Vec::new()
        }
    }
}

/// One topic as seen by the meta-tag TOC builder.
#[derive(Debug, Clone)]
pub struct MetaItem<K> {
    pub key: K,
    pub id: String,
    /// Raw `Microsoft.Help.TocParent` value.
    pub parent: Option<String>,
    pub order: i32,
    pub title: String,
}

/// Sort key used everywhere: `TocOrder`, then title (case-insensitive), then title, so the
/// result is deterministic even when orders collide.
fn sort_nodes<K>(nodes: &mut [TocNode<K>]) {
    nodes.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.title.cmp(&b.title))
    });
    for node in nodes.iter_mut() {
        sort_nodes(&mut node.children);
    }
}

/// Build the TOC from `Microsoft.Help.TocParent` / `TocOrder` meta tags.
pub fn from_meta<K: Clone + Eq + Hash>(items: &[MetaItem<K>]) -> Vec<TocNode<K>> {
    // id → item index (first wins; duplicates are resolved by the package layer).
    let mut by_id: HashMap<String, usize> = HashMap::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let key = item.id.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        by_id.entry(key).or_insert(idx);
    }

    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    // Parent ids that have no topic of their own become synthetic grouping nodes.
    let mut synthetic: HashMap<String, Vec<usize>> = HashMap::new();

    for (idx, item) in items.iter().enumerate() {
        let parent = item
            .parent
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| p.to_ascii_lowercase());
        let own_id = item.id.trim().to_ascii_lowercase();
        match parent.filter(|parent| *parent != own_id) {
            Some(parent) => match by_id.get(&parent) {
                // A parent id that resolves to a real topic always wins, even when it looks
                // like a marker (`root` is a perfectly good topic id).
                Some(parent_idx)
                    if *parent_idx != idx && !creates_cycle(&by_id, items, idx, *parent_idx) =>
                {
                    children.entry(*parent_idx).or_default().push(idx);
                }
                Some(_) => roots.push(idx),
                None if is_root_marker(&parent) => roots.push(idx),
                None => synthetic.entry(parent).or_default().push(idx),
            },
            None => roots.push(idx),
        }
    }

    /// Recursive builder (a nested `fn` rather than a closure, so it can call itself).
    fn build<K: Clone>(
        idx: usize,
        items: &[MetaItem<K>],
        children: &HashMap<usize, Vec<usize>>,
    ) -> TocNode<K> {
        let item = &items[idx];
        let mut node = TocNode::new(item.title.clone(), Some(item.key.clone()), item.order);
        node.id = Some(item.id.clone());
        let mut kids: Vec<TocNode<K>> = children
            .get(&idx)
            .map(|list| {
                list.iter()
                    .copied()
                    .map(|child| build(child, items, children))
                    .collect()
            })
            .unwrap_or_default();
        sort_nodes(&mut kids);
        node.children = kids;
        node
    }

    let mut forest: Vec<TocNode<K>> = Vec::new();

    // Synthetic parents first, in id order (they have no order value of their own).
    let mut synthetic_ids: Vec<&String> = synthetic.keys().collect();
    synthetic_ids.sort();
    for id in synthetic_ids {
        let members = &synthetic[id];
        let mut node = TocNode::new(prettify_id(id), None, i32::MAX);
        node.synthetic = true;
        node.id = Some(id.clone());
        let mut kids: Vec<TocNode<K>> = members
            .iter()
            .map(|idx| build(*idx, items, &children))
            .collect();
        sort_nodes(&mut kids);
        node.children = kids;
        // A synthetic node that has exactly one child with the same title is redundant, but
        // keeping it preserves the authored structure — so it stays.
        forest.push(node);
    }

    let mut root_nodes: Vec<TocNode<K>> = roots
        .iter()
        .map(|idx| {
            // TocOrder is relative to siblings, and roots are siblings, so no adjustment is
            // needed when promoting them.
            let mut node = build(*idx, items, &children);
            node.synthetic = false;
            node
        })
        .collect();
    sort_nodes(&mut root_nodes);
    forest.extend(root_nodes);

    // Anything unreachable (cycle members) must still be visible.
    let mut seen: HashSet<usize> = HashSet::new();
    for node in &forest {
        collect_seen(node, &by_id, items, &mut seen);
    }
    for idx in 0..items.len() {
        if !seen.contains(&idx) {
            forest.push(build(idx, items, &children));
        }
    }

    forest
}

fn collect_seen<K: Clone + Eq + Hash>(
    node: &TocNode<K>,
    by_id: &HashMap<String, usize>,
    items: &[MetaItem<K>],
    seen: &mut HashSet<usize>,
) {
    if let Some(id) = node.id.as_deref() {
        if let Some(idx) = by_id.get(&id.to_ascii_lowercase()) {
            seen.insert(*idx);
        }
    }
    // Nodes without ids: match by key identity.
    if node.id.is_none() {
        if let Some(key) = node.key.as_ref() {
            for (idx, item) in items.iter().enumerate() {
                if &item.key == key {
                    seen.insert(idx);
                }
            }
        }
    }
    for child in &node.children {
        collect_seen(child, by_id, items, seen);
    }
}

fn is_root_marker(parent: &str) -> bool {
    matches!(
        parent.trim().to_ascii_lowercase().as_str(),
        "-1" | "-2" | "0" | "root" | "top" | "(root)"
    )
}

/// Would making `child` a child of `parent` create a cycle?
fn creates_cycle<K: Clone + Eq + Hash>(
    by_id: &HashMap<String, usize>,
    items: &[MetaItem<K>],
    child: usize,
    parent: usize,
) -> bool {
    let mut cursor = parent;
    for _ in 0..items.len() + 1 {
        if cursor == child {
            return true;
        }
        let next = items
            .get(cursor)
            .and_then(|item| item.parent.as_deref())
            .map(|p| p.trim().to_ascii_lowercase())
            .and_then(|p| by_id.get(&p).copied());
        match next {
            Some(next) => cursor = next,
            None => return false,
        }
    }
    true
}

/// Convert a manifest TOC into a [`TocNode`] forest, resolving urls with `resolve`.
///
/// `resolve` receives the raw `Url` value and returns the key of the topic it points at.
/// Nodes whose target cannot be resolved keep their title and are marked synthetic.
pub fn from_manifest<K: Clone>(
    manifest: &RawManifest,
    mut resolve: impl FnMut(&str) -> Option<K>,
    mut id_resolve: impl FnMut(&str) -> Option<K>,
) -> Vec<TocNode<K>> {
    fn convert<K: Clone>(
        node: &RawTocNode,
        resolve: &mut impl FnMut(&str) -> Option<K>,
        id_resolve: &mut impl FnMut(&str) -> Option<K>,
    ) -> TocNode<K> {
        let key = node
            .url
            .as_deref()
            .and_then(|url| resolve(url))
            .or_else(|| node.id.as_deref().and_then(|id| id_resolve(id)));
        let title = node
            .title
            .clone()
            .or_else(|| {
                node.url
                    .as_deref()
                    .map(|u| u.rsplit('/').next().unwrap_or(u).to_owned())
            })
            .or_else(|| node.id.clone().map(|id| prettify_id(&id)))
            .unwrap_or_else(|| "(untitled)".to_owned());
        let mut out = TocNode::new(title, key, 0);
        out.id = node.id.clone();
        out.synthetic = node.url.is_none();
        out.children = node
            .children
            .iter()
            .map(|child| convert(child, resolve, id_resolve))
            .collect();
        out
    }
    manifest
        .toc
        .iter()
        .map(|node| convert(node, &mut resolve, &mut id_resolve))
        .collect()
}

/// Merge two TOC forests: `primary` keeps its structure, `secondary` contributes only nodes
/// whose topic is missing from `primary`. Returns the merged forest and how many secondary
/// nodes were appended.
pub fn merge<K: Clone + Eq + Hash>(
    primary: Vec<TocNode<K>>,
    secondary: Vec<TocNode<K>>,
) -> (Vec<TocNode<K>>, usize) {
    let mut present: HashSet<K> = HashSet::new();
    for node in &primary {
        collect_keys(node, &mut present);
    }
    let mut added = 0usize;
    let mut merged = primary;
    append_missing(&mut merged, secondary, &mut present, &mut added);
    (merged, added)
}

fn collect_keys<K: Clone + Eq + Hash>(node: &TocNode<K>, out: &mut HashSet<K>) {
    if let Some(key) = node.key.as_ref() {
        out.insert(key.clone());
    }
    for child in &node.children {
        collect_keys(child, out);
    }
}

fn append_missing<K: Clone + Eq + Hash>(
    target: &mut Vec<TocNode<K>>,
    incoming: Vec<TocNode<K>>,
    present: &mut HashSet<K>,
    added: &mut usize,
) {
    for node in incoming {
        let mut node = node;
        let children = std::mem::take(&mut node.children);
        match node.key.clone() {
            Some(key) => {
                if present.contains(&key) {
                    // Already placed by the primary tree: graft any *missing* descendants
                    // onto the canonical node so a forgotten child still appears where the
                    // author put it. The node itself is never duplicated — even when the two
                    // trees disagree about its parent (a topic is one place in the contents).
                    if let Some(existing) = find_mut(target, &key) {
                        append_missing(&mut existing.children, children, present, added);
                    }
                } else {
                    present.insert(key);
                    *added += 1;
                    node.children = Vec::new();
                    append_missing(&mut node.children, children, present, added);
                    target.push(node);
                }
            }
            None => {
                // A grouping node (a parent id with no topic of its own). Keep it only when
                // it still holds something after merging, so no empty branch appears.
                if !has_missing(&children, present) {
                    continue;
                }
                *added += 1;
                node.children = Vec::new();
                append_missing(&mut node.children, children, present, added);
                if node.children.is_empty() {
                    *added -= 1;
                } else {
                    target.push(node);
                }
            }
        }
    }
}

/// Does this subtree contain a topic that is not in `present` yet?
fn has_missing<K: Eq + Hash>(nodes: &[TocNode<K>], present: &HashSet<K>) -> bool {
    nodes.iter().any(|node| match node.key.as_ref() {
        Some(key) => !present.contains(key),
        None => has_missing(&node.children, present),
    })
}

fn find_mut<'a, K: PartialEq>(forest: &'a mut [TocNode<K>], key: &K) -> Option<&'a mut TocNode<K>> {
    for node in forest.iter_mut() {
        if node.key.as_ref() == Some(key) {
            return Some(node);
        }
        if let Some(found) = find_mut(&mut node.children, key) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest;

    fn item(key: u32, id: &str, parent: Option<&str>, order: i32, title: &str) -> MetaItem<u32> {
        MetaItem {
            key,
            id: id.to_owned(),
            parent: parent.map(str::to_owned),
            order,
            title: title.to_owned(),
        }
    }

    fn titles<K>(forest: &[TocNode<K>]) -> Vec<String> {
        let mut out = Vec::new();
        fn rec<K>(forest: &[TocNode<K>], out: &mut Vec<String>, level: usize) {
            for node in forest {
                out.push(format!("{}{}", "  ".repeat(level), node.title));
                rec(&node.children, out, level + 1);
            }
        }
        rec(forest, &mut out, 0);
        out
    }

    #[test]
    fn builds_from_meta_tags() {
        let items = vec![
            item(0, "root", Some("-1"), 0, "Root"),
            item(1, "a", Some("root"), 2, "Second"),
            item(2, "b", Some("root"), 1, "First"),
            item(3, "c", Some("a"), 1, "Child of Second"),
        ];
        let forest = from_meta(&items);
        assert_eq!(
            titles(&forest),
            vec!["Root", "  First", "  Second", "    Child of Second"]
        );
    }

    #[test]
    fn missing_parents_become_grouping_nodes() {
        let items = vec![
            item(0, "a", Some("Guides"), 1, "Install"),
            item(1, "b", Some("Guides"), 2, "Configure"),
        ];
        let forest = from_meta(&items);
        assert_eq!(forest.len(), 1);
        assert!(forest[0].synthetic);
        assert_eq!(forest[0].title, "Guides");
        assert_eq!(forest[0].children.len(), 2);
    }

    #[test]
    fn orphan_topics_are_never_dropped() {
        let items = vec![
            item(0, "a", None, 0, "A"),
            item(1, "b", Some("nonexistent"), 0, "B"),
            item(2, "c", Some("c"), 0, "C"),
        ];
        let forest = from_meta(&items);
        let mut flat = Vec::new();
        TocNode::walk_forest(&forest, &mut |node, _| flat.push(node.title.clone()));
        assert!(flat.contains(&"A".to_owned()));
        assert!(flat.contains(&"B".to_owned()));
        assert!(flat.contains(&"C".to_owned()));
    }

    #[test]
    fn parent_cycles_do_not_hang() {
        let items = vec![
            item(0, "a", Some("b"), 0, "A"),
            item(1, "b", Some("a"), 0, "B"),
        ];
        let forest = from_meta(&items);
        let mut count = 0;
        TocNode::walk_forest(&forest, &mut |_, _| count += 1);
        assert_eq!(count, 2);
    }

    #[test]
    fn a_topic_never_appears_twice_when_the_two_trees_disagree() {
        // The manifest places `orphan` at the root; the meta tags point it at a parent that
        // has no topic. Merging must not duplicate it, nor leave an empty grouping node.
        let manifest_toc = vec![
            TocNode::new("Intro", Some(0u32), 0),
            {
                let mut node = TocNode::new("Orphan", Some(1u32), 1);
                node.synthetic = false;
                node
            },
        ];
        let meta_toc = vec![
            TocNode {
                title: "Intro".to_owned(),
                key: Some(0),
                children: Vec::new(),
                id: Some("intro".to_owned()),
                synthetic: false,
                order: 0,
            },
            {
                let mut group = TocNode::new("Missing Parent", None, i32::MAX);
                group.synthetic = true;
                group.children = vec![TocNode::new("Orphan", Some(1u32), 0)];
                group
            },
        ];
        let (merged, added) = merge(manifest_toc, meta_toc);
        assert_eq!(added, 0, "nothing new was added");
        let mut titles = Vec::new();
        TocNode::walk_forest(&merged, &mut |node, level| {
            titles.push(format!("{}{}", "  ".repeat(level), node.title))
        });
        assert_eq!(titles, vec!["Intro", "Orphan"]);
    }

    #[test]
    fn manifest_tree_wins_and_meta_adds_the_rest() {
        let xml = r#"<HelpContentSet Title="T"><HelpTOC>
            <HelpTOCNode Id="root" Title="Authored Root" Url="a.htm">
                <HelpTOCNode Id="x" Title="Authored Child" Url="b.htm"/>
            </HelpTOCNode>
        </HelpTOC></HelpContentSet>"#;
        let manifest = manifest::parse("m.msha", xml).unwrap();
        let manifest_toc: Vec<TocNode<u32>> = from_manifest(
            &manifest,
            |url| match url {
                "a.htm" => Some(0u32),
                "b.htm" => Some(1),
                _ => None,
            },
            |_| None,
        );
        let meta_toc = from_meta(&[
            item(0, "root", None, 0, "Meta Root"),
            item(1, "x", Some("root"), 1, "Meta Child"),
            item(2, "extra", Some("root"), 2, "Only in meta"),
        ]);
        let (merged, added) = merge(manifest_toc, meta_toc);
        assert_eq!(added, 1);
        let mut flat = Vec::new();
        TocNode::walk_forest(&merged, &mut |node, _| flat.push(node.title.clone()));
        assert_eq!(
            flat,
            vec!["Authored Root", "Authored Child", "Only in meta"]
        );
    }

    #[test]
    fn path_to_and_find() {
        let items = vec![
            item(0, "a", Some("-1"), 0, "A"),
            item(1, "b", Some("a"), 0, "B"),
            item(2, "c", Some("b"), 0, "C"),
        ];
        let forest = from_meta(&items);
        assert!(TocNode::find(&forest, |k| *k == 2).is_some());
        let path = TocNode::path_to(&forest, |k| *k == 2);
        assert_eq!(path, vec!["A".to_owned(), "B".to_owned()]);
        assert_eq!(forest[0].depth(), 2);
    }

    #[test]
    fn manifest_nodes_without_urls_stay_as_grouping() {
        let xml = r#"<HelpContentSet><HelpTOC>
            <HelpTOCNode Id="grp" Title="Reference">
                <HelpTOCNode Id="a" Title="API" Url="api.htm"/>
            </HelpTOCNode>
        </HelpTOC></HelpContentSet>"#;
        let m = manifest::parse("m.msha", xml).unwrap();
        let toc: Vec<TocNode<u32>> = from_manifest(&m, |url| (url == "api.htm").then_some(7u32), |_| None);
        assert_eq!(toc.len(), 1);
        assert!(toc[0].synthetic);
        assert_eq!(toc[0].children[0].key, Some(7));
    }
}
