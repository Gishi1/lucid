//! CHM (Microsoft Compiled HTML Help, *Help 1.x*) support.
//!
//! A `.chm` is an ITSF container: an ITSF/ITSP header pair, a PMGL/PMGI directory tree,
//! and either uncompressed parts or one LZX-compressed `MSCompressed` section. The
//! vendored reader in [`vendor`] does the container work (see `vendor/mod.rs` for the
//! provenance note); this module adapts it to Lucid's part-store model and reconstructs
//! what the viewer needs:
//!
//! * a [`ChmStore`] that presents CHM entries with the same [`Entry`] shape as [`crate::zipread::ZipStore`],
//!   so [`crate::package::Package`] does not care which container it reads;
//! * `#SYSTEM` metadata (title, default topic, LCID) — the CHM contract documented in the
//!   [HTML Help specification](https://www.nongnu.org/chmspec/latest/Internal.html);
//! * the HHC/HHK sitemaps, converted into the shape-agnostic [`crate::manifest::RawManifest`]
//!   that the manifest TOC/index builders already understand.
//!
//! Nothing here is Windows-specific and nothing is written: Lucid reads CHM files, it does
//! not compile them.

pub mod vendor;

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::html::{Event, Lexer};
use crate::manifest::{RawManifest, RawTocNode};
use crate::textenc;
use crate::zipread::{self, Entry};

use vendor::{ChmError, ChmFile, EntryCategory, EntryKind, EntrySel};

/// Magic bytes at the start of every CHM file (`ITSF`).
const ITSF_MAGIC: &[u8; 4] = b"ITSF";

/// Path of the CHM system metadata part.
const SYSTEM_PATH: &str = "/#SYSTEM";

/// Is `path` a CHM archive? The extension is the cheap answer; the `ITSF` magic is the
/// authoritative one, so a misnamed file still opens (and a `.chm` file that is really a
/// zip gets the right error).
pub fn is_chm(path: &Path) -> bool {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if extension == "chm" {
        return true;
    }
    let mut magic = [0u8; 4];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .map(|()| &magic == ITSF_MAGIC)
        .unwrap_or(false)
}

/// The parts of the `#SYSTEM` metadata Lucid uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChmSystem {
    /// `Title=` from the project file (code 3).
    pub title: Option<String>,
    /// Contents file (code 0), usually a `.hhc`.
    pub contents_file: Option<String>,
    /// Index file (code 1), usually a `.hhk`.
    pub index_file: Option<String>,
    /// `Default topic=` from the project file (code 2).
    pub default_topic: Option<String>,
    /// Locale derived from the LCID in the code 4 structure.
    pub locale: Option<String>,
    /// Compiler version string (code 9); useful for `lucid info` diagnostics only.
    pub compiled_with: Option<String>,
}

/// Parse the `#SYSTEM` part (CHM spec §5.3.3): a `DWORD` version followed by
/// `(u16 code, u16 length, bytes)` records.
pub fn parse_system(bytes: &[u8]) -> ChmSystem {
    let mut system = ChmSystem::default();
    if bytes.len() < 8 {
        return system;
    }
    let mut pos = 4usize; // version DWORD
    while pos + 4 <= bytes.len() {
        let code = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
        let length = u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        pos += 4;
        if pos + length > bytes.len() {
            break;
        }
        let data = &bytes[pos..pos + length];
        pos += length;
        match code {
            0 => system.contents_file = cstring(data),
            1 => system.index_file = cstring(data),
            2 => system.default_topic = cstring(data),
            3 => system.title = cstring(data),
            4 if data.len() >= 4 => {
                let lcid = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                system.locale = lcid_to_locale(lcid);
            }
            9 => system.compiled_with = cstring(data),
            _ => {}
        }
    }
    system
}

/// NUL-terminated, whitespace-trimmed string from a `#SYSTEM` record.
fn cstring(data: &[u8]) -> Option<String> {
    let end = data.iter().position(|byte| *byte == 0).unwrap_or(data.len());
    let text = textenc::decode_bytes(&data[..end], None).text;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Map a Windows LCID to a BCP-47 tag, or `None` when the language is unknown.
///
/// Only the locales that appear in real help files matter here; the viewer uses it for
/// duplicate-id resolution and for the `lang` attribute of exported pages, so falling back
/// to `None` is harmless.
pub fn lcid_to_locale(lcid: u32) -> Option<String> {
    let tag = match lcid & 0xffff {
        0x0401 => "ar-sa",
        0x0403 => "ca-es",
        0x0405 => "cs-cz",
        0x0406 => "da-dk",
        0x0407 => "de-de",
        0x0807 => "de-ch",
        0x0c07 => "de-at",
        0x0408 => "el-gr",
        0x0409 => "en-us",
        0x0809 => "en-gb",
        0x0c09 => "en-au",
        0x1009 => "en-ca",
        0x040a => "es-es",
        0x080a => "es-mx",
        0x040b => "fi-fi",
        0x040c => "fr-fr",
        0x080c => "fr-be",
        0x0c0c => "fr-ca",
        0x100c => "fr-ch",
        0x040d => "he-il",
        0x040e => "hu-hu",
        0x040f => "is-is",
        0x0410 => "it-it",
        0x0810 => "it-ch",
        0x0411 => "ja-jp",
        0x0412 => "ko-kr",
        0x0413 => "nl-nl",
        0x0813 => "nl-be",
        0x0414 => "nb-no",
        0x0814 => "nn-no",
        0x0415 => "pl-pl",
        0x0416 => "pt-br",
        0x0816 => "pt-pt",
        0x0418 => "ro-ro",
        0x0419 => "ru-ru",
        0x041a => "hr-hr",
        0x041b => "sk-sk",
        0x041c => "sq-al",
        0x041d => "sv-se",
        0x041e => "th-th",
        0x041f => "tr-tr",
        0x0421 => "id-id",
        0x0422 => "uk-ua",
        0x0424 => "sl-si",
        0x0425 => "et-ee",
        0x0426 => "lv-lv",
        0x0427 => "lt-lt",
        0x042a => "vi-vn",
        0x042d => "eu-es",
        0x0804 => "zh-cn",
        0x0404 => "zh-tw",
        0x0c04 => "zh-hk",
        _ => {
            // Unknown sublanguage: degrade to the primary language.
            let primary = match lcid & 0x3ff {
                0x01 => "ar",
                0x03 => "ca",
                0x04 => "zh",
                0x05 => "cs",
                0x06 => "da",
                0x07 => "de",
                0x08 => "el",
                0x09 => "en",
                0x0a => "es",
                0x0b => "fi",
                0x0c => "fr",
                0x0d => "he",
                0x0e => "hu",
                0x0f => "is",
                0x10 => "it",
                0x11 => "ja",
                0x12 => "ko",
                0x13 => "nl",
                0x14 => "no",
                0x15 => "pl",
                0x16 => "pt",
                0x18 => "ro",
                0x19 => "ru",
                0x1a => "hr",
                0x1b => "sk",
                0x1c => "sq",
                0x1d => "sv",
                0x1e => "th",
                0x1f => "tr",
                0x21 => "id",
                0x22 => "uk",
                0x24 => "sl",
                0x25 => "et",
                0x26 => "lv",
                0x27 => "lt",
                0x2a => "vi",
                0x2d => "eu",
                _ => return None,
            };
            return Some(primary.to_owned());
        }
    };
    Some(tag.to_owned())
}

/// One HHC/HHK sitemap, converted to Lucid's manifest shape.
#[derive(Debug, Clone, Default)]
pub struct SiteMap {
    /// Contents nodes (`Name` + optional `Local`).
    pub toc: Vec<RawTocNode>,
    /// Index entries (every node with both a `Name` and a `Local`).
    pub index: Vec<(String, String)>,
}

/// Parse an HHC or HHK sitemap (HTML with `text/sitemap` objects).
///
/// The format is deliberately forgiving: `<UL>` nesting defines the tree, every
/// `text/sitemap` object becomes a node, `Name`/`Local` params are read case-insensitively,
/// and malformed nesting is repaired rather than rejected.
pub fn parse_sitemap(text: &str) -> SiteMap {
    let mut levels: Vec<Vec<RawTocNode>> = vec![Vec::new()];
    let mut pending: Option<RawTocNode> = None;
    let mut in_sitemap_object = false;

    let mut lexer = Lexer::new(text);
    while let Some(event) = lexer.next_event() {
        match event {
            Event::Start { name, attrs, .. } => match name.as_str() {
                "ul" => levels.push(Vec::new()),
                "object" => {
                    let kind = Event::attr(&attrs, "type").unwrap_or("");
                    if kind.eq_ignore_ascii_case("text/sitemap") {
                        in_sitemap_object = true;
                        pending = Some(RawTocNode::default());
                    }
                }
                "param" if in_sitemap_object => {
                    if let Some(node) = pending.as_mut() {
                        let key = Event::attr(&attrs, "name").unwrap_or("");
                        let value = Event::attr(&attrs, "value").unwrap_or("");
                        let value = value.trim();
                        if value.is_empty() {
                            continue;
                        }
                        match key.to_ascii_lowercase().as_str() {
                            "name" | "title" => {
                                if node.title.is_none() {
                                    node.title = Some(value.to_owned());
                                }
                            }
                            "local" | "url" | "href" => {
                                if node.url.is_none() {
                                    node.url = Some(value.to_owned());
                                }
                            }
                            "id" | "idref" => node.id = Some(value.to_owned()),
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Event::End { name } => match name.as_str() {
                "object" if in_sitemap_object => {
                    in_sitemap_object = false;
                    let Some(node) = pending.take() else {
                        continue;
                    };
                    if node.title.is_none() && node.url.is_none() {
                        continue;
                    }
                    // An `<LI>` may hold several objects; only the first has a title.
                    let title = node.title.clone().or_else(|| {
                        node.url
                            .as_deref()
                            .map(|url| url.rsplit('/').next().unwrap_or(url).to_owned())
                    });
                    levels.last_mut().unwrap().push(RawTocNode {
                        title,
                        ..node
                    });
                }
                "ul" => close_level(&mut levels),
                _ => {}
            },
            Event::Text(_) | Event::Meta => {}
        }
    }
    while levels.len() > 1 {
        close_level(&mut levels);
    }

    let toc = levels.pop().unwrap_or_default();
    let mut index = Vec::new();
    flatten_index(&toc, &mut index);
    SiteMap { toc, index }
}

/// Close one `<UL>` level, attaching its nodes to the item on the level above it.
fn close_level(levels: &mut Vec<Vec<RawTocNode>>) {
    if levels.len() < 2 {
        return;
    }
    let children = levels.pop().unwrap_or_default();
    let Some(parent_level) = levels.last_mut() else {
        return;
    };
    match parent_level.last_mut() {
        Some(parent) => parent.children.extend(children),
        // A list before any item (or after a repaired close): keep the nodes at this level.
        None => parent_level.extend(children),
    }
}

/// Flatten a sitemap tree into `(keyword, target)` pairs, keeping the authored order.
fn flatten_index(nodes: &[RawTocNode], out: &mut Vec<(String, String)>) {
    for node in nodes {
        if let (Some(title), Some(url)) = (node.title.as_ref(), node.url.as_ref()) {
            if !title.trim().is_empty() && !url.trim().is_empty() {
                out.push((title.clone(), url.clone()));
            }
        }
        flatten_index(&node.children, out);
    }
}

/// A CHM archive presented as a part store.
///
/// Entry paths are exposed without the leading `/` and with `/` separators, so a CHM
/// behaves like a zip for topic and asset lookup. Only ordinary content parts are listed;
/// the `/#…` and `::DataSpace/…` internals are read through [`ChmStore::read_path`] when
/// needed (the `#SYSTEM` metadata is parsed at open time).
pub struct ChmStore {
    path: PathBuf,
    entries: Vec<Entry>,
    /// The upstream entries, aligned with `entries`.
    raw: Vec<vendor::Entry>,
    by_norm: HashMap<String, usize>,
    by_base: HashMap<String, Vec<usize>>,
    archive: Mutex<ChmFile>,
    system: ChmSystem,
    manifest: Option<RawManifest>,
    warnings: Vec<String>,
}

impl std::fmt::Debug for ChmStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChmStore")
            .field("path", &self.path)
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl ChmStore {
    /// Open a CHM archive and enumerate its content parts.
    pub fn open(path: &Path) -> Result<ChmStore> {
        let mut archive = ChmFile::open(path).map_err(|err| chm_error(path, err))?;
        let system = match archive.find(SYSTEM_PATH) {
            Ok(entry) => parse_system(&read_part(&mut archive, &entry, path)?),
            Err(ChmError::NotFound(_)) => ChmSystem::default(),
            Err(err) => return Err(chm_error(path, err)),
        };

        let all = archive
            .entries(EntrySel::ALL)
            .map_err(|err| chm_error(path, err))?;
        let mut entries: Vec<Entry> = Vec::new();
        let mut raw: Vec<vendor::Entry> = Vec::new();
        let mut by_norm: HashMap<String, usize> = HashMap::new();
        let mut by_base: HashMap<String, Vec<usize>> = HashMap::new();

        for item in all {
            if item.kind != EntryKind::File || item.category != EntryCategory::Normal {
                continue;
            }
            let name = item
                .path
                .trim_start_matches(['/', '\\'])
                .replace('\\', "/");
            if name.is_empty() {
                continue;
            }
            let norm = zipread::normalize_path(&name);
            if norm.is_empty() || by_norm.contains_key(&norm) {
                continue;
            }
            let index = entries.len();
            by_norm.insert(norm.clone(), index);
            let base = norm.rsplit('/').next().unwrap_or(norm.as_str()).to_owned();
            by_base.entry(base).or_default().push(index);
            entries.push(Entry {
                index,
                name,
                norm,
                is_dir: false,
                size: item.length,
                // CHM does not expose a per-part compressed size; the whole compressed
                // section is accounted for by the archive layout, not per entry.
                compressed_size: item.length,
                crc32: 0,
            });
            raw.push(item);
        }

        let mut warnings = Vec::new();
        let manifest = build_manifest(&mut archive, path, &system, &mut warnings)?;

        Ok(ChmStore {
            path: path.to_path_buf(),
            entries,
            raw,
            by_norm,
            by_base,
            archive: Mutex::new(archive),
            system,
            manifest,
            warnings,
        })
    }

    /// Path of the archive.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `#SYSTEM` metadata.
    pub fn system(&self) -> &ChmSystem {
        &self.system
    }

    /// The HHC/HHK sitemap as a manifest, when the archive has one.
    pub fn manifest(&self) -> Option<&RawManifest> {
        self.manifest.as_ref()
    }

    /// Non-fatal problems found while reading (missing or malformed sitemaps).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Content parts, in directory order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Look up by an already normalized path.
    pub fn find_norm(&self, norm: &str) -> Option<&Entry> {
        self.by_norm.get(norm).map(|index| &self.entries[*index])
    }

    /// Look up by a raw path reference from HTML, with the same tolerance as a zip store
    /// (case, `\`, `%20`, leading `/`, `./`, `../`, and a trailing fragment/query).
    pub fn find_reference(&self, raw: &str) -> Option<&Entry> {
        let cleaned = zipread::strip_query_and_fragment(raw).trim();
        if cleaned.is_empty() {
            return None;
        }
        let norm = zipread::normalize_path(cleaned);
        if let Some(entry) = self.by_norm.get(&norm) {
            return Some(&self.entries[*entry]);
        }
        let base = norm.rsplit('/').next().unwrap_or(norm.as_str());
        if let Some(candidates) = self.by_base.get(base) {
            if candidates.len() == 1 {
                return Some(&self.entries[candidates[0]]);
            }
        }
        let suffix = format!("/{norm}");
        let mut hits = self.entries.iter().filter(|entry| entry.norm.ends_with(&suffix));
        if let Some(first) = hits.next() {
            if hits.next().is_none() {
                return Some(first);
            }
        }
        None
    }

    /// Read a content part by its index in [`ChmStore::entries`].
    pub fn read(&self, entry_index: usize) -> Result<Vec<u8>> {
        let item = self
            .raw
            .get(entry_index)
            .ok_or_else(|| Error::NotFound(format!("CHM entry #{entry_index}")))?;
        let mut archive = self
            .archive
            .lock()
            .map_err(|_| Error::other("CHM reader lock poisoned"))?;
        read_part(&mut archive, item, &self.path)
    }

    /// Read any part by CHM path (used for `#SYSTEM`, `#STRINGS`, …).
    pub fn read_path(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let mut archive = self
            .archive
            .lock()
            .map_err(|_| Error::other("CHM reader lock poisoned"))?;
        match find_part(&mut archive, path) {
            Some(entry) => read_part(&mut archive, &entry, &self.path).map(Some),
            None => Ok(None),
        }
    }
}

/// Build the manifest from the HHC/HHK files referenced by `#SYSTEM`.
fn build_manifest(
    archive: &mut ChmFile,
    path: &Path,
    system: &ChmSystem,
    warnings: &mut Vec<String>,
) -> Result<Option<RawManifest>> {
    let mut manifest = RawManifest {
        part: "#SYSTEM".to_owned(),
        title: system.title.clone(),
        locale: system.locale.clone(),
        toc: Vec::new(),
        index: Vec::new(),
    };

    if let Some(contents) = system.contents_file.as_deref() {
        match find_part(archive, contents) {
            Some(entry) => {
                let text = textenc::decode_bytes(&read_part(archive, &entry, path)?, None).text;
                let sitemap = parse_sitemap(&text);
                manifest.toc = sitemap.toc;
                manifest.part = entry.path.clone();
                if manifest.toc.is_empty() {
                    warnings.push(format!("contents file `{contents}` has no sitemap entries"));
                }
            }
            None => warnings.push(format!("contents file `{contents}` not found in the archive")),
        }
    }

    if let Some(index) = system.index_file.as_deref() {
        match find_part(archive, index) {
            Some(entry) => {
                let text = textenc::decode_bytes(&read_part(archive, &entry, path)?, None).text;
                manifest.index = parse_sitemap(&text).index;
                if manifest.index.is_empty() {
                    warnings.push(format!("index file `{index}` has no keyword entries"));
                }
            }
            None => warnings.push(format!("index file `{index}` not found in the archive")),
        }
    }

    if manifest.title.is_none()
        && manifest.locale.is_none()
        && manifest.toc.is_empty()
        && manifest.index.is_empty()
    {
        return Ok(None);
    }
    Ok(Some(manifest))
}

/// Find a part by an absolute or relative CHM path (case-insensitive).
///
/// Internal parts start with `#` (`/#SYSTEM`, `/#STRINGS`), so a leading `#` must not be
/// mistaken for a fragment; anywhere else a `#fragment` is stripped like in a URL.
fn find_part(archive: &mut ChmFile, reference: &str) -> Option<vendor::Entry> {
    let reference = reference.trim();
    let reference = if reference.starts_with('#') {
        reference
    } else {
        zipread::strip_query_and_fragment(reference)
    };
    let cleaned = reference.trim().replace('\\', "/");
    let trimmed = cleaned.trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    for candidate in [format!("/{trimmed}"), trimmed.to_owned()] {
        if let Ok(entry) = archive.find(&candidate) {
            return Some(entry);
        }
    }
    None
}

/// Read one upstream entry, mapping errors into Lucid's error type.
fn read_part(archive: &mut ChmFile, entry: &vendor::Entry, path: &Path) -> Result<Vec<u8>> {
    archive.read(entry).map_err(|err| chm_error(path, err))
}

/// Map a CHM error, preserving I/O errors as [`Error::Io`] (same shape as zip failures).
fn chm_error(path: &Path, err: ChmError) -> Error {
    match err {
        ChmError::Io(source) => Error::io(path, source),
        other => Error::chm(path, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_records_are_parsed() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        // code 3: title
        let title = b"Demo Help\0";
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(&(title.len() as u16).to_le_bytes());
        bytes.extend_from_slice(title);
        // code 0: contents file
        let contents = b"demo.hhc\0";
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&(contents.len() as u16).to_le_bytes());
        bytes.extend_from_slice(contents);
        // code 4: LCID 0x0409
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&28u16.to_le_bytes());
        bytes.extend_from_slice(&0x0409u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 24]);

        let system = parse_system(&bytes);
        assert_eq!(system.title.as_deref(), Some("Demo Help"));
        assert_eq!(system.contents_file.as_deref(), Some("demo.hhc"));
        assert_eq!(system.locale.as_deref(), Some("en-us"));
    }

    #[test]
    fn truncated_records_do_not_panic() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(&100u16.to_le_bytes()); // lies about the length
        bytes.extend_from_slice(b"short");
        assert!(parse_system(&bytes).title.is_none());
    }

    #[test]
    fn lcid_mapping() {
        assert_eq!(lcid_to_locale(0x0409).as_deref(), Some("en-us"));
        assert_eq!(lcid_to_locale(0x0807).as_deref(), Some("de-ch"));
        assert_eq!(lcid_to_locale(0x0407).as_deref(), Some("de-de"));
        assert_eq!(lcid_to_locale(0x0c04).as_deref(), Some("zh-hk"));
        // Unknown sublanguage: the primary language is still useful.
        assert_eq!(lcid_to_locale(0x1809).as_deref(), Some("en"));
        assert_eq!(lcid_to_locale(0x7fff), None);
    }

    #[test]
    fn sitemap_tree_and_index() {
        let text = r#"<HTML><BODY>
<UL>
  <LI> <OBJECT type="text/sitemap"><param name="Name" value="Welcome"><param name="Local" value="index.htm"></OBJECT>
  <LI> <OBJECT type="text/sitemap"><param name="Name" value="Guides"></OBJECT>
    <UL>
      <LI> <OBJECT type="text/sitemap"><param name="Name" value="Install"><param name="Local" value="guides/install.htm"></OBJECT>
    </UL>
</UL>
</BODY></HTML>"#;
        let sitemap = parse_sitemap(text);
        assert_eq!(sitemap.toc.len(), 2);
        assert_eq!(sitemap.toc[0].title.as_deref(), Some("Welcome"));
        assert_eq!(sitemap.toc[0].url.as_deref(), Some("index.htm"));
        assert_eq!(sitemap.toc[1].title.as_deref(), Some("Guides"));
        assert_eq!(sitemap.toc[1].url, None);
        assert_eq!(sitemap.toc[1].children.len(), 1);
        assert_eq!(
            sitemap.toc[1].children[0].url.as_deref(),
            Some("guides/install.htm")
        );
        assert_eq!(sitemap.index.len(), 2);
        assert_eq!(sitemap.index[1].0, "Install");
    }

    #[test]
    fn sitemap_survives_unclosed_lists() {
        let text = r#"<UL><LI><OBJECT type="text/sitemap"><param name="Name" value="A"><param name="Local" value="a.htm"></OBJECT>
        <UL><LI><OBJECT type="text/sitemap"><param name="Name" value="B"><param name="Local" value="b.htm"></OBJECT>"#;
        let sitemap = parse_sitemap(text);
        assert_eq!(sitemap.toc.len(), 1);
        assert_eq!(sitemap.toc[0].children.len(), 1);
        assert_eq!(sitemap.toc[0].children[0].title.as_deref(), Some("B"));
    }

    #[test]
    fn non_chm_files_are_not_chm() {
        assert!(!is_chm(Path::new("/nope/none.mshc")));
    }
}
