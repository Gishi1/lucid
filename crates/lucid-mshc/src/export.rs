//! Export topics (or a whole book) to standalone HTML.
//!
//! The original markup is preserved byte-for-byte except for:
//!
//! * the declared charset (everything is written as UTF-8),
//! * references to assets (`src`/`href`/`poster`/`data`), which are copied next to the
//!   export and rewritten to relative paths,
//! * cross-topic links, rewritten to the exported file names,
//! * an injected minimal stylesheet so an exported topic is readable in a browser without
//!   the (missing) branding package.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::docset::{DocSet, LinkTarget};
use crate::error::{Error, Result};
use crate::model;
use crate::package::TopicKey;
use crate::progress::Progress;

/// What an export produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportReport {
    pub out: PathBuf,
    pub files: usize,
    pub assets: usize,
    pub bytes: u64,
    pub warnings: Vec<String>,
}

/// Tags whose URL attributes are rewritten.
const URL_TAGS: &[&str] = &[
    "img", "link", "script", "source", "embed", "iframe", "input", "track", "video", "audio",
    "object", "area",
];

/// Attributes that carry URLs.
const URL_ATTRS: &[&str] = &["src", "href", "poster", "data", "background"];

/// Export a single topic plus the assets it references.
pub fn export_topic(
    set: &DocSet,
    key: TopicKey,
    out_dir: &Path,
    overwrite: bool,
) -> Result<ExportReport> {
    let file_name = format!("{}.html", file_stem_for(set, key));
    let mut report = export_many(set, &[(key, file_name.clone())], out_dir, overwrite)?;
    report.out = out_dir.join(&file_name);
    Ok(report)
}

/// Export every topic of the content set, plus an `index.html` listing the TOC.
pub fn export_book(
    set: &DocSet,
    out_dir: &Path,
    overwrite: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<ExportReport> {
    // Deterministic file names in TOC order, then any topic missing from the TOC.
    let mut ordered: Vec<TopicKey> = Vec::new();
    crate::toc::TocNode::walk_forest(&set.toc, &mut |node, _| {
        if let Some(key) = node.key {
            if !ordered.contains(&key) {
                ordered.push(key);
            }
        }
    });
    for (pkg_index, package) in set.packages.iter().enumerate() {
        for topic in &package.topics {
            let key = TopicKey::new(pkg_index as u32, topic.key.topic);
            if !ordered.contains(&key) {
                ordered.push(key);
            }
        }
    }

    let mut names: HashMap<TopicKey, String> = HashMap::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    for key in &ordered {
        let stem = file_stem_for(set, *key);
        let counter = used.entry(stem.clone()).or_insert(0);
        *counter += 1;
        let name = if *counter == 1 {
            format!("{stem}.html")
        } else {
            format!("{stem}-{}.html", *counter)
        };
        names.insert(*key, name);
    }

    let pairs: Vec<(TopicKey, String)> = ordered
        .iter()
        .map(|key| (*key, names[key].clone()))
        .collect();
    let mut report = export_many(set, &pairs, out_dir, overwrite)?;
    report.out = out_dir.to_path_buf();

    // index.html with the TOC
    std::fs::create_dir_all(out_dir).map_err(|e| Error::io(out_dir, e))?;
    let index_path = out_dir.join("index.html");
    if index_path.exists() && !overwrite {
        report
            .warnings
            .push(format!("{} exists, not overwritten", index_path.display()));
    } else {
        let title = set
            .packages
            .first()
            .and_then(|p| p.title.clone())
            .unwrap_or_else(|| "Lucid export".to_owned());
        let mut html = String::new();
        html.push_str("<!DOCTYPE html>\n<html lang=\"");
        html.push_str(&set.primary_locale().unwrap_or_else(|| "en".to_owned()));
        html.push_str("\">\n<head>\n<meta charset=\"utf-8\">\n<title>");
        html.push_str(&escape(&title));
        html.push_str("</title>\n");
        html.push_str(STYLE);
        html.push_str("</head>\n<body class=\"lucid-export\">\n<h1>");
        html.push_str(&escape(&title));
        html.push_str("</h1>\n");

        fn render(
            nodes: &[crate::toc::TocNode<TopicKey>],
            names: &HashMap<TopicKey, String>,
            out: &mut String,
        ) {
            out.push_str("<ul class=\"lucid-toc\">\n");
            for node in nodes {
                out.push_str("<li>");
                match node.key.and_then(|key| names.get(&key)) {
                    Some(file) => {
                        out.push_str("<a href=\"");
                        out.push_str(&escape(file));
                        out.push_str("\">");
                        out.push_str(&escape(&node.title));
                        out.push_str("</a>");
                    }
                    None => out.push_str(&escape(&node.title)),
                }
                if !node.children.is_empty() {
                    render(&node.children, names, out);
                }
                out.push_str("</li>\n");
            }
            out.push_str("</ul>\n");
        }
        render(&set.toc, &names, &mut html);
        html.push_str("</body>\n</html>\n");
        std::fs::write(&index_path, html).map_err(|e| Error::io(&index_path, e))?;
        report.files += 1;
        report.bytes += std::fs::metadata(&index_path).map(|m| m.len()).unwrap_or(0);
        progress(Progress::new(
            crate::progress::Stage::Done,
            1,
            1,
            "index.html",
        ));
    }

    Ok(report)
}

fn export_many(
    set: &DocSet,
    topics: &[(TopicKey, String)],
    out_dir: &Path,
    overwrite: bool,
) -> Result<ExportReport> {
    std::fs::create_dir_all(out_dir).map_err(|e| Error::io(out_dir, e))?;
    let assets_dir = out_dir.join("assets");
    let mut report = ExportReport::default();
    let mut asset_names: HashMap<String, String> = HashMap::new();
    // Assets are shared by many topics: copy (and count) each one once.
    let mut written_assets: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut topic_names: HashMap<TopicKey, String> = topics.iter().cloned().collect();

    for (key, file_name) in topics {
        let html = set.topic_html(*key)?;
        let dir_name = Path::new(file_name)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();

        let mut referenced: Vec<String> = Vec::new();
        let rewritten = rewrite_references(&html, |attr, raw| {
            if is_external(raw) || raw.starts_with('#') || raw.trim().is_empty() {
                return None;
            }
            let lower = raw.to_ascii_lowercase();
            if lower.starts_with("ms-xhelp:") || lower.starts_with("ms-help:") {
                // Rewrite to the exported topic when it is part of this export.
                if let LinkTarget::Topic(target) = set.resolve(*key, raw) {
                    if let Some(name) = topic_names.get(&target) {
                        return Some(name.clone());
                    }
                    if let Some(target_set) = set.topic(target) {
                        let name = format!("{}.html", slugify(&target_set.title));
                        topic_names.insert(target, name.clone());
                        return Some(name);
                    }
                }
                return None;
            }
            // Topics referenced by path become exported file names.
            if let Some(target) = set.resolve_topic_path(*key, raw) {
                if let Some(name) = topic_names.get(&target) {
                    if lower.ends_with(".htm") || lower.ends_with(".html") || lower.ends_with(".xhtml")
                    {
                        return Some(join_relative(&dir_name, name));
                    }
                }
            }
            // Everything else that resolves inside a package is an asset to copy.
            if set.asset(*key, raw).ok().flatten().is_some() {
                let base = raw
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(raw)
                    .to_ascii_lowercase();
                let local = asset_names
                    .entry(base.clone())
                    .or_insert_with(|| base.clone())
                    .clone();
                if !referenced.contains(&local) {
                    referenced.push(local.clone());
                }
                let _ = attr;
                return Some(join_relative(&dir_name, &format!("assets/{local}")));
            }
            None
        });

        for asset_name in referenced {
            if !written_assets.insert(asset_name.clone()) {
                continue;
            }
            let path = assets_dir.join(&asset_name);
            if path.exists() && !overwrite {
                continue;
            }
            // Find the asset by basename across the set.
            let mut loaded = None;
            for package in &set.packages {
                if let Some(asset) = package.asset_bytes(&asset_name)? {
                    loaded = Some(asset);
                    break;
                }
            }
            if let Some(asset) = loaded {
                std::fs::create_dir_all(&assets_dir).map_err(|e| Error::io(&assets_dir, e))?;
                std::fs::write(&path, &asset.bytes).map_err(|e| Error::io(&path, e))?;
                report.assets += 1;
                report.bytes += asset.bytes.len() as u64;
            } else {
                report
                    .warnings
                    .push(format!("asset `{asset_name}` not found while exporting"));
            }
        }

        let target = out_dir.join(file_name);
        if target.exists() && !overwrite {
            report
                .warnings
                .push(format!("{} exists, not overwritten", target.display()));
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let document = inject_style_and_charset(&rewritten);
        std::fs::write(&target, document.as_bytes()).map_err(|e| Error::io(&target, e))?;
        report.files += 1;
        report.bytes += document.len() as u64;
    }

    Ok(report)
}

/// Rewrite URL attributes in place using `resolve`; untouched references stay as they are.
fn rewrite_references(html: &str, mut resolve: impl FnMut(&str, &str) -> Option<String>) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let rest = &html[cursor..];
        let Some(tag_start) = rest.find('<') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..tag_start]);
        let tag = &rest[tag_start..];
        // Only the tag name matters for matching, and slicing must stay on character
        // boundaries (topics contain em dashes, CJK and other multi-byte text).
        let probe_end = tag
            .char_indices()
            .nth(64)
            .map(|(index, _)| index)
            .unwrap_or(tag.len());
        let lower_tag = tag[..probe_end].to_ascii_lowercase();
        let matched = URL_TAGS
            .iter()
            .find(|name| lower_tag.starts_with(&format!("<{name}"))
                && lower_tag
                    .chars()
                    .nth(name.len() + 1)
                    .map(|c| c.is_whitespace() || c == '>' || c == '/')
                    .unwrap_or(true));
        let Some(name) = matched else {
            out.push('<');
            cursor += tag_start + 1;
            continue;
        };
        let Some(tag_end) = find_tag_end(tag) else {
            out.push_str(tag);
            break;
        };
        let tag_text = &tag[..tag_end];
        out.push_str(&rewrite_tag(tag_text, name, &mut resolve));
        cursor += tag_start + tag_end;
    }
    out
}

fn rewrite_tag(
    tag: &str,
    _name: &str,
    resolve: &mut impl FnMut(&str, &str) -> Option<String>,
) -> String {
    let mut out = String::with_capacity(tag.len());
    let bytes = tag.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        // Find the next attribute name start.
        let rest = &tag[cursor..];
        let Some(eq) = rest.find('=') else {
            out.push_str(rest);
            break;
        };
        let name_start = rest[..eq]
            .rfind(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<')
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let attr_name = rest[name_start..eq].trim().to_ascii_lowercase();
        out.push_str(&rest[..eq + 1]);
        let after_eq = &rest[eq + 1..];
        let mut idx = 0usize;
        while idx < after_eq.len() && after_eq.as_bytes()[idx].is_ascii_whitespace() {
            idx += 1;
        }
        let (quote, value_start) = match after_eq.as_bytes().get(idx) {
            Some(b'"') => (Some('"'), idx + 1),
            Some(b'\'') => (Some('\''), idx + 1),
            _ => (None, idx),
        };
        let value_end = match quote {
            Some(q) => after_eq[value_start..]
                .find(q)
                .map(|i| value_start + i)
                .unwrap_or(after_eq.len()),
            None => after_eq[value_start..]
                .find(|c: char| c.is_whitespace() || c == '>')
                .map(|i| value_start + i)
                .unwrap_or(after_eq.len()),
        };
        let value = &after_eq[value_start..value_end];
        let replacement = if URL_ATTRS.contains(&attr_name.as_str()) {
            resolve(&attr_name, value)
        } else {
            None
        };
        if let Some(new_value) = replacement {
            if let Some(q) = quote {
                out.push(q);
                out.push_str(&new_value);
                out.push(q);
            } else {
                out.push_str(&new_value);
            }
        } else {
            if let Some(q) = quote {
                out.push(q);
            }
            out.push_str(value);
            if let Some(q) = quote {
                out.push(q);
            }
        }
        cursor += eq + 1 + value_end;
        if quote.is_some() {
            cursor += 1;
        }
    }
    out
}

fn find_tag_end(tag: &str) -> Option<usize> {
    let bytes = tag.as_bytes();
    let mut quote: Option<u8> = None;
    for (idx, byte) in bytes.iter().enumerate() {
        match (quote, byte) {
            (Some(q), b) if *b == q => quote = None,
            (None, b'"') | (None, b'\'') => quote = Some(*byte),
            (None, b'>') => return Some(idx + 1),
            _ => {}
        }
    }
    None
}

/// Make an exported document self-contained: UTF-8 charset + a readable stylesheet.
fn inject_style_and_charset(html: &str) -> String {
    let mut out = html.to_owned();
    // Normalize the declared charset (exported files are UTF-8).
    if let Some(idx) = out.to_ascii_lowercase().find("charset=") {
        let value_start = idx + "charset=".len();
        let rest = &out[value_start..];
        let end = rest
            .find(|c: char| c == '"' || c == '\'' || c == ';' || c == '>' || c.is_whitespace())
            .unwrap_or(rest.len());
        if !rest.is_empty() && end > 0 {
            out.replace_range(value_start..value_start + end, "utf-8");
        }
    }
    match out.to_ascii_lowercase().find("</head>") {
        Some(idx) => out.insert_str(idx, STYLE),
        None => {
            let mut prefixed = String::with_capacity(out.len() + STYLE.len());
            prefixed.push_str(STYLE);
            prefixed.push_str(&out);
            out = prefixed;
        }
    }
    out
}

fn file_stem_for(set: &DocSet, key: TopicKey) -> String {
    let title = set.topic_title(key);
    let slug = slugify(title);
    if slug.is_empty() {
        slugify(&set.topic_id(key))
    } else {
        slug
    }
}

fn slugify(text: &str) -> String {
    model::slugify(text)
}

fn join_relative(dir: &Path, name: &str) -> String {
    if dir.as_os_str().is_empty() {
        name.to_owned()
    } else {
        format!("{}/{}", dir.to_string_lossy().replace('\\', "/"), name)
    }
}

fn is_external(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    ["http://", "https://", "ftp://", "mailto:", "file:", "data:", "javascript:", "//"]
        .iter()
        .any(|p| lower.starts_with(p))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const STYLE: &str = r#"<style>
:root { color-scheme: light dark; }
body.lucid-export, body { font-family: -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  line-height: 1.55; max-width: 52rem; margin: 2rem auto; padding: 0 1rem; }
pre, code { font-family: ui-monospace, "Cascadia Mono", Consolas, monospace; font-size: 0.92em; }
pre { background: rgba(127,127,127,.14); padding: .8rem 1rem; border-radius: 6px; overflow-x: auto; }
table { border-collapse: collapse; margin: 1rem 0; }
th, td { border: 1px solid rgba(127,127,127,.45); padding: .35rem .6rem; text-align: left; vertical-align: top; }
th { background: rgba(127,127,127,.14); }
div.alert, table.alert { border-left: 3px solid #4f8cff; background: rgba(79,140,255,.08);
  padding: .6rem .9rem; border-radius: 4px; margin: 1rem 0; }
div.title { font-size: 1.6rem; font-weight: 600; margin-bottom: .75rem; }
img { max-width: 100%; height: auto; }
ul.lucid-toc ul { margin-top: .2rem; }
</style>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_only_selected_attributes() {
        let html = r#"<p>text</p><img src="ASSETS/A.BMP" alt="x"><a href="topic.htm">t</a>"#;
        let out = rewrite_references(html, |attr, raw| {
            if attr == "src" && raw == "ASSETS/A.BMP" {
                Some("assets/a.bmp".to_owned())
            } else {
                None
            }
        });
        assert!(out.contains(r#"src="assets/a.bmp""#));
        assert!(out.contains(r#"href="topic.htm""#));
        assert!(out.contains(">text<"));
    }

    #[test]
    fn multi_byte_text_near_the_tag_probe_limit_is_safe() {
        // Regression: slicing the tag prefix at 64 *bytes* used to panic inside an em dash.
        let text = "—".repeat(40);
        let html = format!("<p>{text}</p><img src=\"ASSETS/A.BMP\"><p>{text}</p>");
        let out = rewrite_references(&html, |attribute, _| {
            (attribute == "src").then(|| "assets/a.bmp".to_owned())
        });
        assert!(out.contains("assets/a.bmp"));
        assert!(out.contains(&text));

        // The same, with a multi-byte tag name prefix and CJK content.
        let html = "<div class=\"タイトル\"><span>本文</span></div>";
        let out = rewrite_references(html, |_, _| None);
        assert_eq!(out, html);
    }

    #[test]
    fn keeps_unquoted_attributes_readable() {
        let html = "<img src=assets/x.png alt=y>";
        let out = rewrite_references(html, |_, _| Some("z.png".to_owned()));
        assert!(out.contains("src=z.png"));
        assert!(out.contains("alt=y"));
    }

    #[test]
    fn injects_style_and_fixes_charset() {
        let html = r#"<html><head><meta http-equiv="Content-Type" content="text/html; charset=windows-1252"></head><body>x</body></html>"#;
        let out = inject_style_and_charset(html);
        assert!(out.contains("charset=utf-8"));
        assert!(out.contains("lucid-export"));
        assert!(out.contains("<body>x</body>"));
    }

    #[test]
    fn tag_end_respects_quotes() {
        // The `>` inside the quoted value must not end the tag; the returned offset is
        // exclusive (just past the real `>`).
        assert_eq!(find_tag_end(r#"<img src="a>b.png">"#), Some(19));
        assert_eq!(find_tag_end("<img src=x"), None);
    }

}
