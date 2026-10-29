//! One help package: part classification, topic metadata scan, keyword index, assets and
//! TOC reconstruction. The container is a zip (`.mshc`) or a CHM archive; both are reached
//! through [`Store`], so everything in this module is format-agnostic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::error::{Error, Result};
use crate::manifest::{self, RawManifest};
use crate::meta::{self, RawMeta};
use crate::model::{self, Document};
use crate::progress::{Progress, Stage};
use crate::store::Store;
use crate::textenc;
use crate::toc::{self, MetaItem, TocNode};
use crate::zipread::{self, Entry};

/// Identifies a topic inside a content set: `(package index, topic index)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TopicKey {
    pub pkg: u32,
    pub topic: u32,
}

impl TopicKey {
    pub fn new(pkg: u32, topic: u32) -> Self {
        TopicKey { pkg, topic }
    }
}

impl std::fmt::Display for TopicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.pkg, self.topic)
    }
}

/// Metadata of one topic. The HTML itself is *not* kept (packages can hold 100k topics), it
/// is read on demand by [`Package::topic_html`] / [`Package::document`].
#[derive(Debug, Clone)]
pub struct Topic {
    pub key: TopicKey,
    /// Part path as stored in the archive.
    pub part: String,
    /// Lookup key (normalized lowercase path).
    pub norm: String,
    pub meta: RawMeta,
    pub id: String,
    pub id_lc: String,
    pub title: String,
    /// `Microsoft.Help.TocParent`, normalized.
    pub parent_lc: Option<String>,
    /// Raw parent value, for `lucid info`.
    pub parent_raw: Option<String>,
    pub order: i32,
    pub keywords: Vec<String>,
    pub f1: Vec<String>,
    pub locale: Option<String>,
    pub topic_locale: Option<String>,
    pub description: Option<String>,
    pub part_size: u64,
    /// Encoding the topic was decoded from.
    pub encoding: &'static str,
    /// Entry index inside the container (internal).
    entry: usize,
}

impl Topic {
    /// Locale used for duplicate-id resolution: topic locale wins over the generic one.
    pub fn effective_locale(&self) -> Option<&str> {
        self.topic_locale.as_deref().or(self.locale.as_deref())
    }
}

/// A non-topic part.
#[derive(Debug, Clone)]
pub struct Asset {
    pub norm: String,
    pub part: String,
    pub size: u64,
    pub mime: &'static str,
}

/// Asset content, loaded on demand.
#[derive(Debug, Clone)]
pub struct AssetBytes {
    pub norm: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// OPC (Open Packaging Conventions) bookkeeping, mostly for diagnostics.
#[derive(Debug, Clone, Default)]
pub struct OpcInfo {
    pub has_content_types: bool,
    pub has_rels: bool,
    /// Manifest-ish parts found in the package.
    pub manifest_parts: Vec<String>,
}

/// Summary numbers for `lucid info`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageStats {
    pub topics: usize,
    pub assets: usize,
    pub toc_nodes: usize,
    pub keywords: usize,
    pub bytes: u64,
    pub compressed_bytes: u64,
}

/// One help package: a zip `.mshc` or a CHM archive.
pub struct Package {
    pub path: PathBuf,
    pub name: String,
    pub title: Option<String>,
    pub locale: Option<String>,
    pub topics: Vec<Topic>,
    pub assets: Vec<Asset>,
    pub manifest: Option<RawManifest>,
    pub opc: OpcInfo,
    pub warnings: Vec<String>,

    store: Store,
    topic_by_norm: HashMap<String, u32>,
    by_id: HashMap<String, u32>,
    duplicate_ids: Vec<(String, Vec<u32>)>,
    asset_by_norm: HashMap<String, usize>,
    unpacked_bytes: u64,
    compressed_bytes: u64,
}

impl std::fmt::Debug for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Package")
            .field("path", &self.path)
            .field("topics", &self.topics.len())
            .field("assets", &self.assets.len())
            .finish()
    }
}

impl Package {
    /// Open a package and scan the metadata of every topic.
    ///
    /// The container is chosen by content: a zip (`.mshc`) or a CHM archive. Both expose
    /// the same content-part model, so everything downstream is format-agnostic.
    pub fn open(path: &Path, pkg_index: u32, progress: &mut dyn FnMut(Progress)) -> Result<Package> {
        let store = Store::open(path)?;
        progress(Progress::new(
            Stage::Opening,
            0,
            0,
            path.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ));

        let mut topics_idx: Vec<usize> = Vec::new();
        let mut assets: Vec<Asset> = Vec::new();
        let mut manifest_candidates: Vec<&Entry> = Vec::new();
        let mut opc = OpcInfo::default();
        let mut unpacked = 0u64;
        let mut compressed = 0u64;

        for entry in store.entries() {
            if entry.is_dir {
                continue;
            }
            unpacked += entry.size;
            compressed += entry.compressed_size;
            match entry.ext() {
                "htm" | "html" | "xhtml" => topics_idx.push(entry.index),
                _ => {
                    if entry.norm == "[content_types].xml" {
                        opc.has_content_types = true;
                    }
                    if entry.norm.starts_with("_rels/") || entry.norm == "_rels" {
                        opc.has_rels = true;
                    }
                    if manifest::looks_like_manifest(&entry.norm) {
                        manifest_candidates.push(entry);
                    } else {
                        assets.push(Asset {
                            norm: entry.norm.clone(),
                            part: entry.name.clone(),
                            size: entry.size,
                            mime: mime_for(entry.ext()),
                        });
                    }
                }
            }
        }

        if topics_idx.is_empty() {
            return Err(Error::NoTopics {
                path: path.to_path_buf(),
            });
        }

        // Prefer `.msha`, then other manifest-looking XML, in path order.
        manifest_candidates.sort_by(|a, b| {
            let rank = |e: &Entry| if e.ext() == "msha" { 0 } else { 1 };
            rank(a).cmp(&rank(b)).then_with(|| a.norm.cmp(&b.norm))
        });
        // A CHM carries its own manifest (the HHC/HHK sitemap from `#SYSTEM`); a zip is
        // scanned for `.msha`/XML manifest parts below.
        let mut manifest = store.manifest().cloned();
        let mut warnings: Vec<String> = store.warnings().to_vec();
        for candidate in &manifest_candidates {
            opc.manifest_parts.push(candidate.norm.clone());
            if manifest.is_some() {
                continue;
            }
            match store.read(candidate.index) {
                Ok(bytes) => {
                    let text = textenc::decode_bytes(&bytes, None).text;
                    match manifest::parse(&candidate.name, &text) {
                        Ok(parsed) if !parsed.toc.is_empty() || !parsed.index.is_empty() => {
                            manifest = Some(parsed);
                        }
                        Ok(_) => {
                            warnings.push(format!(
                                "manifest part `{}` has no TOC or index entries",
                                candidate.name
                            ));
                        }
                        Err(err) => warnings.push(format!("manifest part `{}`: {err}", candidate.name)),
                    }
                }
                Err(err) => warnings.push(format!("manifest part `{}`: {err}", candidate.name)),
            }
        }

        // Topic metadata scan (parallel: reading + parsing per topic).
        progress(Progress::new(
            Stage::Scanning,
            0,
            topics_idx.len(),
            path.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ));
        let scanned: Vec<Result<Topic>> = topics_idx
            .par_iter()
            .map(|entry_index| {
                let entry = store
                    .entries()
                    .iter()
                    .find(|e| e.index == *entry_index)
                    .ok_or_else(|| Error::other("entry vanished during scan"))?;
                scan_topic(&store, *entry_index, entry, pkg_index)
            })
            .collect();

        let mut topics: Vec<Topic> = Vec::with_capacity(scanned.len());
        for result in scanned {
            match result {
                Ok(topic) => topics.push(topic),
                Err(err) => warnings.push(err.to_string()),
            }
        }
        if topics.is_empty() {
            return Err(Error::NoTopics {
                path: path.to_path_buf(),
            });
        }
        // Stable order regardless of thread scheduling.
        topics.sort_by(|a, b| a.norm.cmp(&b.norm));
        for (idx, topic) in topics.iter_mut().enumerate() {
            topic.key = TopicKey::new(pkg_index, idx as u32);
        }

        let mut topic_by_norm = HashMap::with_capacity(topics.len());
        for topic in &topics {
            topic_by_norm.insert(topic.norm.clone(), topic.key.topic);
        }
        let mut asset_by_norm = HashMap::with_capacity(assets.len());
        for (idx, asset) in assets.iter().enumerate() {
            asset_by_norm.entry(asset.norm.clone()).or_insert(idx);
        }

        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "package".to_owned());
        let title = manifest
            .as_ref()
            .and_then(|m| m.title.clone())
            .or_else(|| store.title())
            .or_else(|| Some(name.clone()));
        let locale = manifest
            .as_ref()
            .and_then(|m| m.locale.clone())
            .or_else(|| store.locale())
            .or_else(|| most_common_locale(&topics));

        let mut package = Package {
            path: path.to_path_buf(),
            name,
            title,
            locale,
            topics,
            assets,
            manifest,
            opc,
            warnings,
            store,
            topic_by_norm,
            by_id: HashMap::new(),
            duplicate_ids: Vec::new(),
            asset_by_norm,
            unpacked_bytes: unpacked,
            compressed_bytes: compressed,
        };
        package.resolve_ids();
        Ok(package)
    }

    /// Build the id map, preferring topics whose locale matches the package locale.
    fn resolve_ids(&mut self) {
        let mut grouped: HashMap<String, Vec<u32>> = HashMap::with_capacity(self.topics.len());
        for topic in &self.topics {
            if topic.id_lc.is_empty() {
                continue;
            }
            grouped
                .entry(topic.id_lc.clone())
                .or_default()
                .push(topic.key.topic);
        }
        let package_locale = self.locale.clone().map(|l| l.to_ascii_lowercase());
        for (id, indices) in grouped {
            if indices.len() == 1 {
                self.by_id.insert(id, indices[0]);
                continue;
            }
            let preferred = package_locale
                .as_deref()
                .and_then(|locale| {
                    indices.iter().copied().find(|idx| {
                        self.topics[*idx as usize]
                            .effective_locale()
                            .is_some_and(|l| l.eq_ignore_ascii_case(locale))
                    })
                })
                .unwrap_or(indices[0]);
            self.warnings.push(format!(
                "duplicate topic id `{}` ({} topics); using package locale `{}`",
                id,
                indices.len(),
                self.topics[preferred as usize]
                    .effective_locale()
                    .unwrap_or("unknown")
            ));
            self.duplicate_ids.push((id.clone(), indices.clone()));
            self.by_id.insert(id, preferred);
        }
    }

    /// Topic by internal index.
    pub fn topic(&self, index: u32) -> Option<&Topic> {
        self.topics.get(index as usize)
    }

    /// Topic by part path reference (case/`\`/percent tolerant), as found in links.
    pub fn topic_by_reference(&self, reference: &str) -> Option<u32> {
        let cleaned = zipread::strip_query_and_fragment(reference).trim();
        if cleaned.is_empty() {
            return None;
        }
        let norm = zipread::normalize_path(cleaned);
        if let Some(idx) = self.topic_by_norm.get(&norm) {
            return Some(*idx);
        }
        // some manifests reference topics by id instead of by path
        if let Some(idx) = self.by_id.get(&norm) {
            return Some(*idx);
        }
        let base = norm.rsplit('/').next().unwrap_or(norm.as_str());
        let mut hits = self
            .topics
            .iter()
            .filter(|t| t.norm.rsplit('/').next() == Some(base));
        if let Some(first) = hits.next() {
            if hits.next().is_none() {
                return Some(first.key.topic);
            }
        }
        None
    }

    /// Exact lookup by an already normalized path (used for cross-package link resolution).
    pub fn topic_by_norm_lookup(&self, norm: &str) -> Option<u32> {
        self.topic_by_norm.get(norm).copied()
    }

    /// Topic by `Microsoft.Help.Id` (case-insensitive).
    pub fn find_id(&self, id: &str) -> Option<u32> {
        let key = id.trim().to_ascii_lowercase();
        if key.is_empty() {
            return None;
        }
        self.by_id.get(&key).copied()
    }

    /// Topic by F1 keyword.
    pub fn find_f1(&self, keyword: &str) -> Option<u32> {
        self.topics
            .iter()
            .find(|t| t.f1.iter().any(|k| k.eq_ignore_ascii_case(keyword)))
            .map(|t| t.key.topic)
    }

    /// Read and decode a topic part.
    pub fn topic_html(&self, index: u32) -> Result<String> {
        let topic = self
            .topic(index)
            .ok_or_else(|| Error::NotFound(format!("topic #{index}")))?;
        let bytes = self.store.read(topic.entry)?;
        let declared = textenc::sniff_declared_charset(&bytes, 4096);
        Ok(textenc::decode_bytes(&bytes, declared.as_deref()).text)
    }

    /// Parse a topic body into the block model (what the viewer renders).
    pub fn document(&self, index: u32) -> Result<Document> {
        let topic = self
            .topic(index)
            .ok_or_else(|| Error::NotFound(format!("topic #{index}")))?;
        let html = self.topic_html(index)?;
        let mut doc = model::parse_document(&html);
        if doc.title.is_none() {
            doc.title = Some(topic.title.clone());
        }
        match &doc.language {
            Some(_) => {}
            None => doc.language = topic.locale.clone(),
        }
        Ok(doc)
    }

    /// Look up and load an asset referenced from a topic.
    ///
    /// Fast path: an exact normalized hit in the pre-built asset map. Slow path: the
    /// tolerant reference lookup (case, `\`, `%XX`, `./`, basename fallbacks).
    pub fn asset_bytes(&self, reference: &str) -> Result<Option<AssetBytes>> {
        let cleaned = zipread::strip_query_and_fragment(reference).trim();
        let norm = zipread::normalize_path(cleaned);
        if let Some(index) = self.asset_by_norm.get(&norm) {
            let asset = &self.assets[*index];
            let entry = self.store.find_norm(&asset.norm);
            if let Some(entry) = entry {
                let bytes = self.store.read(entry.index)?;
                return Ok(Some(AssetBytes {
                    norm: asset.norm.clone(),
                    mime: asset.mime,
                    bytes,
                }));
            }
        }
        let entry = match self.store.find_reference(reference) {
            Some(entry) => entry,
            None => return Ok(None),
        };
        let bytes = self.store.read(entry.index)?;
        Ok(Some(AssetBytes {
            norm: entry.norm.clone(),
            mime: mime_for(entry.ext()),
            bytes,
        }))
    }

    /// TOC from the manifest part, if any (urls resolved to topic indices).
    pub fn manifest_toc(&self) -> Vec<TocNode<u32>> {
        let Some(manifest) = self.manifest.as_ref() else {
            return Vec::new();
        };
        toc::from_manifest(
            manifest,
            |url| self.topic_by_reference(url),
            |id| self.find_id(id),
        )
    }

    /// TOC from `Microsoft.Help.TocParent` / `TocOrder` meta tags.
    pub fn meta_toc(&self) -> Vec<TocNode<u32>> {
        let items: Vec<MetaItem<u32>> = self
            .topics
            .iter()
            .map(|topic| MetaItem {
                key: topic.key.topic,
                id: if topic.id.is_empty() {
                    topic.norm.clone()
                } else {
                    topic.id.clone()
                },
                parent: topic.parent_raw.clone(),
                order: topic.order,
                title: topic.title.clone(),
            })
            .collect();
        toc::from_meta(&items)
    }

    /// The TOC used by the viewer: manifest structure plus every topic the manifest forgot.
    pub fn toc(&self) -> Vec<TocNode<u32>> {
        let manifest_toc = self.manifest_toc();
        let meta_toc = self.meta_toc();
        if manifest_toc.is_empty() {
            return meta_toc;
        }
        let (merged, added) = toc::merge(manifest_toc, meta_toc);
        let _ = added;
        merged
    }

    /// Keyword index: manifest entries first, then `Microsoft.Help.Keywords`, then F1.
    pub fn keyword_entries(&self) -> Vec<(String, u32, bool)> {
        let mut out: Vec<(String, u32, bool)> = Vec::new();
        let mut seen: std::collections::HashSet<(String, u32)> = std::collections::HashSet::new();
        if let Some(manifest) = self.manifest.as_ref() {
            for (keyword, target) in &manifest.index {
                if let Some(idx) = self.topic_by_reference(target).or_else(|| self.find_id(target)) {
                    let key = (keyword.to_ascii_lowercase(), idx);
                    if seen.insert(key) {
                        out.push((keyword.clone(), idx, false));
                    }
                }
            }
        }
        for topic in &self.topics {
            for keyword in &topic.keywords {
                let key = (keyword.to_ascii_lowercase(), topic.key.topic);
                if seen.insert(key) {
                    out.push((keyword.clone(), topic.key.topic, false));
                }
            }
        }
        for topic in &self.topics {
            for keyword in &topic.f1 {
                let key = (keyword.to_ascii_lowercase(), topic.key.topic);
                if seen.insert(key) {
                    out.push((keyword.clone(), topic.key.topic, true));
                }
            }
        }
        out
    }

    /// Summary numbers.
    pub fn stats(&self) -> PackageStats {
        PackageStats {
            topics: self.topics.len(),
            assets: self.assets.len(),
            toc_nodes: self.toc().iter().map(|n| n.count()).sum(),
            keywords: self.keyword_entries().len(),
            bytes: self.unpacked_bytes,
            compressed_bytes: self.compressed_bytes,
        }
    }

    /// Duplicate ids detected while resolving.
    pub fn duplicate_ids(&self) -> &[(String, Vec<u32>)] {
        &self.duplicate_ids
    }

    /// Iterate topics, read + parse each body, and hand it to `f` (used to build the search
    /// index). Bodies are dropped immediately, so memory stays flat on huge packages.
    pub fn extract(
        &self,
        progress: &mut dyn FnMut(Progress),
        mut f: impl FnMut(&Topic, Document),
    ) -> (usize, Vec<String>) {
        let mut done = 0usize;
        let mut warnings = Vec::new();
        let total = self.topics.len();
        let report_every = if total > 500 { 64 } else { usize::MAX };
        for topic in &self.topics {
            match self.document(topic.key.topic) {
                Ok(doc) => f(topic, doc),
                Err(err) => warnings.push(format!("{}: {err}", topic.part)),
            }
            done += 1;
            if done % report_every == 0 {
                progress(Progress::new(
                    Stage::Extracting,
                    done,
                    total,
                    self.name.clone(),
                ));
            }
        }
        progress(Progress::new(Stage::Extracting, total, total, self.name.clone()));
        (done, warnings)
    }

    /// Raw access to the container (zip or CHM).
    pub fn store(&self) -> &Store {
        &self.store
    }
}

fn scan_topic(
    store: &Store,
    entry_index: usize,
    entry: &Entry,
    pkg_index: u32,
) -> Result<Topic> {
    let bytes = store.read(entry_index)?;
    let declared = textenc::sniff_declared_charset(&bytes, 4096);
    let decoded = textenc::decode_bytes(&bytes, declared.as_deref());
    let meta = meta::scan(&decoded.text);
    let id = meta.id.clone().unwrap_or_default();
    Ok(Topic {
        key: TopicKey::new(pkg_index, 0),
        part: entry.name.clone(),
        norm: entry.norm.clone(),
        id: id.clone(),
        id_lc: id.trim().to_ascii_lowercase(),
        title: meta.display_title(),
        parent_lc: meta
            .toc_parent
            .as_deref()
            .map(|p| p.trim().to_ascii_lowercase())
            .filter(|p| !p.is_empty()),
        parent_raw: meta.toc_parent.clone(),
        order: meta.toc_order.unwrap_or(0),
        keywords: meta.keywords.clone(),
        f1: meta.f1.clone(),
        locale: meta.locale.clone(),
        topic_locale: meta.topic_locale.clone(),
        description: meta.description.clone(),
        part_size: entry.size,
        encoding: decoded.encoding.name(),
        entry: entry_index,
        meta,
    })
}

fn most_common_locale(topics: &[Topic]) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for topic in topics {
        if let Some(locale) = topic.effective_locale() {
            *counts.entry(locale.to_ascii_lowercase()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(locale, count)| (*count, locale.clone()))
        .map(|(locale, _)| locale)
}

/// MIME type by file extension (what the viewer needs to decide how to render an asset).
pub fn mime_for(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "htm" | "html" | "xhtml" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "json" => "application/json",
        "xml" | "msha" | "mshi" | "rels" => "application/xml",
        "txt" | "log" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" | "svgz" => "image/svg+xml",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" | "m4v" => "video/mp4",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "pdf" => "application/pdf",
        "zip" | "mshc" => "application/zip",
        "cab" => "application/vnd.ms-cab-compressed",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_lookup_is_case_insensitive() {
        assert_eq!(mime_for("PNG"), "image/png");
        assert_eq!(mime_for("BMP"), "image/bmp");
        assert_eq!(mime_for("unknown"), "application/octet-stream");
    }

    #[test]
    fn missing_package_is_an_io_error() {
        let mut progress = |_p: Progress| {};
        let err = Package::open(Path::new("/nope/none.mshc"), 0, &mut progress).unwrap_err();
        assert!(matches!(err, Error::Io { .. } | Error::NotPackage { .. }));
    }

    #[test]
    fn topic_key_displays_readably() {
        assert_eq!(TopicKey::new(1, 2).to_string(), "1#2");
    }
}
