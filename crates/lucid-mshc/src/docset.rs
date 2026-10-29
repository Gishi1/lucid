//! A content set: one package, or several merged (a Help Library content-store folder, a
//! `.msha` setup document, a directory of vendor packages, a single `.chm` archive).

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::package::{AssetBytes, Package, Topic, TopicKey};
use crate::progress::{Progress, Stage};
use crate::toc::TocNode;
use crate::zipread;

/// Format version of derived data (index cache); bump when parsing semantics change.
pub const FORMAT_VERSION: u32 = 2;

/// What the user pointed Lucid at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A single `.mshc` file.
    Package,
    /// A single `.chm` archive.
    Chm,
    /// A folder containing one or more packages (content store).
    Folder,
    /// A `.msha` setup document listing packages.
    Setup,
}

impl SourceKind {
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Package => "package",
            SourceKind::Chm => "CHM archive",
            SourceKind::Folder => "content set",
            SourceKind::Setup => "content set (setup document)",
        }
    }
}

/// One entry of the keyword index pane.
#[derive(Debug, Clone)]
pub struct KeywordEntry {
    pub keyword: String,
    pub key: TopicKey,
    /// True when the keyword comes from `Microsoft.Help.F1`.
    pub f1: bool,
}

/// Where a link in a topic points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Another topic in the content set.
    Topic(TopicKey),
    /// An anchor inside a topic (possibly the current one).
    Anchor(TopicKey, String),
    /// An external URL (opened in the system browser).
    External(String),
    /// Unresolvable (shown as plain text with a tooltip).
    Missing(String),
}

/// Summary numbers for the status bar and `lucid info`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocSetStats {
    pub packages: usize,
    pub topics: usize,
    pub assets: usize,
    pub toc_nodes: usize,
    pub keywords: usize,
    pub bytes: u64,
}

/// One or more packages presented as a single browsable library.
pub struct DocSet {
    pub path: PathBuf,
    pub kind: SourceKind,
    pub packages: Vec<Package>,
    pub toc: Vec<TocNode<TopicKey>>,
    pub keywords: Vec<KeywordEntry>,
    pub warnings: Vec<String>,
    by_id: HashMap<String, TopicKey>,
    by_f1: HashMap<String, TopicKey>,
    parents: HashMap<TopicKey, Option<TopicKey>>,
}

impl std::fmt::Debug for DocSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocSet")
            .field("path", &self.path)
            .field("kind", &self.kind)
            .field("packages", &self.packages.len())
            .field("topics", &self.topic_count())
            .finish()
    }
}

impl DocSet {
    /// Open a package, a `.chm` archive, a folder of packages, or a `.msha` setup document.
    pub fn open(path: &Path, progress: &mut dyn FnMut(Progress)) -> Result<DocSet> {
        let mut warnings: Vec<String> = Vec::new();
        let (kind, package_paths) = discover(path, progress, &mut warnings)?;

        let mut packages = Vec::with_capacity(package_paths.len());
        for (index, package_path) in package_paths.iter().enumerate() {
            progress(Progress::new(
                Stage::Opening,
                index,
                package_paths.len(),
                package_path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ));
            let package = Package::open(package_path, index as u32, progress)?;
            for warning in &package.warnings {
                warnings.push(format!("{}: {warning}", package.name));
            }
            packages.push(package);
        }

        progress(Progress::new(Stage::BuildingToc, 0, 0, ""));
        let mut set = DocSet {
            path: path.to_path_buf(),
            kind,
            packages,
            toc: Vec::new(),
            keywords: Vec::new(),
            warnings,
            by_id: HashMap::new(),
            by_f1: HashMap::new(),
            parents: HashMap::new(),
        };
        set.build_toc();
        set.build_id_maps();
        set.build_keywords();
        progress(Progress::new(Stage::Done, 1, 1, ""));
        Ok(set)
    }

    /// Open from an already loaded package list (used by tests).
    pub fn from_packages(path: &Path, kind: SourceKind, packages: Vec<Package>) -> DocSet {
        let mut set = DocSet {
            path: path.to_path_buf(),
            kind,
            packages,
            toc: Vec::new(),
            keywords: Vec::new(),
            warnings: Vec::new(),
            by_id: HashMap::new(),
            by_f1: HashMap::new(),
            parents: HashMap::new(),
        };
        set.build_toc();
        set.build_id_maps();
        set.build_keywords();
        set
    }

    // ------------------------------------------------------------------ building

    fn build_toc(&mut self) {
        let multiple = self.packages.len() > 1;
        let mut forest: Vec<TocNode<TopicKey>> = Vec::new();
        /// Wrap package-local topic indices into content-set-wide keys.
        fn rekey(node: &TocNode<u32>, pkg: u32) -> TocNode<TopicKey> {
            let mut out = TocNode {
                title: node.title.clone(),
                key: node.key.map(|topic| TopicKey::new(pkg, topic)),
                children: Vec::new(),
                id: node.id.clone(),
                synthetic: node.synthetic,
                order: node.order,
            };
            out.children = node.children.iter().map(|child| rekey(child, pkg)).collect();
            out
        }
        for (pkg_index, package) in self.packages.iter().enumerate() {
            let pkg = pkg_index as u32;
            let mut nodes: Vec<TocNode<TopicKey>> = package
                .toc()
                .iter()
                .map(|node| rekey(node, pkg))
                .collect();
            if multiple {
                let mut wrapper = TocNode::new(
                    package
                        .title
                        .clone()
                        .unwrap_or_else(|| package.name.clone()),
                    None,
                    0,
                );
                wrapper.synthetic = true;
                wrapper.children = nodes;
                forest.push(wrapper);
            } else {
                forest.append(&mut nodes);
            }
        }
        self.toc = forest;
        self.index_parents();
    }

    fn index_parents(&mut self) {
        let mut parents: HashMap<TopicKey, Option<TopicKey>> = HashMap::new();
        fn walk(node: &TocNode<TopicKey>, parent: Option<TopicKey>, out: &mut HashMap<TopicKey, Option<TopicKey>>) {
            if let Some(key) = node.key {
                out.entry(key).or_insert(parent);
            }
            let next_parent = node.key.or(parent);
            for child in &node.children {
                walk(child, next_parent, out);
            }
        }
        for node in &self.toc {
            walk(node, None, &mut parents);
        }
        self.parents = parents;
    }

    fn build_id_maps(&mut self) {
        let mut by_id: HashMap<String, TopicKey> = HashMap::new();
        let mut by_f1: HashMap<String, TopicKey> = HashMap::new();
        let mut collisions: Vec<String> = Vec::new();
        for (pkg_index, package) in self.packages.iter().enumerate() {
            let pkg = pkg_index as u32;
            for topic in &package.topics {
                let key = TopicKey::new(pkg, topic.key.topic);
                if !topic.id_lc.is_empty() {
                    match by_id.get(&topic.id_lc) {
                        None => {
                            by_id.insert(topic.id_lc.clone(), key);
                        }
                        Some(existing) if *existing == key => {}
                        Some(existing) => {
                            // Prefer the topic whose own locale matches, then the first.
                            let existing_locale = self
                                .topic(*existing)
                                .and_then(|t| t.effective_locale().map(|l| l.to_ascii_lowercase()));
                            let new_locale = topic
                                .effective_locale()
                                .map(|l| l.to_ascii_lowercase());
                            if new_locale.is_some() && new_locale == self.locale_of(*existing) {
                                by_id.insert(topic.id_lc.clone(), key);
                            } else if existing_locale.is_none() && new_locale.is_some() {
                                by_id.insert(topic.id_lc.clone(), key);
                            } else {
                                collisions.push(format!(
                                    "topic id `{}` exists in {} and {}",
                                    topic.id_lc,
                                    self.packages[existing.pkg as usize].name,
                                    package.name
                                ));
                            }
                        }
                    }
                }
                for keyword in &topic.f1 {
                    by_f1
                        .entry(keyword.to_ascii_lowercase())
                        .or_insert(TopicKey::new(pkg, topic.key.topic));
                }
            }
        }
        for collision in collisions {
            if !self.warnings.contains(&collision) {
                self.warnings.push(collision);
            }
        }
        self.by_id = by_id;
        self.by_f1 = by_f1;
    }

    fn locale_of(&self, key: TopicKey) -> Option<String> {
        self.topic(key)
            .and_then(|t| t.effective_locale())
            .map(|l| l.to_ascii_lowercase())
    }

    fn build_keywords(&mut self) {
        let mut entries: Vec<KeywordEntry> = Vec::new();
        let mut seen: HashSet<(String, TopicKey)> = HashSet::new();
        for (pkg_index, package) in self.packages.iter().enumerate() {
            let pkg = pkg_index as u32;
            for (keyword, topic, f1) in package.keyword_entries() {
                let key = TopicKey::new(pkg, topic);
                if seen.insert((keyword.to_ascii_lowercase(), key)) {
                    entries.push(KeywordEntry { keyword, key, f1 });
                }
            }
        }
        entries.sort_by(|a, b| {
            a.keyword
                .to_lowercase()
                .cmp(&b.keyword.to_lowercase())
                .then_with(|| a.keyword.cmp(&b.keyword))
                .then_with(|| b.f1.cmp(&a.f1))
        });
        self.keywords = entries;
    }

    // ------------------------------------------------------------------ access

    /// Number of topics in the whole content set.
    pub fn topic_count(&self) -> usize {
        self.packages.iter().map(|p| p.topics.len()).sum()
    }

    /// Look up a topic by key.
    pub fn topic(&self, key: TopicKey) -> Option<&Topic> {
        self.packages
            .get(key.pkg as usize)
            .and_then(|pkg| pkg.topic(key.topic))
    }

    /// Title of a topic (empty string when unknown).
    pub fn topic_title(&self, key: TopicKey) -> &str {
        self.topic(key).map(|t| t.title.as_str()).unwrap_or("")
    }

    /// `Microsoft.Help.Id` of a topic.
    pub fn topic_id(&self, key: TopicKey) -> &str {
        self.topic(key).map(|t| t.id.as_str()).unwrap_or("")
    }

    /// Part path of a topic inside its package.
    pub fn topic_part(&self, key: TopicKey) -> &str {
        self.topic(key).map(|t| t.part.as_str()).unwrap_or("")
    }

    /// Topic by `Microsoft.Help.Id`.
    pub fn find_by_id(&self, id: &str) -> Option<TopicKey> {
        let needle = id.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return None;
        }
        if let Some(key) = self.by_id.get(&needle) {
            return Some(*key);
        }
        // Tolerate `Id` values that carry a suffix (`Topic#method`) — Help Viewer's
        // "base native namespace" behaviour.
        let base = needle.split(['#', '/']).next().unwrap_or(&needle).to_owned();
        self.by_id.get(&base).copied()
    }

    /// Topic by F1 keyword.
    pub fn find_by_f1(&self, keyword: &str) -> Option<TopicKey> {
        let needle = keyword.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return None;
        }
        self.by_f1.get(&needle).copied().or_else(|| {
            let base = needle.split(['#', '/']).next().unwrap_or(&needle).to_owned();
            self.by_f1.get(&base).copied()
        })
    }

    /// Read and decode a topic's HTML.
    pub fn topic_html(&self, key: TopicKey) -> Result<String> {
        self.packages
            .get(key.pkg as usize)
            .ok_or_else(|| Error::NotFound(format!("package #{}", key.pkg)))?
            .topic_html(key.topic)
    }

    /// Parse a topic body.
    pub fn document(&self, key: TopicKey) -> Result<crate::model::Document> {
        self.packages
            .get(key.pkg as usize)
            .ok_or_else(|| Error::NotFound(format!("package #{}", key.pkg)))?
            .document(key.topic)
    }

    /// Load an asset referenced from a topic (resolved inside its own package first, then
    /// across the content set).
    pub fn asset(&self, from: TopicKey, reference: &str) -> Result<Option<AssetBytes>> {
        if let Some(package) = self.packages.get(from.pkg as usize) {
            if let Some(asset) = package.asset_bytes(reference)? {
                return Ok(Some(asset));
            }
        }
        for (index, package) in self.packages.iter().enumerate() {
            if index == from.pkg as usize {
                continue;
            }
            if let Some(asset) = package.asset_bytes(reference)? {
                return Ok(Some(asset));
            }
        }
        Ok(None)
    }

    /// Resolve a link found in `from`.
    pub fn resolve(&self, from: TopicKey, href: &str) -> LinkTarget {
        let raw = href.trim();
        if raw.is_empty() {
            return LinkTarget::Missing(href.to_owned());
        }
        let lower = raw.to_ascii_lowercase();

        if lower.starts_with("ms-xhelp:")
            || lower.starts_with("ms-help:")
            || lower.starts_with("msitstore:")
        {
            if let Some(id) = extract_id(raw) {
                if let Some(key) = self.find_by_id(&id) {
                    return LinkTarget::Topic(key);
                }
                if let Some(key) = self.find_by_f1(&id) {
                    return LinkTarget::Topic(key);
                }
            }
            return LinkTarget::Missing(raw.to_owned());
        }
        // CHM links. `ms-its:book.chm::/topic.htm#anchor` and
        // `mk:@MSITStore:C:\path\book.chm::/topic.htm` both name a topic inside a CHM;
        // when the book is the one being read, resolve it locally (multi-book link
        // resolution is not supported, but the common case works).
        if lower.starts_with("ms-its:")
            || lower.starts_with("ms-itss:")
            || lower.starts_with("mk:@msitstore:")
            || lower.starts_with("ms-its ")
        {
            if let Some(reference) = extract_chm_reference(raw) {
                let (path, fragment) = split_fragment(&reference);
                if !path.trim().is_empty() {
                    if let Some(key) = self.resolve_topic_path(from, path) {
                        return match fragment {
                            Some(anchor) if !anchor.is_empty() => {
                                LinkTarget::Anchor(key, anchor.to_owned())
                            }
                            _ => LinkTarget::Topic(key),
                        };
                    }
                }
                if let Some(id) = extract_id(raw) {
                    if let Some(key) = self.find_by_id(&id).or_else(|| self.find_by_f1(&id)) {
                        return LinkTarget::Topic(key);
                    }
                }
            }
            return LinkTarget::Missing(raw.to_owned());
        }
        if is_external(raw) {
            return LinkTarget::External(raw.to_owned());
        }
        if let Some(anchor) = raw.strip_prefix('#') {
            if anchor.is_empty() {
                return LinkTarget::Anchor(from, String::new());
            }
            return LinkTarget::Anchor(from, anchor.to_owned());
        }

        let (path, fragment) = split_fragment(raw);
        if path.is_empty() {
            return match fragment {
                Some(anchor) => LinkTarget::Anchor(from, anchor.to_owned()),
                None => LinkTarget::Missing(raw.to_owned()),
            };
        }
        match self.resolve_topic_path(from, path) {
            Some(key) => match fragment {
                Some(anchor) if !anchor.is_empty() => LinkTarget::Anchor(key, anchor.to_owned()),
                _ => LinkTarget::Topic(key),
            },
            None => LinkTarget::Missing(raw.to_owned()),
        }
    }

    /// Resolve a relative or absolute topic path against the package of `from`.
    pub fn resolve_topic_path(&self, from: TopicKey, path: &str) -> Option<TopicKey> {
        let base_part = self.topic_part(from).to_owned();
        let cleaned = zipread::strip_query_and_fragment(path).trim();
        if cleaned.is_empty() {
            return None;
        }
        let absolute = cleaned.starts_with('/') || cleaned.starts_with('\\');
        let resolved = if absolute {
            zipread::normalize_path(cleaned)
        } else {
            zipread::resolve_relative(&base_part, cleaned)
        };

        if let Some(package) = self.packages.get(from.pkg as usize) {
            if let Some(topic) = package.topic_by_norm_lookup(&resolved) {
                return Some(TopicKey::new(from.pkg, topic));
            }
        }
        for (index, package) in self.packages.iter().enumerate() {
            if let Some(topic) = package.topic_by_norm_lookup(&resolved) {
                return Some(TopicKey::new(index as u32, topic));
            }
        }
        // Tolerant fallbacks: basename matching inside the source package, then ids.
        if let Some(package) = self.packages.get(from.pkg as usize) {
            if let Some(topic) = package.topic_by_reference(cleaned) {
                return Some(TopicKey::new(from.pkg, topic));
            }
        }
        for (index, package) in self.packages.iter().enumerate() {
            if let Some(topic) = package.topic_by_reference(cleaned) {
                return Some(TopicKey::new(index as u32, topic));
            }
        }
        self.find_by_id(cleaned)
    }

    /// Ancestor titles of a topic in the TOC, ending with the topic itself.
    pub fn breadcrumb(&self, key: TopicKey) -> Vec<(String, Option<TopicKey>)> {
        fn search(
            nodes: &[TocNode<TopicKey>],
            key: TopicKey,
            stack: &mut Vec<(String, Option<TopicKey>)>,
        ) -> bool {
            for node in nodes {
                stack.push((node.title.clone(), node.key));
                if node.key == Some(key) {
                    return true;
                }
                if search(&node.children, key, stack) {
                    return true;
                }
                stack.pop();
            }
            false
        }
        let mut stack = Vec::new();
        if search(&self.toc, key, &mut stack) {
            stack
        } else {
            vec![(self.topic_title(key).to_owned(), Some(key))]
        }
    }

    /// Parent topic in the TOC (for "up" navigation).
    pub fn toc_parent(&self, key: TopicKey) -> Option<TopicKey> {
        self.parents.get(&key).copied().flatten()
    }

    /// Keywords matching a substring filter, capped at `limit`.
    pub fn keyword_matches(&self, needle: &str, limit: usize) -> Vec<&KeywordEntry> {
        let needle = needle.trim().to_lowercase();
        self.keywords
            .iter()
            .filter(|entry| needle.is_empty() || entry.keyword.to_lowercase().contains(&needle))
            .take(limit)
            .collect()
    }

    /// Primary locale of the content set (majority of topics).
    pub fn primary_locale(&self) -> Option<String> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for package in &self.packages {
            if let Some(locale) = package.locale.as_ref() {
                *counts.entry(locale.to_ascii_lowercase()).or_default() += package.topics.len();
            }
        }
        counts
            .into_iter()
            .max_by_key(|(locale, count)| (*count, locale.clone()))
            .map(|(locale, _)| locale)
    }

    /// Summary numbers.
    pub fn stats(&self) -> DocSetStats {
        let mut stats = DocSetStats {
            packages: self.packages.len(),
            keywords: self.keywords.len(),
            ..Default::default()
        };
        for package in &self.packages {
            let package_stats = package.stats();
            stats.topics += package_stats.topics;
            stats.assets += package_stats.assets;
            stats.bytes += package_stats.bytes;
        }
        stats.toc_nodes = self
            .toc
            .iter()
            .map(|node| node.count())
            .sum::<usize>();
        stats
    }

    /// Cache fingerprint: everything that invalidates a persisted search index.
    pub fn fingerprint(&self) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        FORMAT_VERSION.hash(&mut hasher);
        for package in &self.packages {
            package.path.to_string_lossy().hash(&mut hasher);
            if let Ok(meta) = std::fs::metadata(&package.path) {
                meta.len().hash(&mut hasher);
                if let Ok(modified) = meta.modified() {
                    if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                        duration.as_secs().hash(&mut hasher);
                    }
                }
            }
            package.topics.len().hash(&mut hasher);
        }
        format!("{:016x}", hasher.finish())
    }

    /// Read every topic body once and feed an index builder (memory stays flat).
    pub fn extract_all(
        &self,
        progress: &mut dyn FnMut(Progress),
        mut f: impl FnMut(&Package, &Topic, crate::model::Document),
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        for package in &self.packages {
            let (_, package_warnings) = package.extract(progress, |topic, document| {
                f(package, topic, document);
            });
            warnings.extend(package_warnings);
        }
        warnings
    }

    /// Packages sorted for display/merge order: locale match first, then title, then name.
    pub fn ordered_packages(&self) -> Vec<usize> {
        let locale = self.primary_locale();
        let mut indices: Vec<usize> = (0..self.packages.len()).collect();
        indices.sort_by(|a, b| {
            let pa = &self.packages[*a];
            let pb = &self.packages[*b];
            let rank = |package: &Package| {
                match (locale.as_deref(), package.locale.as_deref()) {
                    (Some(want), Some(have)) if want.eq_ignore_ascii_case(have) => 0,
                    (_, Some(_)) => 1,
                    _ => 2,
                }
            };
            rank(pa)
                .cmp(&rank(pb))
                .then_with(|| pa.title.cmp(&pb.title))
                .then_with(|| pa.name.cmp(&pb.name))
        });
        indices
    }
}

// ---------------------------------------------------------------------- discovery

fn discover(
    path: &Path,
    progress: &mut dyn FnMut(Progress),
    warnings: &mut Vec<String>,
) -> Result<(SourceKind, Vec<PathBuf>)> {
    let metadata = std::fs::metadata(path).map_err(|e| Error::io(path, e))?;
    if metadata.is_dir() {
        progress(Progress::new(Stage::Discovering, 0, 0, path.display().to_string()));
        let mut packages: Vec<PathBuf> = Vec::new();
        let mut setups: Vec<PathBuf> = Vec::new();
        let entries = std::fs::read_dir(path).map_err(|e| Error::io(path, e))?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(path, e))?;
            let child = entry.path();
            if !child.is_file() {
                continue;
            }
            match extension(&child).as_str() {
                "mshc" | "chm" => packages.push(child),
                "msha" | "mshi" => setups.push(child),
                _ => {}
            }
        }
        // Setup/`.mshi` documents can point at packages elsewhere in the folder.
        for setup in &setups {
            match setup_package_paths(setup) {
                Ok(referenced) => {
                    for candidate in referenced {
                        if candidate.is_file() && !packages.contains(&candidate) {
                            packages.push(candidate);
                        }
                    }
                }
                Err(err) => warnings.push(format!("{}: {err}", setup.display())),
            }
        }
        packages.sort();
        packages.dedup();
        if packages.is_empty() {
            return Err(Error::NotFound(format!(
                "no .mshc packages or .chm archives in {}",
                path.display()
            )));
        }
        let kind = if setups.is_empty() {
            SourceKind::Folder
        } else {
            SourceKind::Setup
        };
        Ok((kind, packages))
    } else {
        match extension(path).as_str() {
            "mshc" => Ok((SourceKind::Package, vec![path.to_path_buf()])),
            "chm" => Ok((SourceKind::Chm, vec![path.to_path_buf()])),
            "msha" | "mshi" => {
                progress(Progress::new(Stage::Discovering, 0, 0, path.display().to_string()));
                let mut packages = setup_package_paths(path)?;
                packages.retain(|candidate| candidate.is_file());
                if packages.is_empty() {
                    return Err(Error::NotFound(format!(
                        "{} does not reference any existing .mshc package",
                        path.display()
                    )));
                }
                Ok((SourceKind::Setup, packages))
            }
            // Unknown extension: let the magic bytes decide (a rename must not break it).
            _ => {
                let kind = if crate::chm::is_chm(path) {
                    SourceKind::Chm
                } else {
                    SourceKind::Package
                };
                Ok((kind, vec![path.to_path_buf()]))
            }
        }
    }
}

/// Packages referenced by a `.msha` setup document or a `.mshi` index file.
fn setup_package_paths(path: &Path) -> Result<Vec<PathBuf>> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let text = crate::textenc::decode_bytes(&bytes, None).text;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let lower = text.to_ascii_lowercase();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut cursor = 0usize;
    while let Some(found) = lower[cursor..].find(".mshc") {
        let end = cursor + found + 5;
        // Walk back to the start of the reference (quote, '=', '>' or whitespace).
        let start = text[..end - 5]
            .rfind(['"', '\'', '=', '>', ' ', '\t', '\r', '\n', '(', ','])
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let raw = &text[start..end];
        let cleaned = zipread::strip_query_and_fragment(raw)
            .trim()
            .trim_start_matches(['/', '\\']);
        if !cleaned.is_empty() {
            let decoded = zipread::percent_decode(cleaned);
            let candidate = dir.join(decoded.replace('\\', "/"));
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
        cursor = end;
    }
    out.sort();
    Ok(out)
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_external(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    [
        "http://",
        "https://",
        "ftp://",
        "ftps://",
        "mailto:",
        "file:",
        "tel:",
        "javascript:",
        "data:",
        "//",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

/// Split `path#anchor` (query strings are dropped, `a.htm?x=1#y` keeps its anchor).
fn split_fragment(raw: &str) -> (&str, Option<&str>) {
    let (before_hash, fragment) = match raw.find('#') {
        Some(idx) => (&raw[..idx], Some(&raw[idx + 1..])),
        None => (raw, None),
    };
    let path = match before_hash.find('?') {
        Some(idx) => &before_hash[..idx],
        None => before_hash,
    };
    (path, fragment)
}

/// Pull a topic id out of an `ms-xhelp://` URL.
fn extract_id(href: &str) -> Option<String> {
    let lower = href.to_ascii_lowercase();
    if let Some(idx) = lower.find("id=") {
        let rest = &href[idx + 3..];
        let end = rest
            .find(['&', '#', ';', ' '])
            .unwrap_or(rest.len());
        let value = zipread::percent_decode(rest[..end].trim().trim_matches(['"', '\'', '(', ')']));
        if !value.is_empty() {
            return Some(value);
        }
    }
    // `ms-xhelp:///Topic.Id` and `ms-help://x/Topic.Id`
    if let Some(anchor) = href.strip_prefix('#') {
        if !anchor.is_empty() {
            return Some(anchor.to_owned());
        }
    }
    let trimmed = href
        .split(['?', '#'])
        .next()
        .unwrap_or(href)
        .trim_end_matches('/');
    let last = trimmed.rsplit(['/', '\\', ':']).next().unwrap_or("");
    if last.is_empty() || last.eq_ignore_ascii_case("ms-xhelp") || last.eq_ignore_ascii_case("ms-help")
    {
        None
    } else {
        Some(zipread::percent_decode(last))
    }
}

/// Pull the topic reference out of a CHM link.
///
/// Handles `ms-its:book.chm::/topic.htm`, `ms-itss:…`, `mk:@MSITStore:C:\dir\book.chm::/topic.htm`
/// and the `hh.exe`-style `book.chm::/topic.htm`. The book name is dropped: Lucid resolves
/// the reference against the packages of the open content set.
fn extract_chm_reference(href: &str) -> Option<String> {
    let marker = href.find("::")?;
    let rest = href[marker + 2..].trim();
    let rest = rest.trim_start_matches(['/', '\\']);
    if rest.is_empty() {
        return None;
    }
    Some(rest.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_extraction_variants() {
        assert_eq!(
            extract_id("ms-xhelp:///?Id=Guides.Install").as_deref(),
            Some("Guides.Install")
        );
        assert_eq!(
            extract_id("ms-xhelp:///?id=a%2Eb&other=1").as_deref(),
            Some("a.b")
        );
        assert_eq!(
            extract_id("ms-help://Hx/HxRuntime/Product/Topic.Id.htm").as_deref(),
            Some("Topic.Id.htm")
        );
        assert_eq!(extract_id("ms-xhelp:///?query") , None);
    }

    #[test]
    fn external_detection() {
        assert!(is_external("https://example.com"));
        assert!(is_external("mailto:x@y.z"));
        assert!(!is_external("topics/a.htm"));
    }

    #[test]
    fn fragment_splitting() {
        assert_eq!(split_fragment("a.htm#x"), ("a.htm", Some("x")));
        assert_eq!(split_fragment("a.htm?y=1#x"), ("a.htm", Some("x")));
        assert_eq!(split_fragment("#x"), ("", Some("x")));
        assert_eq!(split_fragment("a.htm"), ("a.htm", None));
    }

    #[test]
    fn missing_folder_is_an_error() {
        let mut progress = |_p: Progress| {};
        let err = DocSet::open(Path::new("/nope/nowhere"), &mut progress).unwrap_err();
        assert!(matches!(err, Error::Io { .. }));
    }

    #[test]
    fn chm_reference_extraction() {
        assert_eq!(
            extract_chm_reference("ms-its:demo.chm::/guides/install.htm").as_deref(),
            Some("guides/install.htm")
        );
        assert_eq!(
            extract_chm_reference("mk:@MSITStore:C:\\docs\\demo.chm::/index.htm#top").as_deref(),
            Some("index.htm#top")
        );
        assert_eq!(
            extract_chm_reference("book.chm::/a.htm").as_deref(),
            Some("a.htm")
        );
        assert_eq!(extract_chm_reference("ms-its:demo.chm"), None);
        assert_eq!(extract_chm_reference("plain.htm"), None);
    }

    #[test]
    fn chm_links_are_not_external() {
        assert!(!is_external("ms-its:demo.chm::/a.htm"));
        assert!(!is_external("mk:@MSITStore:C:\\x\\demo.chm::/a.htm"));
    }
}
