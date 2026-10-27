//! Topic metadata extraction — the contract described in `docs/DESIGN.md` §2.2.
//!
//! Names are matched case-insensitively and without namespaces (`Microsoft.Help.Id`,
//! `microsoft.help.id`, `MSHelp:Id` all work). Every tag we see is also kept verbatim in
//! [`RawMeta::all`] so `lucid info` can show unknown metadata.

use crate::html::{Event, Lexer};

/// Metadata of one topic, as found in its head (and, tolerantly, anywhere else).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawMeta {
    /// `<title>` or `<meta name="Title">`.
    pub title: Option<String>,
    /// `Microsoft.Help.Id` — the topic id (`ms-xhelp:///?Id=…`).
    pub id: Option<String>,
    /// `Microsoft.Help.TocParent` — parent topic id, `-1` (or absent) means root.
    pub toc_parent: Option<String>,
    /// `Microsoft.Help.TocOrder` (leniently parsed; `None` when absent/unparsable).
    pub toc_order: Option<i32>,
    /// `Microsoft.Help.Keywords` (comma/semicolon separated), plus generic `keywords`.
    pub keywords: Vec<String>,
    /// `Microsoft.Help.F1` keywords.
    pub f1: Vec<String>,
    /// `Microsoft.Help.Locale` (preferred) or a generic `Language` meta.
    pub locale: Option<String>,
    /// `Microsoft.Help.TopicLocale`.
    pub topic_locale: Option<String>,
    /// `Description`.
    pub description: Option<String>,
    pub product: Option<String>,
    pub product_version: Option<String>,
    pub topic_version: Option<String>,
    /// `Microsoft.Help.Category` entries (`Attribute:Value` pairs).
    pub categories: Vec<String>,
    /// `Microsoft.Help.Book` (older content).
    pub book: Option<String>,
    /// `Microsoft.Help.TopicPublishDate` (kept as text — formats vary).
    pub publish_date: Option<String>,
    /// `SelfBranded` / `Microsoft.Help.SelfBranded`.
    pub self_branded: Option<bool>,
    /// `Microsoft.Help.TopicVendor`.
    pub vendor: Option<String>,
    /// Declared charset (`<meta charset>` / `http-equiv="Content-Type"`).
    pub charset: Option<String>,
    /// `lang` attribute of `<html>`.
    pub html_lang: Option<String>,
    /// Every `name=value` meta pair seen, in document order (for `lucid info`).
    pub all: Vec<(String, String)>,
}

impl RawMeta {
    /// Title with an id-based fallback so the UI never shows an empty label.
    pub fn display_title(&self) -> String {
        if let Some(t) = self.title.as_ref().filter(|t| !t.trim().is_empty()) {
            return t.trim().to_owned();
        }
        if let Some(id) = self.id.as_ref().filter(|t| !t.trim().is_empty()) {
            return prettify_id(id);
        }
        "(untitled topic)".to_owned()
    }

    /// Look up an arbitrary meta tag (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.all
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Turn `My.Topic_Id-Name` (or `my.topic_id-name`) into `My Topic Id Name`, used as a
/// title fallback and for synthetic TOC grouping nodes.
pub fn prettify_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut at_word_start = true;
    for ch in id.chars() {
        if ch.is_alphanumeric() {
            if at_word_start {
                if !out.is_empty() {
                    out.push(' ');
                }
                for upper in ch.to_uppercase() {
                    out.push(upper);
                }
            } else {
                out.push(ch);
            }
            at_word_start = false;
        } else {
            at_word_start = true;
        }
    }
    out
}

/// Scan a topic for metadata.
pub fn scan(html: &str) -> RawMeta {
    let mut meta = RawMeta::default();
    let mut lexer = Lexer::new(html);
    let mut in_title = false;
    let mut title_buf = String::new();
    let mut seen = 0usize;

    while let Some(ev) = lexer.next_event() {
        seen += 1;
        // A topic cannot have relevant metadata after a few thousand tokens; this bounds
        // work on pathological documents.
        if seen > 20_000 {
            break;
        }
        match ev {
            Event::Start { name, attrs, .. } => match name.as_str() {
                "title" => {
                    in_title = true;
                    title_buf.clear();
                }
                "meta" => {
                    let key = Event::attr(&attrs, "name")
                        .or_else(|| Event::attr(&attrs, "property"))
                        .map(str::trim)
                        .filter(|k| !k.is_empty())
                        .map(str::to_owned);
                    let http_equiv = Event::attr(&attrs, "http-equiv").map(str::to_owned);
                    let content = Event::attr(&attrs, "content")
                        .map(crate::html::decode_entities)
                        .unwrap_or_default();
                    let charset_attr = Event::attr(&attrs, "charset")
                        .map(str::to_owned)
                        .filter(|c| !c.trim().is_empty());

                    if let Some(cs) = charset_attr {
                        meta.charset.get_or_insert(cs.trim().to_owned());
                    }
                    if let Some(equiv) = http_equiv {
                        if equiv.eq_ignore_ascii_case("content-type") {
                            if let Some(pos) = content.to_ascii_lowercase().find("charset=") {
                                let value = content[pos + 8..]
                                    .trim_matches(|c: char| {
                                        c.is_whitespace() || c == '"' || c == '\'' || c == ';'
                                    })
                                    .to_owned();
                                if !value.is_empty() {
                                    meta.charset.get_or_insert(value);
                                }
                            }
                        }
                    }
                    if let Some(key) = key {
                        meta.all.push((key.clone(), content.clone()));
                        apply(&mut meta, &key, &content);
                    }
                }
                "html" => {
                    if meta.html_lang.is_none() {
                        if let Some(lang) = Event::attr(&attrs, "lang") {
                            let lang = lang.trim();
                            if !lang.is_empty() {
                                meta.html_lang = Some(lang.to_owned());
                            }
                        }
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_title {
                    title_buf.push_str(&text);
                }
            }
            Event::End { name } => {
                if name == "title" {
                    in_title = false;
                    let t = title_buf.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !t.is_empty() && meta.title.is_none() {
                        meta.title = Some(t);
                    }
                }
            }
            Event::Meta => {}
        }
    }

    if meta.locale.is_none() {
        meta.locale = meta.html_lang.clone();
    }
    meta
}

fn apply(meta: &mut RawMeta, key: &str, content: &str) {
    let k = key.trim().to_ascii_lowercase();
    let value = content.trim();
    match k.as_str() {
        "title" => set_once(&mut meta.title, value),
        "microsoft.help.id" | "mshelp.id" | "mshelp:id" => set_once(&mut meta.id, value),
        "microsoft.help.tocparent" | "mshelp.tocparent" | "mshelp:tocparent" => {
            set_once(&mut meta.toc_parent, value)
        }
        "microsoft.help.tocorder" | "mshelp.tocorder" => {
            if meta.toc_order.is_none() {
                meta.toc_order = parse_order(value);
            }
        }
        "microsoft.help.keywords" | "mshelp.keywords" => {
            meta.keywords = split_list(value);
        }
        "keywords" => {
            if meta.keywords.is_empty() {
                meta.keywords = split_list(value);
            }
        }
        "microsoft.help.f1" | "mshelp.f1" => {
            meta.f1 = split_list(value);
        }
        "microsoft.help.locale" | "mshelp.locale" => set_once(&mut meta.locale, value),
        "microsoft.help.topiclocale" => set_once(&mut meta.topic_locale, value),
        "description" | "microsoft.help.description" => set_once(&mut meta.description, value),
        "microsoft.help.product" => set_once(&mut meta.product, value),
        "microsoft.help.productversion" => set_once(&mut meta.product_version, value),
        "microsoft.help.topicversion" => set_once(&mut meta.topic_version, value),
        "microsoft.help.category" => meta.categories.push(value.to_owned()),
        "microsoft.help.book" => set_once(&mut meta.book, value),
        "microsoft.help.topicpublishdate" => set_once(&mut meta.publish_date, value),
        "selfbranded" | "microsoft.help.selfbranded" => {
            if meta.self_branded.is_none() {
                meta.self_branded = Some(matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "1" | "yes"
                ));
            }
        }
        "microsoft.help.topicvendor" => set_once(&mut meta.vendor, value),
        "language" => {
            if meta.locale.is_none() {
                meta.locale = Some(value.to_owned());
            }
        }
        _ => {}
    }
}

fn set_once(slot: &mut Option<String>, value: &str) {
    if slot.is_none() && !value.is_empty() {
        *slot = Some(value.to_owned());
    }
}

/// `Microsoft.Help.TocOrder` is an integer, but real files contain `4`, `4.0`, ` 4 `, `#4`
/// and occasionally garbage. Take the first integer-looking run.
pub fn parse_order(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(n) = trimmed.parse::<i32>() {
        return Some(n);
    }
    let mut digits = String::new();
    let mut started = false;
    let mut negative = false;
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() {
            started = true;
            digits.push(ch);
        } else if !started {
            if ch == '-' {
                negative = true;
            } else if ch.is_whitespace() || ch == '#' || ch == '+' {
                continue;
            } else if idx > 0 {
                break;
            }
        } else {
            break;
        }
    }
    if digits.is_empty() {
        return None;
    }
    let n: i64 = digits.parse().ok()?;
    let n = if negative { -n } else { n };
    Some(n.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

/// Split a keyword list on commas/semicolons, trimming and dropping empties.
pub fn split_list(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOPIC: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html>
<head>
<title>Installing Lucid</title>
<meta http-equiv="Content-Type" content="text/html; charset=UTF-8" />
<meta name="Microsoft.Help.Id" content="Guides.Install" />
<meta name="microsoft.help.f1" content=" lucid install , setup " />
<meta name="Microsoft.Help.TocParent" content="Guides" />
<meta name="Microsoft.Help.TocOrder" content="2" />
<meta name="Microsoft.Help.Keywords" content="install, getting started;setup" />
<meta name="Microsoft.Help.Locale" content="en-US" />
<meta name="Description" content="How to install." />
<meta name="SelfBranded" content="false" />
<meta name="MSHelp:Id" content="ignored-because-first-wins" />
</head>
<body><div class="title">Installing Lucid</div></body>
</html>"#;

    #[test]
    fn extracts_the_documented_set() {
        let m = scan(TOPIC);
        assert_eq!(m.title.as_deref(), Some("Installing Lucid"));
        assert_eq!(m.id.as_deref(), Some("Guides.Install"));
        assert_eq!(m.toc_parent.as_deref(), Some("Guides"));
        assert_eq!(m.toc_order, Some(2));
        assert_eq!(m.keywords, vec!["install", "getting started", "setup"]);
        assert_eq!(m.f1, vec!["lucid install", "setup"]);
        assert_eq!(m.locale.as_deref(), Some("en-US"));
        assert_eq!(m.description.as_deref(), Some("How to install."));
        assert_eq!(m.self_branded, Some(false));
        assert_eq!(m.charset.as_deref(), Some("UTF-8"));
    }

    #[test]
    fn first_value_wins_for_id() {
        let m = scan(TOPIC);
        assert_eq!(m.id.as_deref(), Some("Guides.Install"));
    }

    #[test]
    fn meta_title_alias_and_id_fallback() {
        let m = scan(r#"<html><head><meta name="Title" content="From meta"></head><body/></html>"#);
        assert_eq!(m.display_title(), "From meta");

        let m2 = scan(r#"<html><head><meta name="Microsoft.Help.Id" content="Guides.Search_Phrase"></head>"#);
        assert_eq!(m2.display_title(), "Guides Search Phrase");
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
    fn html_lang_is_a_locale_fallback() {
        let m = scan(r#"<html lang="de-DE"><head><title>x</title></head></html>"#);
        assert_eq!(m.locale.as_deref(), Some("de-DE"));
    }

    #[test]
    fn absent_metadata_is_none_not_panic() {
        let m = scan("<html><body>nothing</body></html>");
        assert_eq!(m.title, None);
        assert_eq!(m.id, None);
        assert_eq!(m.toc_order, None);
        assert!(m.keywords.is_empty());
        assert_eq!(m.display_title(), "(untitled topic)");
    }

    #[test]
    fn keeps_raw_pairs_in_order() {
        let m = scan(TOPIC);
        assert_eq!(m.all[0].0, "Microsoft.Help.Id");
        assert_eq!(m.get("microsoft.help.tocparent"), Some("Guides"));
    }
}
