//! A tolerant, allocation-light HTML tokenizer.
//!
//! Help topics come from many authoring tools: XHTML, HTML 4 transitional, badly nested
//! fragments, Windows-1252 smart quotes. We therefore never validate — we tokenize and let
//! the arena builder deal with structure. The tokenizer knows only about:
//!
//! * comments `<!-- … -->`, doctypes `<!…>` and processing instructions `<?…>`,
//! * start tags with attributes (quoted or unquoted, single or double quotes),
//! * end tags, including implied end tags by omission (`<li>` inside `<li>`),
//! * raw-text elements (`script`, `style`, `textarea`, `title`) whose content is not markup,
//! * character references (`&amp;`, `&#233;`, `&#xE9;`).

use super::entities::decode_entities;

/// One token from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `<name a="1" b=2>`
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    /// `</name>`
    End { name: String },
    /// Character data, already character-reference decoded.
    Text(String),
    /// `<!-- … -->`, `<!DOCTYPE …>`, `<?xml …?>`
    Meta,
}

impl Event {
    /// Look up an attribute case-insensitively.
    pub fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
        attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Elements whose content is raw text (`<` does not start a tag inside them).
fn is_raw_text(name: &str) -> bool {
    matches!(name, "script" | "style" | "textarea" | "title")
}

/// Iterate the events of a document.
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    /// Set while inside a raw-text element: the tag we are waiting for `</…>`.
    raw: Option<String>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src,
            pos: 0,
            raw: None,
        }
    }

    pub fn offset(&self) -> usize {
        self.pos
    }

    /// Next event, or `None` at end of input.
    pub fn next_event(&mut self) -> Option<Event> {
        loop {
            if self.pos >= self.src.len() {
                return None;
            }
            if let Some(close_name) = self.raw.clone() {
                if let Some(ev) = self.raw_text(&close_name) {
                    return Some(ev);
                }
                continue;
            }
            let rest = &self.src[self.pos..];
            match rest.find('<') {
                None => {
                    let text = decode_entities(rest);
                    self.pos = self.src.len();
                    return Some(Event::Text(text));
                }
                Some(0) => {
                    if let Some(ev) = self.parse_markup() {
                        return Some(ev);
                    }
                    // Markup construct with no event of its own (comment/doctype): continue.
                }
                Some(idx) => {
                    let text = decode_entities(&rest[..idx]);
                    self.pos += idx;
                    return Some(Event::Text(text));
                }
            }
        }
    }

    /// Raw text until the matching end tag (script/style/textarea/title content).
    fn raw_text(&mut self, close_name: &str) -> Option<Event> {
        let rest = &self.src[self.pos..];
        let needle = format!("</{}", close_name);
        match find_ignore_ascii_case(rest, &needle) {
            Some(idx) => {
                let text = rest[..idx].to_owned();
                self.pos += idx;
                self.raw = None;
                // The following `</name>` is tokenized by the normal path.
                if text.is_empty() {
                    None
                } else {
                    Some(Event::Text(text))
                }
            }
            None => {
                let text = rest.to_owned();
                self.pos = self.src.len();
                self.raw = None;
                Some(Event::Text(text))
            }
        }
    }

    /// Parse one markup construct starting at `self.pos` (which points at `<`).
    fn parse_markup(&mut self) -> Option<Event> {
        let rest = &self.src[self.pos..];
        debug_assert!(rest.starts_with('<'));

        if rest.starts_with("<!--") {
            self.pos += match rest.find("-->") {
                Some(idx) => idx + 3,
                None => rest.len(),
            };
            return Some(Event::Meta);
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            self.pos += match rest.find('>') {
                Some(idx) => idx + 1,
                None => rest.len(),
            };
            return Some(Event::Meta);
        }
        if let Some(after) = rest.strip_prefix("</") {
            let name_len = after
                .find(|c: char| !is_name_char(c))
                .unwrap_or(after.len());
            let name = after[..name_len].to_ascii_lowercase();
            let close = after[name_len..].find('>').map(|i| name_len + i + 1);
            self.pos += match close {
                Some(end) => 2 + end,
                None => rest.len(),
            };
            if name.is_empty() {
                return None;
            }
            return Some(Event::End { name });
        }

        // `<name …>`
        let after = &rest[1..];
        let name_len = after
            .find(|c: char| !is_name_char(c))
            .unwrap_or(after.len());
        if name_len == 0 || !after.as_bytes()[0].is_ascii_alphabetic() {
            // Not a tag after all (`<` used as less-than). Emit it as text and move on.
            self.pos += 1;
            return Some(Event::Text("<".to_owned()));
        }
        let name = after[..name_len].to_ascii_lowercase();
        let mut cursor = 1 + name_len;
        let mut attrs = Vec::new();
        let mut self_closing = false;

        loop {
            if cursor >= rest.len() {
                // Unterminated tag: consume the rest.
                self.pos = self.src.len();
                break;
            }
            let bytes = rest.as_bytes();
            let ch = bytes[cursor] as char;
            if ch.is_ascii_whitespace() {
                cursor += 1;
                continue;
            }
            if ch == '>' {
                cursor += 1;
                self.pos += cursor;
                break;
            }
            if ch == '/' {
                // Could be `/` of `/>` or the start of an attribute-ish token.
                let next = bytes.get(cursor + 1).map(|b| *b as char);
                if matches!(next, Some('>')) {
                    self_closing = true;
                    cursor += 2;
                    self.pos += cursor;
                    break;
                }
                cursor += 1;
                continue;
            }

            // attribute name
            let name_start = cursor;
            while cursor < rest.len() {
                let c = rest.as_bytes()[cursor] as char;
                if c.is_ascii_whitespace() || c == '=' || c == '>' || c == '/' {
                    break;
                }
                cursor += 1;
            }
            let attr_name = rest[name_start..cursor].to_ascii_lowercase();
            // optional value
            let mut skip = cursor;
            while skip < rest.len() && (rest.as_bytes()[skip] as char).is_ascii_whitespace() {
                skip += 1;
            }
            let mut value = String::new();
            if skip < rest.len() && rest.as_bytes()[skip] == b'=' {
                skip += 1;
                while skip < rest.len() && (rest.as_bytes()[skip] as char).is_ascii_whitespace() {
                    skip += 1;
                }
                if skip < rest.len() {
                    let quote = rest.as_bytes()[skip];
                    if quote == b'"' || quote == b'\'' {
                        let q = quote as char;
                        let val_start = skip + 1;
                        match rest[val_start..].find(q) {
                            Some(idx) => {
                                value = decode_entities(&rest[val_start..val_start + idx]);
                                cursor = val_start + idx + 1;
                            }
                            None => {
                                value = decode_entities(&rest[val_start..]);
                                cursor = rest.len();
                            }
                        }
                    } else {
                        let val_start = skip;
                        let mut end = val_start;
                        while end < rest.len() {
                            let c = rest.as_bytes()[end] as char;
                            if c.is_ascii_whitespace() || c == '>' {
                                break;
                            }
                            end += 1;
                        }
                        let mut raw = &rest[val_start..end];
                        // `<img src=foo.bmp/>` — the trailing slash is the XHTML marker,
                        // not part of the URL.
                        if raw.ends_with('/') && rest.as_bytes().get(end) == Some(&b'>') {
                            raw = &raw[..raw.len() - 1];
                            self_closing = true;
                        }
                        value = decode_entities(raw);
                        cursor = end;
                    }
                }
            }
            if !attr_name.is_empty() {
                attrs.push((attr_name, value));
            }
            if cursor <= name_start {
                // Defensive: guarantee forward progress.
                cursor += 1;
            }
        }

        if is_raw_text(&name) && !self_closing {
            self.raw = Some(name.clone());
        }
        Some(Event::Start {
            name,
            attrs,
            self_closing,
        })
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
}

fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let hay = haystack.as_bytes();
    let pat = needle.as_bytes();
    if pat.is_empty() || hay.len() < pat.len() {
        return None;
    }
    let first_lower = pat[0].to_ascii_lowercase();
    let first_upper = pat[0].to_ascii_uppercase();
    for start in 0..=(hay.len() - pat.len()) {
        if hay[start] != first_lower && hay[start] != first_upper {
            continue;
        }
        if hay[start..start + pat.len()].eq_ignore_ascii_case(pat) {
            return Some(start);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(src: &str) -> Vec<Event> {
        let mut lex = Lexer::new(src);
        let mut out = Vec::new();
        while let Some(ev) = lex.next_event() {
            out.push(ev);
        }
        out
    }

    #[test]
    fn text_and_tags() {
        let ev = events("<p class=\"x\">Hi <b>there</b></p>");
        assert!(matches!(&ev[0], Event::Start { name, .. } if name == "p"));
        assert_eq!(ev[1], Event::Text("Hi ".into()));
        assert!(matches!(&ev[2], Event::Start { name, .. } if name == "b"));
        assert_eq!(ev[3], Event::Text("there".into()));
        assert_eq!(ev[4], Event::End { name: "b".into() });
    }

    #[test]
    fn attributes_forms() {
        let ev = events(r#"<img src=foo.bmp alt='a"b' data-x="y" disabled>"#);
        match &ev[0] {
            Event::Start { attrs, name, .. } => {
                assert_eq!(name, "img");
                assert_eq!(Event::attr(attrs, "SRC"), Some("foo.bmp"));
                assert_eq!(Event::attr(attrs, "alt"), Some("a\"b"));
                assert_eq!(Event::attr(attrs, "data-x"), Some("y"));
                assert_eq!(Event::attr(attrs, "disabled"), Some(""));
            }
            other => panic!("expected start tag, got {other:?}"),
        }
    }

    #[test]
    fn xhtml_self_closing_with_unquoted_url() {
        let ev = events("<img src=ASSETS/IMAGE.BMP/>");
        match &ev[0] {
            Event::Start {
                attrs,
                self_closing,
                name,
            } => {
                assert_eq!(name, "img");
                assert_eq!(Event::attr(attrs, "src"), Some("ASSETS/IMAGE.BMP"));
                assert!(self_closing);
            }
            other => panic!("expected start tag, got {other:?}"),
        }
    }

    #[test]
    fn entities_in_text_and_attrs() {
        let ev = events(r#"<a href="a.htm?x=1&amp;y=2">A &amp; B &#233;</a>"#);
        match &ev[0] {
            Event::Start { attrs, .. } => {
                assert_eq!(Event::attr(attrs, "href"), Some("a.htm?x=1&y=2"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(ev[1], Event::Text("A & B é".into()));
    }

    #[test]
    fn comments_and_doctypes() {
        let ev = events("<?xml version=\"1.0\"?><!DOCTYPE html><!-- hi --><p>x</p>");
        let metas = ev.iter().filter(|e| matches!(e, Event::Meta)).count();
        assert_eq!(metas, 3);
        assert!(ev.iter().any(|e| matches!(e, Event::Text(t) if t == "x")));
    }

    #[test]
    fn raw_text_elements() {
        let ev = events("<script>if (a < b) { x(); }</script><p>y</p>");
        assert!(matches!(&ev[0], Event::Start { name, .. } if name == "script"));
        assert!(matches!(&ev[1], Event::Text(t) if t.contains("a < b")));
        assert_eq!(ev[2], Event::End { name: "script".into() });
        assert!(matches!(&ev[3], Event::Start { name, .. } if name == "p"));
    }

    #[test]
    fn stray_lt_is_text() {
        let ev = events("a < b > c");
        let text: String = ev
            .iter()
            .filter_map(|e| match e {
                Event::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("a "));
        assert!(text.contains("<"));
        assert!(text.contains(" b > c"));
    }

    #[test]
    fn unterminated_tag_does_not_hang() {
        let ev = events("<p class=\"oops");
        assert_eq!(ev.len(), 1);
        assert!(matches!(&ev[0], Event::Start { name, .. } if name == "p"));
    }

    #[test]
    fn collapsible_area_custom_element() {
        let ev = events(r#"<CollapsibleArea Expanded="1" Title="More">text</CollapsibleArea>"#);
        match &ev[0] {
            Event::Start { name, attrs, .. } => {
                assert_eq!(name, "collapsiblearea");
                assert_eq!(Event::attr(attrs, "expanded"), Some("1"));
                assert_eq!(Event::attr(attrs, "title"), Some("More"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(ev[2], Event::End { name: "collapsiblearea".into() });
    }
}
