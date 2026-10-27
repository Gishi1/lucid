//! Structural and metadata validation (the rules in `docs/DESIGN.md` §5).
//!
//! Validation never panics on a bad package: anything that is wrong with the *content* is
//! reported in a [`ValidationReport`], and only a file that cannot be opened at all comes back
//! as an [`Error`].

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use zip::ZipArchive;

use crate::error::Error;
use crate::manifest::{self, ManifestSummary, TocNodeSummary};
use crate::meta::{self, TopicMeta};
use crate::options::is_root_parent_id;
use crate::{is_topic_part, normalize_part_name};

/// The result of [`crate::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    /// `true` when no error was found (warnings do not make a package invalid).
    pub ok: bool,
    /// Topic parts found in the package.
    pub topics: usize,
    /// Non-topic parts, excluding the manifest and the OPC parts.
    pub assets: usize,
    /// Distinct `(keyword, url)` index entries (meta tags plus manifest).
    pub keywords: usize,
    /// Problems that make the package invalid.
    pub errors: Vec<String>,
    /// Tolerable quirks: unresolved `TocParent`, missing title, missing OPC parts, …
    pub warnings: Vec<String>,
}

impl Default for ValidationReport {
    fn default() -> Self {
        ValidationReport {
            ok: true,
            topics: 0,
            assets: 0,
            keywords: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

/// Parts larger than this are counted but not inspected in detail.
const MAX_INSPECTED_PART: u64 = 64 * 1024 * 1024;

/// Open a package for reading.
///
/// `.cab` archives (VS content packages) are detected by their magic bytes and reported with an
/// actionable message instead of a confusing zip error (DESIGN §2.1).
pub(crate) fn open_zip(path: &Path) -> Result<ZipArchive<BufReader<File>>, Error> {
    let mut file = File::open(path).map_err(|error| Error::io(path, error))?;
    let mut magic = [0u8; 4];
    let read = file
        .read(&mut magic)
        .map_err(|error| Error::io(path, error))?;
    if read == 4 && &magic == b"MSCF" {
        return Err(Error::Input(format!(
            "{} is a .cab archive, not a .mshc package — extract it with cabextract or unshield \
             first",
            path.display()
        )));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| Error::io(path, error))?;
    Ok(ZipArchive::new(BufReader::new(file))?)
}

pub(crate) fn validate_package(path: &Path) -> Result<ValidationReport, Error> {
    let mut report = ValidationReport {
        ok: false,
        ..ValidationReport::default()
    };
    let mut archive = match open_zip(path) {
        Ok(archive) => archive,
        Err(error @ Error::Io { .. }) => return Err(error),
        Err(error) => {
            report.errors.push(error.to_string());
            return Ok(report);
        }
    };

    let mut parts: BTreeMap<String, String> = BTreeMap::new();
    let mut topics: Vec<(String, TopicMeta)> = Vec::new();
    let mut topic_names: Vec<String> = Vec::new();
    let mut manifests: Vec<(String, String)> = Vec::new();
    let mut index: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut assets = 0usize;
    let mut has_content_types = false;
    let mut has_rels = false;

    for position in 0..archive.len() {
        let mut entry = match archive.by_index(position) {
            Ok(entry) => entry,
            Err(error) => {
                report
                    .errors
                    .push(format!("archive entry #{position} cannot be read: {error}"));
                continue;
            }
        };
        if entry.is_dir() {
            continue;
        }
        let name = normalize_part_name(entry.name());
        if name.is_empty() {
            continue;
        }
        let key = name.to_ascii_lowercase();
        match parts.get(&key) {
            Some(previous) => report.warnings.push(format!(
                "parts {previous} and {name} differ only in case; Lucid resolves assets \
                 case-insensitively and would use {previous}"
            )),
            None => {
                parts.insert(key.clone(), name.clone());
            }
        }
        if key == "[content_types].xml" {
            has_content_types = true;
            continue;
        }
        if key == "_rels/.rels" {
            has_rels = true;
            continue;
        }

        let size = entry.size();
        if key.ends_with(".msha") {
            match read_text(&mut entry) {
                Ok(text) => manifests.push((name, text)),
                Err(error) => report
                    .errors
                    .push(format!("manifest {name} cannot be read: {error}")),
            }
            continue;
        }
        if is_topic_part(&name) {
            topic_names.push(name.clone());
            if size == 0 {
                report
                    .errors
                    .push(format!("topic {name} is empty (0 bytes)"));
                continue;
            }
            if size > MAX_INSPECTED_PART {
                report.warnings.push(format!(
                    "topic {name} is larger than {} MiB and was only counted",
                    MAX_INSPECTED_PART / (1024 * 1024)
                ));
                continue;
            }
            let text = match read_text(&mut entry) {
                Ok(text) => text,
                Err(error) => {
                    report
                        .errors
                        .push(format!("topic {name} cannot be read: {error}"));
                    continue;
                }
            };
            if text.trim().is_empty() {
                report
                    .errors
                    .push(format!("topic {name} is empty (0 bytes)"));
                continue;
            }
            let topic_meta = meta::extract_meta(&text);
            for keyword in topic_meta.keywords.iter().chain(topic_meta.f1.iter()) {
                index
                    .entry(index_key(keyword, &key))
                    .or_insert_with(|| (keyword.clone(), name.clone()));
            }
            topics.push((name, topic_meta));
            continue;
        }

        assets += 1;
        if size == 0 {
            report
                .warnings
                .push(format!("asset {name} is empty (0 bytes)"));
        }
    }

    report.topics = topic_names.len();
    report.assets = assets;

    // Manifests are read before the topic checks: a topic packed with `--allow-missing-ids`
    // carries its id in the manifest only.
    let mut manifest_urls: BTreeSet<String> = BTreeSet::new();
    let mut manifest_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut manifest_titles: Vec<(String, String, String)> = Vec::new();
    let mut manifest_nodes = 0usize;
    if manifests.len() > 1 {
        let names: Vec<&str> = manifests.iter().map(|(name, _)| name.as_str()).collect();
        report.warnings.push(format!(
            "package contains {} manifest parts ({}); all of them are read",
            manifests.len(),
            names.join(", ")
        ));
    }
    for (name, text) in &manifests {
        let Some(ManifestSummary {
            toc_nodes,
            index_entries,
        }) = manifest::read_manifest(text)
        else {
            report
                .errors
                .push(format!("manifest {name} is not well-formed XML"));
            continue;
        };
        manifest_nodes += toc_nodes.len();
        for node in &toc_nodes {
            match node
                .url
                .as_deref()
                .map(normalize_part_name)
                .filter(|url| !url.is_empty())
            {
                Some(url) => {
                    let url_key = url.to_ascii_lowercase();
                    manifest_urls.insert(url_key.clone());
                    if !node.id.trim().is_empty() {
                        manifest_ids
                            .entry(url_key.clone())
                            .or_insert_with(|| node.id.trim().to_owned());
                    }
                    if !parts.contains_key(&url_key) {
                        report.errors.push(format!(
                            "manifest {name}: TOC node \"{}\" points at \"{url}\", which is not a \
                             part of the package",
                            node_label(node)
                        ));
                    } else if !is_topic_part(&url) {
                        report.errors.push(format!(
                            "manifest {name}: TOC node \"{}\" points at \"{url}\", which is an \
                             asset, not a topic",
                            node_label(node)
                        ));
                    }
                    if !node.title.trim().is_empty() {
                        manifest_titles.push((
                            node.id.trim().to_ascii_lowercase(),
                            url_key,
                            node.title.clone(),
                        ));
                    }
                }
                None => report.warnings.push(format!(
                    "manifest {name}: TOC node \"{}\" has no Url",
                    node_label(node)
                )),
            }
        }
        for (keyword, url) in &index_entries {
            let keyword = keyword.trim();
            if keyword.is_empty() {
                report.warnings.push(format!(
                    "manifest {name}: an index entry has an empty Keyword"
                ));
                continue;
            }
            index
                .entry(index_key(
                    keyword,
                    &normalize_part_name(url).to_ascii_lowercase(),
                ))
                .or_insert_with(|| (keyword.to_owned(), url.clone()));
        }
    }

    let mut ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, topic_meta) in &topics {
        let meta_id = topic_id(topic_meta);
        let id = meta_id
            .map(str::to_owned)
            .or_else(|| manifest_ids.get(&name.to_ascii_lowercase()).cloned())
            .filter(|id| !id.is_empty());
        match id {
            Some(id) => {
                if meta_id.is_none() {
                    report.warnings.push(format!(
                        "topic {name} has no Microsoft.Help.Id meta tag; its id comes from the \
                         manifest only"
                    ));
                }
                ids.entry(id.to_ascii_lowercase())
                    .or_default()
                    .push(name.clone());
            }
            None => report.errors.push(format!(
                "topic {name} has no Microsoft.Help.Id (Help Viewer cannot link to it)"
            )),
        }
    }
    for (id, names) in &ids {
        if names.len() > 1 {
            let listing: Vec<&str> = names.iter().take(3).map(String::as_str).collect();
            let extra = names.len().saturating_sub(listing.len());
            let suffix = if extra > 0 {
                format!(" and {extra} more")
            } else {
                String::new()
            };
            report.errors.push(format!(
                "duplicate Microsoft.Help.Id \"{id}\" used by {}{suffix}",
                listing.join(", ")
            ));
        }
    }

    for (name, topic_meta) in &topics {
        if topic_meta
            .title
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            report
                .warnings
                .push(format!("topic {name} has no <title> or Title meta tag"));
        }
        if topic_meta.toc_order_raw.is_some() && topic_meta.toc_order.is_none() {
            report.warnings.push(format!(
                "topic {name} has a non-numeric Microsoft.Help.TocOrder ({})",
                topic_meta.toc_order_raw.as_deref().unwrap_or("")
            ));
        }
        if topic_meta.keywords_declared && topic_meta.keywords.is_empty() {
            report.warnings.push(format!(
                "topic {name} declares Microsoft.Help.Keywords with no usable keyword"
            ));
        }
        if topic_meta.f1_declared && topic_meta.f1.is_empty() {
            report.warnings.push(format!(
                "topic {name} declares Microsoft.Help.F1 with no usable keyword"
            ));
        }
        let parent = topic_meta.toc_parent.as_deref().unwrap_or("").trim();
        if !is_root_parent_id("-1", parent) && !ids.contains_key(&parent.to_ascii_lowercase()) {
            report.warnings.push(format!(
                "topic {name} has an unresolvable Microsoft.Help.TocParent (\"{parent}\"); Lucid \
                 shows it as a TOC root"
            ));
        }
    }

    if !manifests.is_empty() {
        if manifest_nodes == 0 {
            let names: Vec<&str> = manifests.iter().map(|(name, _)| name.as_str()).collect();
            report.errors.push(format!(
                "manifest {} contains no HelpTOCNode entries (meta tags and manifest disagree)",
                names.join(", ")
            ));
        } else {
            for name in &topic_names {
                if !manifest_urls.contains(&name.to_ascii_lowercase()) {
                    report.errors.push(format!(
                        "meta tags and manifest disagree: topic {name} is not listed in the \
                         manifest TOC"
                    ));
                }
            }
        }
        for (id, url, title) in &manifest_titles {
            let matching = topics.iter().find(|(name, topic_meta)| {
                (!url.is_empty() && name.to_ascii_lowercase() == *url)
                    || (!id.is_empty()
                        && topic_id(topic_meta).map(str::to_ascii_lowercase).as_deref()
                            == Some(id.as_str()))
            });
            if let Some((name, topic_meta)) = matching {
                let topic_title = topic_meta.title.as_deref().unwrap_or("").trim();
                if !topic_title.is_empty() && topic_title != title.trim() {
                    report.warnings.push(format!(
                        "manifest title \"{title}\" for {name} differs from the topic title \
                         \"{topic_title}\""
                    ));
                }
            }
        }
    }

    if !has_content_types {
        report.warnings.push(
            "package has no [Content_Types].xml (Lucid tolerates that, Help Viewer does not)"
                .to_owned(),
        );
    }
    if !has_rels && !manifests.is_empty() {
        report.warnings.push(
            "package has a manifest but no _rels/.rels; the manifest was found by its extension"
                .to_owned(),
        );
    }
    if topic_names.is_empty() {
        report
            .errors
            .push("package contains no topic parts (.htm/.html/.xhtml)".to_owned());
    }
    report.keywords = index.len();
    if report.keywords == 0 {
        report
            .warnings
            .push("package contains no index keywords".to_owned());
    }
    report.ok = report.errors.is_empty();
    Ok(report)
}

fn topic_id(topic_meta: &TopicMeta) -> Option<&str> {
    topic_meta
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

fn node_label(node: &TocNodeSummary) -> String {
    if !node.title.trim().is_empty() {
        node.title.trim().to_owned()
    } else if !node.id.trim().is_empty() {
        node.id.trim().to_owned()
    } else {
        "(unnamed node)".to_owned()
    }
}

fn index_key(keyword: &str, url_key: &str) -> String {
    format!(
        "{}\u{1}{}",
        keyword.to_lowercase(),
        url_key.to_ascii_lowercase()
    )
}

fn read_text<R: Read>(reader: &mut R) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(meta::decode_text(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_report_is_ok() {
        let report = ValidationReport::default();
        assert!(report.ok);
        assert_eq!(report.topics, 0);
    }

    #[test]
    fn index_keys_are_case_insensitive() {
        assert_eq!(
            index_key("Alpha", "topics/A.htm"),
            index_key("alpha", "topics/a.htm")
        );
        assert_eq!(node_label(&TocNodeSummary::default()), "(unnamed node)");
    }
}
