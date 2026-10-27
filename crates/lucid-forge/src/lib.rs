#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! MSHC packer: build valid `.mshc` packages from a folder of topic HTML.
//!
//! Lucid's packer is a library plus a small CLI. The frozen contract — command line, package
//! layout, `.msha` schema, validation rules — is `docs/DESIGN.md` §5; this crate is its
//! implementation.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use lucid_forge::{build_package, BuildOptions};
//!
//! let report = build_package(
//!     std::path::Path::new("docs-src"),
//!     std::path::Path::new("docs.mshc"),
//!     &BuildOptions {
//!         title: Some("My Product".to_owned()),
//!         ..BuildOptions::default()
//!     },
//! )?;
//! println!("{} topics, {} bytes", report.topics, report.bytes);
//! # Ok(())
//! # }
//! ```
//!
//! What a build writes, in this order:
//!
//! 1. the manifest (`<name>.msha`) when `write_manifest` is set,
//! 2. `[Content_Types].xml` and `_rels/.rels` when `write_opc` is set,
//! 3. every topic, sorted by path,
//! 4. every asset, sorted by path.
//!
//! Entry order and entry timestamps are fixed, so two builds of the same input are
//! byte-identical (`preserve_times` opts into the source files' timestamps instead).

mod bmp;
mod demo;
mod error;
mod manifest;
mod meta;
mod opc;
mod options;
mod validate;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use walkdir::WalkDir;

pub use crate::bmp::{encode_rgb24, logo};
pub use crate::demo::{demo_files, demo_options};
pub use crate::error::Error;
pub use crate::manifest::{
    escape_xml, read_manifest, toc_from_topics, write_manifest, ManifestSpec, ManifestSummary,
    TocNode, TocNodeSummary, TopicRef, HELP_NAMESPACE,
};
pub use crate::meta::{
    cp1252_byte, decode_cp1252, decode_entities, decode_text, encode_cp1252, extract_meta,
    extract_meta_bytes, parse_order, slugify, split_list, TopicMeta,
};
pub use crate::opc::{content_types_xml, mime_for_extension, package_rels_xml};
pub use crate::options::{is_root_parent_id, BuildOptions, Compression};
pub use crate::validate::ValidationReport;

/// What a `build` or `demo` run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
    /// The package that was written.
    pub out: PathBuf,
    /// Size of the package in bytes.
    pub bytes: u64,
    /// Number of topic parts (`.htm`/`.html`/`.xhtml`) in the package.
    pub topics: usize,
    /// Number of non-topic parts copied into the package.
    pub assets: usize,
    /// Number of TOC nodes written to the manifest (one per topic).
    pub toc_nodes: usize,
    /// Number of `HelpIndexEntry` entries written (keywords and F1 keywords, deduplicated).
    pub keywords: usize,
    /// Non-fatal problems: derived ids, garbage `TocOrder` values, unresolvable parents, …
    pub warnings: Vec<String>,
}

/// The timestamp every entry gets unless [`BuildOptions::preserve_times`] is set.
const FIXED_TIMESTAMP: (u16, u8, u8, u8, u8, u8) = (2020, 1, 1, 0, 0, 0);

/// Assemble a package from a directory of topic HTML and assets.
///
/// The input tree is copied verbatim (relative paths preserved); a topic is any `.htm`,
/// `.html` or `.xhtml` file, everything else is an asset. `[Content_Types].xml`,
/// `_rels/.rels` and `*.msha` files in the input are skipped with a warning, because the
/// packer generates those parts itself.
///
/// # Errors
///
/// [`Error::MissingId`] when a topic has no `Microsoft.Help.Id` and
/// [`BuildOptions::allow_missing_ids`] is off, [`Error::NoTopics`] when the directory contains
/// no topic at all, [`Error::Io`]/[`Error::Zip`] for filesystem and archive failures.
pub fn build_package(
    input_dir: &Path,
    out: &Path,
    opts: &BuildOptions,
) -> Result<BuildReport, Error> {
    let metadata = fs::metadata(input_dir).map_err(|error| Error::io(input_dir, error))?;
    if !metadata.is_dir() {
        return Err(Error::Input(format!(
            "{} is not a directory",
            input_dir.display()
        )));
    }

    let mut warnings: Vec<String> = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in WalkDir::new(input_dir).follow_links(false) {
        let entry = entry.map_err(|error| {
            Error::Input(format!("cannot walk {}: {error}", input_dir.display()))
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(input_dir).unwrap_or(entry.path());
        let Some(name) = relative.to_str().map(normalize_part_name) else {
            warnings.push(format!(
                "skipped {}: the file name is not valid UTF-8",
                entry.path().display()
            ));
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if is_reserved_part(&name) {
            warnings.push(format!(
                "skipped reserved part {name}: lucid-forge generates it"
            ));
            continue;
        }
        files.push((name, entry.path().to_path_buf()));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (name, _) in &files {
        let key = name.to_ascii_lowercase();
        if let Some(previous) = seen.get(&key) {
            warnings.push(format!(
                "parts {previous} and {name} differ only in case; both are stored (a zip is \
                 case-sensitive, Lucid is not)"
            ));
        } else {
            seen.insert(key, name.clone());
        }
    }

    let mut parts: Vec<Part> = Vec::with_capacity(files.len());
    for (name, path) in files {
        let bytes = fs::read(&path).map_err(|error| Error::io(&path, error))?;
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok();
        parts.push(Part {
            name,
            bytes,
            modified,
        });
    }

    assemble(out, parts, opts, warnings, &input_dir.display().to_string())
}

/// Write the built-in demo package (the self-authored book described in `docs/DESIGN.md` §5).
///
/// The output does not depend on `out`, so every build of the demo produces the same bytes.
/// An existing file is overwritten; `lucid-forge demo` refuses to do that without `--force`.
pub fn demo_package(out: &Path) -> Result<BuildReport, Error> {
    let parts: Vec<Part> = demo_files()
        .into_iter()
        .map(|(name, bytes)| Part {
            name,
            bytes,
            modified: None,
        })
        .collect();
    assemble(out, parts, &demo_options(), Vec::new(), "the built-in demo")
}

/// Check a package against the rules in `docs/DESIGN.md` §5.
///
/// Returns a report instead of failing on bad content: dangling manifest URLs, duplicate ids,
/// zero-byte topics, missing ids/titles, empty keywords and manifest-versus-meta TOC
/// mismatches all come back as entries in [`ValidationReport::errors`] or
/// [`ValidationReport::warnings`]. Only a file that cannot be opened at all is an [`Error`].
pub fn validate(path: &Path) -> Result<ValidationReport, Error> {
    validate::validate_package(path)
}

/// Print the TOC derived from the topic meta tags (`Microsoft.Help.TocParent` /
/// `Microsoft.Help.TocOrder`), one line per node:
///
/// ```text
/// TOC (19 topics, 4 roots)
/// Introduction to Lucid  [Lucid.Intro]
/// Guides  [Lucid.Guides]
///   Getting started  [Lucid.Guides.GettingStarted]
/// ```
///
/// `-1` and an absent `TocParent` mean "TOC root", exactly as with
/// [`BuildOptions::root_id`]'s default.
pub fn outline(path: &Path) -> Result<String, Error> {
    let mut archive = validate::open_zip(path)?;
    let mut topics: Vec<(String, Vec<u8>)> = Vec::new();
    for position in 0..archive.len() {
        let mut entry = archive.by_index(position)?;
        if entry.is_dir() {
            continue;
        }
        let name = normalize_part_name(entry.name());
        if !is_topic_part(&name) {
            continue;
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)
            .map_err(|error| Error::io(path, error))?;
        topics.push((name, bytes));
    }
    topics.sort_by(|left, right| left.0.cmp(&right.0));

    let refs: Vec<TopicRef> = topics
        .iter()
        .map(|(name, bytes)| topic_ref_from_meta(name, &extract_meta_bytes(bytes)))
        .collect();
    let (roots, _warnings) = toc_from_topics(&refs, "-1");
    Ok(format_outline(&roots, refs.len()))
}

/// Normalize a zip part name: `\` becomes `/`, leading `./` and `/` are stripped.
pub fn normalize_part_name(name: &str) -> String {
    let mut name = name.replace('\\', "/");
    loop {
        let stripped = name
            .strip_prefix("./")
            .or_else(|| name.strip_prefix('/'))
            .map(str::to_owned);
        match stripped {
            Some(rest) => name = rest,
            None => break,
        }
    }
    name
}

/// Is `name` a topic part? (`.htm`, `.html` or `.xhtml`, case-insensitively.)
pub fn is_topic_part(name: &str) -> bool {
    matches!(extension_of(name).as_str(), "htm" | "html" | "xhtml")
}

/// The lowercase extension of a part name, without the dot.
pub fn extension_of(name: &str) -> String {
    name.rsplit('/')
        .next()
        .unwrap_or(name)
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Parts the packer generates itself and therefore refuses to copy from the input tree.
fn is_reserved_part(name: &str) -> bool {
    let key = name.to_ascii_lowercase();
    key == "[content_types].xml" || key == "_rels/.rels" || key.ends_with(".msha")
}

/// One part of the package being written.
struct Part {
    name: String,
    bytes: Vec<u8>,
    /// The source file's modification time; only used when `preserve_times` is set.
    modified: Option<SystemTime>,
}

impl Part {
    /// A part the packer generated (fixed timestamp).
    fn generated(name: impl Into<String>, bytes: Vec<u8>) -> Part {
        Part {
            name: name.into(),
            bytes,
            modified: None,
        }
    }
}

/// Turn parts (topics and assets) into a package: scan topics, synthesize the TOC and the
/// index, write the manifest and the OPC parts, then the tree itself.
fn assemble(
    out: &Path,
    input: Vec<Part>,
    opts: &BuildOptions,
    mut warnings: Vec<String>,
    origin: &str,
) -> Result<BuildReport, Error> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for part in &input {
        if !names.insert(part.name.clone()) {
            return Err(Error::Input(format!(
                "part {} would be written twice",
                part.name
            )));
        }
    }

    let mut topics: Vec<(Part, TopicRef)> = Vec::new();
    let mut assets: Vec<Part> = Vec::new();
    let mut index: BTreeMap<String, (String, String)> = BTreeMap::new();
    for part in input {
        if !is_topic_part(&part.name) {
            assets.push(part);
            continue;
        }
        let topic_meta = extract_meta_bytes(&part.bytes);
        let topic = topic_ref(&part.name, &topic_meta, opts, &mut warnings)?;
        if topic_meta.keywords_declared && topic_meta.keywords.is_empty() {
            warnings.push(format!(
                "topic {} declares Microsoft.Help.Keywords with no usable keyword",
                part.name
            ));
        }
        for keyword in topic_meta.keywords.iter().chain(topic_meta.f1.iter()) {
            index
                .entry(index_key(keyword, &part.name))
                .or_insert_with(|| (keyword.clone(), part.name.clone()));
        }
        topics.push((part, topic));
    }
    if topics.is_empty() {
        return Err(Error::NoTopics(origin.to_owned()));
    }

    let refs: Vec<TopicRef> = topics.iter().map(|(_, topic)| topic.clone()).collect();
    let (toc, toc_warnings) = toc_from_topics(&refs, &opts.root_id);
    warnings.extend(toc_warnings);
    let toc_nodes = count_nodes(&toc);
    let index: Vec<(String, String)> = index.into_values().collect();

    let title = package_title(opts, out);
    let manifest_name = manifest_name(opts, out);
    let mut parts: Vec<Part> = Vec::with_capacity(topics.len() + assets.len() + 3);
    if opts.write_manifest {
        let spec = ManifestSpec {
            title: &title,
            locale: &opts.locale,
            toc: &toc,
            index: &index,
        };
        parts.push(Part::generated(
            manifest_name.clone(),
            write_manifest(&spec).into_bytes(),
        ));
    }
    if opts.write_opc {
        let mut extensions: BTreeSet<String> = BTreeSet::new();
        extensions.insert("rels".to_owned());
        for (part, _) in &topics {
            extensions.insert(extension_of(&part.name));
        }
        for part in &assets {
            extensions.insert(extension_of(&part.name));
        }
        let extensions: Vec<String> = extensions
            .into_iter()
            .filter(|name| !name.is_empty())
            .collect();
        parts.push(Part::generated(
            "[Content_Types].xml",
            content_types_xml(
                &extensions,
                opts.write_manifest.then_some(manifest_name.as_str()),
            )
            .into_bytes(),
        ));
        if opts.write_manifest {
            parts.push(Part::generated(
                "_rels/.rels",
                package_rels_xml(&manifest_name).into_bytes(),
            ));
        }
    }
    let topic_count = topics.len();
    let asset_count = assets.len();
    parts.extend(topics.into_iter().map(|(part, _)| part));
    parts.extend(assets);

    let bytes = write_zip(out, &parts, opts)?;
    Ok(BuildReport {
        out: out.to_path_buf(),
        bytes,
        topics: topic_count,
        assets: asset_count,
        toc_nodes,
        keywords: index.len(),
        warnings,
    })
}

/// Build the [`TopicRef`] for one topic, deriving the id when the packer is allowed to.
fn topic_ref(
    path: &str,
    meta: &TopicMeta,
    opts: &BuildOptions,
    warnings: &mut Vec<String>,
) -> Result<TopicRef, Error> {
    let id = match meta
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) => id.to_owned(),
        None if opts.allow_missing_ids => {
            let derived = slugify(&file_stem(path));
            warnings.push(format!(
                "topic {path} has no Microsoft.Help.Id; derived \"{derived}\" from the file name"
            ));
            derived
        }
        None => return Err(Error::MissingId(path.to_owned())),
    };
    let title = match meta
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(title) => title.to_owned(),
        None => {
            warnings.push(format!(
                "topic {path} has no <title> or Title meta tag; using its id as the TOC label"
            ));
            id.clone()
        }
    };
    if meta.toc_order_raw.is_some() && meta.toc_order.is_none() {
        warnings.push(format!(
            "topic {path} has a non-numeric Microsoft.Help.TocOrder ({}); using 0",
            meta.toc_order_raw.as_deref().unwrap_or("")
        ));
    }
    Ok(TopicRef {
        path: path.to_owned(),
        id,
        title,
        parent: meta.toc_parent.clone(),
        order: meta.toc_order.unwrap_or(0),
    })
}

/// The [`TopicRef`] for a topic of an existing package (`outline` has no options to apply).
fn topic_ref_from_meta(path: &str, meta: &TopicMeta) -> TopicRef {
    TopicRef {
        path: path.to_owned(),
        id: meta
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| slugify(&file_stem(path))),
        title: meta.effective_title(),
        parent: meta.toc_parent.clone(),
        order: meta.toc_order.unwrap_or(0),
    }
}

fn file_stem(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.')
        .map(|(stem, _)| stem.to_owned())
        .unwrap_or_else(|| base.to_owned())
}

fn index_key(keyword: &str, part: &str) -> String {
    format!(
        "{}\u{1}{}",
        keyword.to_lowercase(),
        part.to_ascii_lowercase()
    )
}

/// `<HelpContentSet Title>`: the configured title, else the output file name.
fn package_title(opts: &BuildOptions, out: &Path) -> String {
    opts.title
        .clone()
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| {
            out.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| "Help".to_owned())
        })
}

/// The manifest part name: `<slug of the title>.msha`, else the slug of the output file name.
fn manifest_name(opts: &BuildOptions, out: &Path) -> String {
    let slug = opts
        .title
        .as_deref()
        .map(slugify)
        .filter(|slug| !slug.is_empty())
        .or_else(|| {
            out.file_stem()
                .and_then(|stem| stem.to_str())
                .map(slugify)
                .filter(|slug| !slug.is_empty())
        })
        .unwrap_or_else(|| "help".to_owned());
    format!("{slug}.msha")
}

fn count_nodes(roots: &[TocNode]) -> usize {
    let mut count = 0usize;
    let mut stack: Vec<&TocNode> = roots.iter().collect();
    while let Some(node) = stack.pop() {
        count += 1;
        stack.extend(node.children.iter());
    }
    count
}

fn format_outline(roots: &[TocNode], topics: usize) -> String {
    let mut out = format!("TOC ({topics} topics, {} roots)\n", roots.len());
    let mut stack: Vec<(&TocNode, usize)> = roots.iter().rev().map(|node| (node, 0)).collect();
    while let Some((node, depth)) = stack.pop() {
        out.push_str(&"  ".repeat(depth));
        out.push_str(&node.title);
        if !node.id.is_empty() {
            out.push_str(&format!("  [{}]", node.id));
        }
        out.push('\n');
        for child in node.children.iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    out
}

/// Write every part into a fresh zip archive and put the archive at `out`.
///
/// The parts are written in the order they appear in `parts`, which is the documented stable
/// order, and every entry gets the same timestamp unless `preserve_times` is set. Returns the
/// size of the written file.
fn write_zip(out: &Path, parts: &[Part], opts: &BuildOptions) -> Result<u64, Error> {
    let buffer = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(buffer);
    for part in parts {
        let method = opts.compression.method();
        let level = match method {
            zip::CompressionMethod::Stored => None,
            _ => Some(6),
        };
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(level)
            .unix_permissions(0o644)
            .large_file(false)
            .last_modified_time(entry_timestamp(part, opts));
        writer.start_file(part.name.as_str(), options)?;
        writer
            .write_all(&part.bytes)
            .map_err(|error| Error::io(out, error))?;
    }
    let buffer = writer.finish()?.into_inner();

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
        }
    }
    fs::write(out, &buffer).map_err(|error| Error::io(out, error))?;
    Ok(buffer.len() as u64)
}

fn entry_timestamp(part: &Part, opts: &BuildOptions) -> zip::DateTime {
    if opts.preserve_times {
        part.modified
            .and_then(dos_timestamp)
            .unwrap_or_else(fixed_timestamp)
    } else {
        fixed_timestamp()
    }
}

fn fixed_timestamp() -> zip::DateTime {
    let (year, month, day, hour, minute, second) = FIXED_TIMESTAMP;
    zip::DateTime::from_date_and_time(year, month, day, hour, minute, second).unwrap_or_default()
}

/// Convert a `SystemTime` to a zip timestamp, clamped to the range the format can express
/// (1980–2107). `None` when the time is before the epoch.
fn dos_timestamp(time: SystemTime) -> Option<zip::DateTime> {
    let seconds = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let days = (seconds / 86_400) as i64;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    if !(1980..=2107).contains(&year) {
        return None;
    }
    zip::DateTime::from_date_and_time(
        year as u16,
        month as u8,
        day as u8,
        (second_of_day / 3600) as u8,
        ((second_of_day % 3600) / 60) as u8,
        (second_of_day % 60) as u8,
    )
    .ok()
}

/// Days since 1970-01-01 to `(year, month, day)` — Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_names_are_normalized() {
        assert_eq!(normalize_part_name("topics\\intro.htm"), "topics/intro.htm");
        assert_eq!(
            normalize_part_name("./topics/intro.htm"),
            "topics/intro.htm"
        );
        assert_eq!(
            normalize_part_name("/topics//intro.htm"),
            "topics//intro.htm"
        );
    }

    #[test]
    fn topic_detection_is_case_insensitive() {
        assert!(is_topic_part("topics/Intro.HTM"));
        assert!(is_topic_part("a.HTML"));
        assert!(is_topic_part("a.xHtMl"));
        assert!(!is_topic_part("assets/logo.bmp"));
        assert!(!is_topic_part("topics/no-extension"));
        assert_eq!(extension_of("assets/logo.BMP"), "bmp");
        assert_eq!(extension_of("no-dot"), "");
    }

    #[test]
    fn reserved_parts_are_recognized() {
        assert!(is_reserved_part("[Content_Types].xml"));
        assert!(is_reserved_part("_rels/.rels"));
        assert!(is_reserved_part("MyProduct.msha"));
        assert!(!is_reserved_part("topics/intro.htm"));
    }

    #[test]
    fn civil_dates_round_trip_known_values() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(18_262), (2020, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn the_fixed_timestamp_is_the_documented_one() {
        let stamp = fixed_timestamp();
        assert_eq!(stamp.year(), 2020);
        assert_eq!(stamp.month(), 1);
        assert_eq!(stamp.day(), 1);
        assert_eq!(stamp.hour(), 0);
    }

    #[test]
    fn timestamps_outside_the_zip_range_are_rejected() {
        assert!(dos_timestamp(UNIX_EPOCH).is_none());
        let in_range = UNIX_EPOCH + std::time::Duration::from_secs(18_262 * 86_400);
        assert!(dos_timestamp(in_range).is_some());
    }

    #[test]
    fn outline_formatting_indents_by_depth() {
        let nodes = vec![TocNode {
            id: "A".to_owned(),
            title: "Alpha".to_owned(),
            url: Some("a.htm".to_owned()),
            children: vec![TocNode {
                id: "B".to_owned(),
                title: "Beta".to_owned(),
                url: None,
                children: Vec::new(),
            }],
        }];
        let text = format_outline(&nodes, 2);
        assert_eq!(text, "TOC (2 topics, 1 roots)\nAlpha  [A]\n  Beta  [B]\n");
    }
}
