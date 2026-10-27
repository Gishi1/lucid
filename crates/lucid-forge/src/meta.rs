//! Tolerant topic metadata extraction (the contract in `docs/DESIGN.md` §2.2) and text decoding.
//!
//! Extraction is driven by [`scraper`] (html5ever), which parses whatever a browser would
//! parse, and falls back to a small hand-rolled scanner for documents the HTML parser mangles.
//! Meta names are matched case-insensitively and without namespaces, so
//! `Microsoft.Help.Id`, `microsoft.help.id` and `MSHelp:Id` are the same key.

use scraper::{ElementRef, Html, Selector};

/// The metadata `lucid-forge` reads out of one topic document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopicMeta {
    /// Effective title: the first `<meta name="Title">` when present, else the `<title>` text.
    pub title: Option<String>,
    /// The text of `<title>`, kept separately because real packages carry both.
    pub document_title: Option<String>,
    /// `Microsoft.Help.Id` — the target of `ms-xhelp:///?Id=…` links.
    pub id: Option<String>,
    /// `Microsoft.Help.TocParent`; the root sentinel is interpreted by the caller.
    pub toc_parent: Option<String>,
    /// `Microsoft.Help.TocOrder`, parsed leniently; `None` when absent or garbage.
    pub toc_order: Option<i32>,
    /// The raw `Microsoft.Help.TocOrder` text, kept for diagnostics.
    pub toc_order_raw: Option<String>,
    /// `Microsoft.Help.Keywords`, split on commas/semicolons, duplicates removed.
    pub keywords: Vec<String>,
    /// Whether any `Microsoft.Help.Keywords` meta tag was present at all.
    pub keywords_declared: bool,
    /// `Microsoft.Help.F1` keywords.
    pub f1: Vec<String>,
    /// Whether any `Microsoft.Help.F1` meta tag was present at all.
    pub f1_declared: bool,
    /// Effective locale: `Microsoft.Help.Locale` / `Microsoft.Help.TopicLocale`, else the
    /// generic `Language` meta, else `<html lang>`.
    pub locale: Option<String>,
    /// The generic `Language` meta tag (kept for diagnostics).
    pub language: Option<String>,
    /// The `lang` attribute of `<html>` (kept for diagnostics).
    pub html_lang: Option<String>,
    /// `Description` — shown as a tooltip and indexed as its own search field.
    pub description: Option<String>,
    /// The declared charset (`<meta charset>` or a `Content-Type` meta), lowercased.
    pub charset: Option<String>,
}

impl TopicMeta {
    /// A title that is never empty: `title` → `id` → `(untitled)`.
    pub fn effective_title(&self) -> String {
        self.title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| self.id.clone().filter(|t| !t.trim().is_empty()))
            .unwrap_or_else(|| "(untitled)".to_owned())
    }
}

/// Extract metadata from already-decoded topic text.
pub fn extract_meta(html: &str) -> TopicMeta {
    let document = Html::parse_document(html);
    let mut meta = TopicMeta::default();

    for element in select(&document, "meta") {
        let value = element.value();
        if meta.charset.is_none() {
            if let Some(charset) = value
                .attr("charset")
                .map(str::trim)
                .filter(|charset| !charset.is_empty())
            {
                meta.charset = Some(charset.to_ascii_lowercase());
            }
        }
        let name = value
            .attr("name")
            .or_else(|| value.attr("property"))
            .or_else(|| value.attr("http-equiv"));
        let content = value.attr("content");
        if let (Some(equiv), Some(content)) = (value.attr("http-equiv"), content) {
            if equiv.eq_ignore_ascii_case("content-type") {
                if let Some(charset) = charset_in(content) {
                    meta.charset.get_or_insert(charset);
                }
            }
        }
        if let (Some(name), Some(content)) = (name, content) {
            apply(&mut meta, name, content);
        }
    }

    if let Some(element) = select(&document, "title").into_iter().next() {
        let text = collapse_whitespace(&element.text().collect::<String>());
        if !text.is_empty() {
            meta.document_title = Some(text);
        }
    }
    if let Some(element) = select(&document, "html").into_iter().next() {
        if let Some(lang) = element
            .value()
            .attr("lang")
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            meta.html_lang = Some(lang.to_owned());
        }
    }

    merge_missing(&mut meta, manual_scan(html));
    finish(&mut meta);
    meta
}

/// Decode topic bytes and extract their metadata.
pub fn extract_meta_bytes(bytes: &[u8]) -> TopicMeta {
    extract_meta(&decode_text(bytes))
}

/// Select `selector` in `document`, returning nothing instead of panicking when the selector
/// itself cannot be parsed (it can, for the selectors used here — this is only so that no
/// public function has a panic path).
fn select<'a>(document: &'a Html, selector: &str) -> Vec<ElementRef<'a>> {
    match Selector::parse(selector) {
        Ok(selector) => document.select(&selector).collect(),
        Err(_) => Vec::new(),
    }
}

/// Map a raw meta name to the canonical key used internally (case- and namespace-insensitive).
fn canonical_key(key: &str) -> Option<&'static str> {
    let lowered = key.trim().to_ascii_lowercase().replace([':', '_'], ".");
    let lowered = lowered.as_str();
    let lowered = lowered
        .strip_prefix("microsoft.help.")
        .or_else(|| lowered.strip_prefix("mshelp."))
        .unwrap_or(lowered);
    match lowered {
        "title" => Some("title"),
        "id" => Some("id"),
        "tocparent" => Some("tocparent"),
        "tocorder" => Some("tocorder"),
        "keywords" => Some("keywords"),
        "f1" => Some("f1"),
        "locale" => Some("locale"),
        "topiclocale" => Some("locale"),
        "language" => Some("language"),
        "description" => Some("description"),
        "charset" => Some("charset"),
        _ => None,
    }
}

fn apply(meta: &mut TopicMeta, key: &str, value: &str) {
    let Some(name) = canonical_key(key) else {
        return;
    };
    let value = value.trim();
    match name {
        "title" => set_once(&mut meta.title, value),
        "id" => set_once(&mut meta.id, value),
        "tocparent" => set_once(&mut meta.toc_parent, value),
        "tocorder" => {
            if meta.toc_order_raw.is_none() {
                meta.toc_order = parse_order(value);
                meta.toc_order_raw = Some(value.to_owned());
            }
        }
        "keywords" => {
            meta.keywords_declared = true;
            extend_unique(&mut meta.keywords, split_list(value));
        }
        "f1" => {
            meta.f1_declared = true;
            extend_unique(&mut meta.f1, split_list(value));
        }
        "locale" => set_once(&mut meta.locale, value),
        "language" => set_once(&mut meta.language, value),
        "description" => set_once(&mut meta.description, value),
        "charset" => set_once(&mut meta.charset, &value.to_ascii_lowercase()),
        _ => {}
    }
}

fn set_once(slot: &mut Option<String>, value: &str) {
    if slot.is_none() && !value.is_empty() {
        *slot = Some(value.to_owned());
    }
}

fn extend_unique(list: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !list
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&value))
        {
            list.push(value);
        }
    }
}

/// Fill in the effective title and locale once every source has been consulted.
fn finish(meta: &mut TopicMeta) {
    if meta.title.is_none() {
        meta.title = meta.document_title.clone();
    }
    if meta.locale.is_none() {
        meta.locale = meta
            .language
            .clone()
            .or_else(|| meta.html_lang.clone())
            .filter(|l| !l.trim().is_empty());
    }
}

fn merge_missing(meta: &mut TopicMeta, fallback: TopicMeta) {
    if meta.title.is_none() {
        meta.title = fallback.title;
    }
    if meta.document_title.is_none() {
        meta.document_title = fallback.document_title;
    }
    if meta.id.is_none() {
        meta.id = fallback.id;
    }
    if meta.toc_parent.is_none() {
        meta.toc_parent = fallback.toc_parent;
    }
    if meta.toc_order_raw.is_none() {
        meta.toc_order_raw = fallback.toc_order_raw;
        meta.toc_order = fallback.toc_order;
    }
    if !meta.keywords_declared {
        meta.keywords_declared = fallback.keywords_declared;
        meta.keywords = fallback.keywords;
    }
    if !meta.f1_declared {
        meta.f1_declared = fallback.f1_declared;
        meta.f1 = fallback.f1;
    }
    if meta.locale.is_none() {
        meta.locale = fallback.locale;
    }
    if meta.language.is_none() {
        meta.language = fallback.language;
    }
    if meta.description.is_none() {
        meta.description = fallback.description;
    }
    if meta.charset.is_none() {
        meta.charset = fallback.charset;
    }
}

/// A hand-rolled `<meta>` / `<title>` scan used when the HTML parser yields nothing (or as a
/// second opinion). It is intentionally forgiving: unquoted attributes and mis-nested tags are
/// fine, and only the first value of a key wins.
fn manual_scan(html: &str) -> TopicMeta {
    let mut meta = TopicMeta::default();
    let lowered = html.to_ascii_lowercase();

    if let Some(open) = lowered.find("<title") {
        if let Some(gt) = lowered[open..].find('>') {
            let text_start = open + gt + 1;
            if let Some(close) = lowered[text_start..].find("</title") {
                let text =
                    collapse_whitespace(&decode_entities(&html[text_start..text_start + close]));
                if !text.is_empty() {
                    meta.document_title = Some(text);
                }
            }
        }
    }

    let bytes = lowered.as_bytes();
    let mut from = 0usize;
    while let Some(offset) = lowered[from..].find("<meta") {
        let start = from + offset;
        let after = start + 5;
        let is_tag = matches!(
            bytes.get(after),
            Some(b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>')
        );
        if !is_tag {
            from = after;
            continue;
        }
        let Some(end) = lowered[start..].find('>').map(|gt| start + gt + 1) else {
            break;
        };
        apply_meta_tag(&mut meta, &html[start..end]);
        from = end;
    }
    meta
}

fn apply_meta_tag(meta: &mut TopicMeta, tag: &str) {
    if let Some(charset) = attr_value(tag, "charset")
        .map(|charset| charset.trim().to_ascii_lowercase())
        .filter(|charset| !charset.is_empty())
    {
        meta.charset.get_or_insert(charset);
    }
    let name = attr_value(tag, "name").or_else(|| attr_value(tag, "property"));
    let content = attr_value(tag, "content");
    if let (Some(equiv), Some(content)) = (attr_value(tag, "http-equiv"), content.as_deref()) {
        if equiv.eq_ignore_ascii_case("content-type") {
            if let Some(charset) = charset_in(content) {
                meta.charset.get_or_insert(charset);
            }
        }
    }
    if let (Some(name), Some(content)) = (name, content) {
        apply(meta, &name, &content);
    }
}

/// Read `attr` from a raw tag string (`<meta name="x" content='y'>`), quoted or not.
fn attr_value(tag: &str, attr: &str) -> Option<String> {
    if attr.is_empty() {
        return None;
    }
    let lowered = tag.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(offset) = lowered[from..].find(attr) {
        let position = from + offset;
        let after = position + attr.len();
        let before_ok = position == 0 || !is_name_byte(lowered.as_bytes()[position - 1]);
        let after_ok = !lowered
            .as_bytes()
            .get(after)
            .copied()
            .is_some_and(is_name_byte);
        from = after;
        if !before_ok || !after_ok {
            continue;
        }
        let rest = tag.get(after..).unwrap_or("").trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => {
                let inner = &rest[quote.len_utf8()..];
                let end = inner.find(quote).unwrap_or(inner.len());
                &inner[..end]
            }
            _ => {
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .unwrap_or(rest.len());
                &rest[..end]
            }
        };
        return Some(decode_entities(value));
    }
    None
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b':'
}

/// Find a `charset=…` / `encoding="…"` declaration in the head of a document.
fn charset_in(text: &str) -> Option<String> {
    let lowered = text.to_ascii_lowercase();
    let mut from = 0usize;
    while from < lowered.len() {
        let next = ["charset", "encoding"]
            .iter()
            .filter_map(|needle| {
                lowered[from..]
                    .find(needle)
                    .map(|offset| (from + offset, needle.len()))
            })
            .min_by_key(|(position, _)| *position);
        let Some((position, length)) = next else {
            break;
        };
        from = position + length;
        if let Some(name) = charset_value(&lowered, from) {
            return Some(name);
        }
    }
    None
}

fn charset_value(lowered: &str, start: usize) -> Option<String> {
    let rest = lowered.get(start..)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest
        .strip_prefix('"')
        .or_else(|| rest.strip_prefix('\''))
        .unwrap_or(rest);
    let name: String = rest
        .chars()
        .take(32)
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let name = name.trim_matches(|c: char| matches!(c, '-' | '_' | '.'));
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

/// Decode topic bytes to text.
///
/// A UTF-8 or UTF-16 BOM wins, then an explicit `charset`/`encoding` declaration
/// (`windows-1252` and friends are decoded properly, which matters because real packages ship
/// code-page encoded topics), then strict UTF-8, and finally windows-1252 — which cannot fail.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, false);
    }
    let head: String = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into_owned();
    let declared = charset_in(&head).map(|c| c.to_ascii_lowercase());
    match declared.as_deref() {
        Some("utf-8" | "utf8" | "unicode-1-1-utf-8") | None => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => decode_cp1252(bytes),
        },
        Some(
            "windows-1252" | "windows1252" | "cp1252" | "1252" | "iso-8859-1" | "iso8859-1"
            | "latin1" | "latin-1" | "ansi" | "us-ascii" | "ascii",
        ) => decode_cp1252(bytes),
        Some(_) => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => decode_cp1252(bytes),
        },
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// The windows-1252 mappings for `0x80..=0xA0` (the five undefined slots map to U+0081 etc.).
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}', '\u{017D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
];

/// Decode windows-1252 (a superset of ISO-8859-1). Never fails.
pub fn decode_cp1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if let Some(mapped) = CP1252_HIGH.get(byte.wrapping_sub(0x80) as usize) {
                *mapped
            } else {
                char::from(*byte)
            }
        })
        .collect()
}

/// The windows-1252 byte for `ch`, when the code page has one.
pub fn cp1252_byte(ch: char) -> Option<u8> {
    let code = ch as u32;
    if let Some(index) = CP1252_HIGH.iter().position(|mapped| *mapped == ch) {
        return Some(0x80 + index as u8);
    }
    if code < 0x80 || (0xA0..=0xFF).contains(&code) {
        return Some(code as u8);
    }
    None
}

/// Encode text as windows-1252, replacing characters the code page cannot represent with `?`.
pub fn encode_cp1252(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| cp1252_byte(ch).unwrap_or(b'?'))
        .collect()
}

/// Decode the handful of XML/HTML entities that show up in meta values.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find('&') {
        out.push_str(&rest[..position]);
        let after = &rest[position + 1..];
        let decoded = after.find(';').and_then(|semi| {
            let name = &after[..semi];
            if name.is_empty() || name.len() > 12 || name.contains([' ', '\t', '&', '<', '>']) {
                return None;
            }
            entity_value(name).map(|ch| (ch, semi + 1))
        });
        match decoded {
            Some((ch, consumed)) => {
                out.push(ch);
                rest = &after[consumed..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity_value(name: &str) -> Option<char> {
    match name.to_ascii_lowercase().as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{A0}'),
        other => {
            let digits = other.strip_prefix('#')?;
            let code = match digits.strip_prefix('x') {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// `Microsoft.Help.TocOrder` is an integer, but real files contain `4`, `4.0`, ` 4 `, `#12`,
/// `-3` and occasionally garbage. Take the first integer-looking run.
pub fn parse_order(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(number) = trimmed.parse::<i32>() {
        return Some(number);
    }
    let bytes = trimmed.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index].is_ascii_digit() {
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            let digits = trimmed.get(start..index)?;
            let negative = trimmed.get(..start)?.trim_end().ends_with('-');
            let mut number: i64 = digits.parse().ok()?;
            if negative {
                number = -number;
            }
            return Some(number.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
        }
        index += 1;
    }
    None
}

/// Split a keyword list on commas and semicolons, trimming and dropping empty entries.
pub fn split_list(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|part| !part.is_empty())
        .collect()
}

/// Turn arbitrary text into an id: lowercase, alphanumerics kept, everything else becomes a
/// single `-`. An empty result becomes `topic`.
pub fn slugify(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut separator = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            if separator && !out.is_empty() {
                out.push('-');
            }
            out.extend(ch.to_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if out.is_empty() {
        "topic".to_owned()
    } else {
        out
    }
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOPIC: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" lang="de-DE">
<head>
<title>Installing Lucid</title>
<meta http-equiv="Content-Type" content="text/html; charset=UTF-8" />
<meta name="Microsoft.Help.Id" content="Guides.Install" />
<meta name="microsoft.help.f1" content=" lucid install , setup " />
<meta name="MSHelp:TocParent" content="Guides" />
<meta name="Microsoft.Help.TocOrder" content=" 2 " />
<meta name="Microsoft.Help.Keywords" content="install, getting started;setup" />
<meta name="Microsoft.Help.Locale" content="en-US" />
<meta name="Description" content="How to install &amp; run." />
</head>
<body><div class="title">Installing Lucid</div></body>
</html>"#;

    #[test]
    fn extracts_the_documented_set() {
        let meta = extract_meta(TOPIC);
        assert_eq!(meta.title.as_deref(), Some("Installing Lucid"));
        assert_eq!(meta.document_title.as_deref(), Some("Installing Lucid"));
        assert_eq!(meta.id.as_deref(), Some("Guides.Install"));
        assert_eq!(meta.toc_parent.as_deref(), Some("Guides"));
        assert_eq!(meta.toc_order, Some(2));
        assert_eq!(meta.keywords, vec!["install", "getting started", "setup"]);
        assert!(meta.keywords_declared);
        assert_eq!(meta.f1, vec!["lucid install", "setup"]);
        assert_eq!(meta.locale.as_deref(), Some("en-US"));
        assert_eq!(meta.description.as_deref(), Some("How to install & run."));
        assert_eq!(meta.charset.as_deref(), Some("utf-8"));
        assert_eq!(meta.html_lang.as_deref(), Some("de-DE"));
    }

    #[test]
    fn meta_title_beats_the_title_element() {
        let meta = extract_meta(
            r#"<html><head><title>Document</title><meta name="Title" content="Meta" /></head></html>"#,
        );
        assert_eq!(meta.title.as_deref(), Some("Meta"));
        assert_eq!(meta.document_title.as_deref(), Some("Document"));
    }

    #[test]
    fn locale_falls_back_to_language_and_html_lang() {
        let meta = extract_meta(r#"<html lang="fr-FR"><head><title>x</title></head></html>"#);
        assert_eq!(meta.locale.as_deref(), Some("fr-FR"));
        let meta = extract_meta(
            r#"<html><head><meta name="Language" content="ja-JP" /><meta name="Microsoft.Help.Locale" content="en-us" /></head></html>"#,
        );
        assert_eq!(meta.locale.as_deref(), Some("en-us"));
    }

    #[test]
    fn falls_back_to_the_hand_written_scanner() {
        // html5ever drops a `<meta>` that appears after a stray `</head>`; the fallback keeps it.
        let broken = "<html><head></head><meta name=\"Microsoft.Help.Id\" content=\"Broken\">\
                      <meta name=\"Microsoft.Help.TocOrder\" content=\"seven\">";
        let meta = extract_meta(broken);
        assert_eq!(meta.id.as_deref(), Some("Broken"));
        assert_eq!(meta.toc_order_raw.as_deref(), Some("seven"));
        assert_eq!(meta.toc_order, None);
    }

    #[test]
    fn manual_scan_handles_unquoted_attributes() {
        let scanned = manual_scan("<meta name=Title content=Plain><title>Fallback</title>");
        assert_eq!(scanned.title.as_deref(), Some("Plain"));
        assert_eq!(scanned.document_title.as_deref(), Some("Fallback"));
    }

    #[test]
    fn decodes_windows_1252_topics() {
        let mut bytes = b"<html><head><meta http-equiv=\"Content-Type\" \
            content=\"text/html; charset=windows-1252\" /><title>caf"
            .to_vec();
        bytes.push(0xE9); // é
        bytes.extend_from_slice(b" \x80 \x97</title></head></html>");
        assert!(std::str::from_utf8(&bytes).is_err());
        let meta = extract_meta_bytes(&bytes);
        assert_eq!(meta.charset.as_deref(), Some("windows-1252"));
        assert_eq!(meta.title.as_deref(), Some("café € —"));
        assert_eq!(encode_cp1252("café € —"), b"caf\xE9 \x80 \x97".to_vec());
    }

    #[test]
    fn lenient_order_parsing() {
        assert_eq!(parse_order("4"), Some(4));
        assert_eq!(parse_order(" 4 "), Some(4));
        assert_eq!(parse_order("4.0"), Some(4));
        assert_eq!(parse_order("#12"), Some(12));
        assert_eq!(parse_order("-3"), Some(-3));
        assert_eq!(parse_order("abc"), None);
        assert_eq!(parse_order(""), None);
    }

    #[test]
    fn keywords_are_split_and_deduplicated() {
        assert_eq!(split_list(" a, b ; c "), vec!["a", "b", "c"]);
        assert_eq!(
            split_list("search/phrase, search"),
            vec!["search/phrase", "search"]
        );
        let meta = extract_meta(
            r#"<html><head><meta name="Microsoft.Help.F1" content="install, install" /></head></html>"#,
        );
        assert_eq!(meta.f1, vec!["install"]);
    }

    #[test]
    fn slugify_keeps_readable_ids() {
        assert_eq!(slugify("first-package"), "first-package");
        assert_eq!(slugify("Getting Started"), "getting-started");
        assert_eq!(slugify("FirstPackage"), "firstpackage");
        assert_eq!(slugify("///"), "topic");
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(decode_entities("a &amp; b &#233; &#x2014;"), "a & b é —");
        assert_eq!(decode_entities("100% &"), "100% &");
    }

    #[test]
    fn missing_metadata_is_none_not_a_panic() {
        let meta = extract_meta("<html><body>nothing</body></html>");
        assert_eq!(meta.title, None);
        assert_eq!(meta.id, None);
        assert!(!meta.keywords_declared);
        assert_eq!(meta.effective_title(), "(untitled)");
        let empty = extract_meta("");
        assert_eq!(empty.title, None);
        assert_eq!(empty.toc_order, None);
    }
}
