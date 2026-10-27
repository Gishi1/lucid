//! Snippets and highlight ranges (DESIGN §4.3).
//!
//! Body text is split into sentences/lines, a sliding window of at most
//! `max_snippet_chars` is scored by matched-term density (phrase matches count double)
//! and the best window is returned with `…` ellipses. When only the code field matched,
//! the snippet comes from the code line with the best match and is prefixed with
//! `⟨code⟩ `.
//!
//! All ranges are **char** indices into the produced snippet (never byte offsets), and
//! slicing the snippet with them yields the matched query term, case-insensitively.

use std::ops::Range;

use crate::tokenizer;

/// Prefix used when the snippet was taken from the code field.
pub const CODE_PREFIX: &str = "\u{27e8}code\u{27e9} ";

/// Shortest accepted `max_chars` (protects against a degenerate configuration).
const MIN_CHARS: usize = 32;
/// Upper bound on collected highlight ranges.
const MAX_RANGES: usize = 128;
/// Upper bound on occurrences considered while scoring one document.
const MAX_OCCURRENCES: usize = 512;

/// What to look for while building a snippet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnippetRequest {
    /// Terms used to find the best window (raw terms plus prefix/fuzzy expansions).
    pub terms: Vec<String>,
    /// Terms that produce highlight ranges (the raw query terms).
    pub highlight: Vec<String>,
    /// Query phrases; matches count double when scoring a window.
    pub phrases: Vec<Vec<String>>,
    /// Longest snippet, in characters.
    pub max_chars: usize,
}

/// Snippet text and the char ranges to highlight inside it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnippetOutput {
    /// The snippet (possibly prefixed with `⟨code⟩ ` and/or `…`).
    pub text: String,
    /// Char ranges into [`SnippetOutput::text`].
    pub ranges: Vec<Range<usize>>,
}

/// Drop empty/whitespace terms, lowercase them and de-duplicate.
fn clean_terms(terms: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for term in terms {
        let term = term.trim();
        if term.is_empty() {
            continue;
        }
        let lower = term.to_lowercase();
        if !out.contains(&lower) {
            out.push(lower);
        }
        if out.len() >= MAX_RANGES {
            break;
        }
    }
    out
}

/// Char length of a string.
fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// All case-insensitive occurrences of `terms` inside `text`, as char ranges of `text`.
///
/// An occurrence must start at a word boundary (so `install` does not match inside
/// `reinstallable`), which is what makes the ranges safe to highlight.
pub fn ranges_in(text: &str, terms: &[String]) -> Vec<Range<usize>> {
    let terms = clean_terms(terms);
    if terms.is_empty() || text.is_empty() {
        return Vec::new();
    }
    let (lower, map) = tokenizer::lowercase_with_map(text);
    find_ranges(&lower, &map, &terms)
}

/// Search the lowered text for every term and map the hits back to the original.
fn find_ranges(lower: &str, map: &[usize], terms: &[String]) -> Vec<Range<usize>> {
    let hay: Vec<char> = lower.chars().collect();
    let mut out: Vec<Range<usize>> = Vec::new();
    for term in terms {
        let needle: Vec<char> = term.chars().collect();
        if needle.is_empty() || needle.len() > hay.len() {
            continue;
        }
        let mut i = 0usize;
        while i + needle.len() <= hay.len() {
            let hit =
                hay[i..i + needle.len()] == needle[..] && (i == 0 || !hay[i - 1].is_alphanumeric());
            if hit {
                if let (Some(start), Some(end)) = (map.get(i), map.get(i + needle.len() - 1)) {
                    out.push(*start..*end + 1);
                }
                i += needle.len();
                if out.len() >= MAX_OCCURRENCES {
                    break;
                }
            } else {
                i += 1;
            }
        }
        if out.len() >= MAX_OCCURRENCES {
            break;
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    out.dedup();
    out
}

/// Split `text` into sentence/line units of at most `cap` characters.
///
/// Returns char ranges; together they cover the whole text.
fn split_units(text: &str, cap: usize) -> Vec<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let mut raw: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let ends_sentence = c == '\n'
            || ((c == '.' || c == '!' || c == '?')
                && chars.get(i + 1).is_none_or(|n| n.is_whitespace()));
        i += 1;
        if ends_sentence {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            raw.push(start..i);
            start = i;
        }
    }
    if start < chars.len() {
        raw.push(start..chars.len());
    }
    // Cut over-long paragraphs at the last whitespace before the cap.
    let cap = cap.max(MIN_CHARS);
    let mut out: Vec<Range<usize>> = Vec::with_capacity(raw.len());
    for unit in raw {
        let mut from = unit.start;
        while unit.end.saturating_sub(from) > cap {
            let limit = from + cap;
            let mut cut = limit;
            while cut > from && !chars[cut].is_whitespace() {
                cut -= 1;
            }
            if cut <= from {
                cut = limit;
            }
            out.push(from..cut);
            from = cut;
        }
        if from < unit.end {
            out.push(from..unit.end);
        }
    }
    out
}

/// Take `start..end` (char indices) of `text` as a string.
fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

/// Count occurrences fully inside `unit`.
fn count_inside(occ: &[Range<usize>], unit: &Range<usize>) -> usize {
    occ.iter()
        .filter(|o| o.start >= unit.start && o.end <= unit.end)
        .count()
}

/// Occurrences of a phrase: its words adjacent, whitespace only in between.
fn phrase_occurrences(text: &str, phrase: &[String]) -> Vec<Range<usize>> {
    let words = clean_terms(phrase);
    if words.len() < 2 {
        return Vec::new();
    }
    let (lower, map) = tokenizer::lowercase_with_map(text);
    let hay: Vec<char> = lower.chars().collect();
    let first: Vec<char> = words[0].chars().collect();
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut i = 0usize;
    while i + first.len() <= hay.len() {
        let matches =
            hay[i..i + first.len()] == first[..] && (i == 0 || !hay[i - 1].is_alphanumeric());
        if matches {
            let mut cursor = i + first.len();
            let mut ok = true;
            for word in &words[1..] {
                let needle: Vec<char> = word.chars().collect();
                let mut j = cursor;
                while j < hay.len() && hay[j].is_whitespace() {
                    j += 1;
                }
                if j + needle.len() > hay.len() || hay[j..j + needle.len()] != needle[..] {
                    ok = false;
                    break;
                }
                cursor = j + needle.len();
            }
            if ok {
                if let (Some(start), Some(end)) = (map.get(i), map.get(cursor - 1)) {
                    out.push(*start..*end + 1);
                }
                i = cursor;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A window of the body text together with how to render it.
struct Window {
    range: Range<usize>,
    start_ellipsis: bool,
    end_ellipsis: bool,
}

/// Pick the densest window of units that contains matches.
fn best_window(
    text: &str,
    units: &[Range<usize>],
    occ: &[Range<usize>],
    phrases: &[Vec<String>],
    max_chars: usize,
) -> Option<Window> {
    let counts: Vec<f32> = units
        .iter()
        .map(|u| count_inside(occ, u) as f32)
        .collect();
    let mut bonus = vec![0.0f32; units.len()];
    for phrase in phrases {
        for range in phrase_occurrences(text, phrase) {
            for (i, unit) in units.iter().enumerate() {
                if range.start >= unit.start && range.end <= unit.end {
                    bonus[i] += 1.0;
                }
            }
        }
    }
    let total = char_len(text);
    let mut best: Option<(f32, f32, Window)> = None;
    for (i, count) in counts.iter().enumerate() {
        if *count == 0.0 {
            continue;
        }
        // One unit of context before the match reads better — but it must be counted
        // against the budget, otherwise the window overshoots `max_chars`.
        let start = i.saturating_sub(1);
        let mut end = i;
        let mut chars = units[start].end - units[start].start;
        let mut score = counts[i] + 2.0 * bonus[i];
        if start != i {
            chars += units[i].end - units[i].start;
            score += counts[start] + 2.0 * bonus[start];
        }
        while end + 1 < units.len() {
            let next = units[end + 1].end - units[end + 1].start;
            if chars + next > max_chars {
                break;
            }
            end += 1;
            chars += next;
            score += counts[end] + 2.0 * bonus[end];
        }
        let density = score / (chars + 60) as f32;
        let window = Window {
            range: units[start].start..units[end].end,
            start_ellipsis: units[start].start > 0,
            end_ellipsis: units[end].end < total,
        };
        let better = match &best {
            None => true,
            Some((best_density, best_score, current)) => {
                density > *best_density
                    || (density == *best_density
                        && (score > *best_score || window.range.start < current.range.start))
            }
        };
        if better {
            best = Some((density, score, window));
        }
    }
    best.map(|(_, _, window)| window)
}

/// Render a char window of `text`: collapse whitespace, add ellipses and project the
/// highlight ranges into snippet coordinates.
///
/// `occ` are the occurrences that should be highlighted; `fallback` is used when none of
/// them falls inside the window (prefix/fuzzy matches are highlighted then).
fn render(
    text: &str,
    window: &Window,
    occ: &[Range<usize>],
    fallback: &[Range<usize>],
    prefix: &str,
) -> SnippetOutput {
    let chars: Vec<char> = slice_chars(text, window.range.start, window.range.end)
        .chars()
        .collect();
    let mut out = String::new();
    let mut out_len = 0usize;
    if !prefix.is_empty() {
        out.push_str(prefix);
        out_len += char_len(prefix);
    }
    if window.start_ellipsis {
        out.push('\u{2026}');
        out_len += 1;
    }
    let mut map: Vec<Option<usize>> = vec![None; chars.len()];
    let mut prev_space = true;
    for (i, c) in chars.iter().enumerate() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                map[i] = Some(out_len);
                out_len += 1;
                prev_space = true;
            }
            continue;
        }
        prev_space = false;
        map[i] = Some(out_len);
        out.push(*c);
        out_len += 1;
    }
    // drop the collapsed trailing space before closing the snippet
    if out.ends_with(' ') {
        out.pop();
        out_len -= 1;
        for entry in map.iter_mut() {
            if entry.is_some_and(|v| v >= out_len) {
                *entry = None;
            }
        }
    }
    if window.end_ellipsis {
        out.push('\u{2026}');
    }
    let window_start = window.range.start;
    let window_end = window.range.end;
    let project = |ranges: &[Range<usize>], out_ranges: &mut Vec<Range<usize>>| {
        for r in ranges {
            if r.start < window_start || r.end > window_end || r.start >= r.end {
                continue;
            }
            let (s, e) = (r.start - window_start, r.end - window_start);
            if e > chars.len() {
                continue;
            }
            if let (Some(Some(a)), Some(Some(b))) = (map.get(s), map.get(e - 1)) {
                out_ranges.push(*a..*b + 1);
            }
        }
    };
    let mut ranges: Vec<Range<usize>> = Vec::new();
    project(occ, &mut ranges);
    if ranges.is_empty() {
        project(fallback, &mut ranges);
    }
    ranges.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    ranges.dedup();
    ranges.truncate(MAX_RANGES);
    SnippetOutput { text: out, ranges }
}

/// First lines of `text`, used when nothing matched at all.
fn plain(text: &str, max_chars: usize, prefix: &str) -> SnippetOutput {
    let total = char_len(text);
    if total == 0 {
        return SnippetOutput::default();
    }
    let cap = max_chars.max(MIN_CHARS);
    let units = split_units(text, cap);
    let mut end = units.first().map_or(total, |u| u.end);
    for unit in units.iter().skip(1) {
        if unit.end <= cap {
            end = unit.end;
        } else {
            break;
        }
    }
    let window = Window {
        range: 0..end.min(total),
        start_ellipsis: false,
        end_ellipsis: end < total,
    };
    render(text, &window, &[], &[], prefix)
}

/// Snippet from the code line containing the most matches, prefixed with `⟨code⟩ `.
fn code_snippet(
    code: &str,
    occ: &[Range<usize>],
    fallback: &[Range<usize>],
    max_chars: usize,
) -> SnippetOutput {
    let units = split_units(code, max_chars.max(MIN_CHARS));
    let mut best: Option<(usize, Range<usize>)> = None;
    for unit in &units {
        let count = count_inside(occ, unit);
        if count == 0 {
            continue;
        }
        if best.as_ref().is_none_or(|(c, _)| count > *c) {
            best = Some((count, unit.clone()));
        }
    }
    let Some((_, range)) = best else {
        return plain(code, max_chars, CODE_PREFIX);
    };
    let window = Window {
        range,
        start_ellipsis: false,
        end_ellipsis: false,
    };
    let out = render(code, &window, occ, fallback, CODE_PREFIX);
    if out.text.trim().is_empty() || out.text.chars().count() <= char_len(CODE_PREFIX) {
        return plain(code, max_chars, CODE_PREFIX);
    }
    out
}

/// Build the snippet of a document from its body and code fields.
///
/// `req.terms` drives window selection (they include prefix/fuzzy expansions), while
/// `req.highlight` (the raw query terms) drives the returned ranges.
pub fn make(body: &str, code: &str, req: &SnippetRequest) -> SnippetOutput {
    let max_chars = req.max_chars.max(MIN_CHARS);
    let terms = clean_terms(&req.terms);
    let highlight = if req.highlight.is_empty() {
        terms.clone()
    } else {
        clean_terms(&req.highlight)
    };
    let body_occ = if body.trim().is_empty() {
        Vec::new()
    } else {
        ranges_in(body, &terms)
    };
    if !body_occ.is_empty() {
        let highlight_occ = ranges_in(body, &highlight);
        let units = split_units(body, max_chars);
        if let Some(window) = best_window(body, &units, &body_occ, &req.phrases, max_chars) {
            return render(body, &window, &highlight_occ, &body_occ, "");
        }
    }
    let code_occ = if code.trim().is_empty() {
        Vec::new()
    } else {
        ranges_in(code, &terms)
    };
    if !code_occ.is_empty() {
        let highlight_occ = ranges_in(code, &highlight);
        return code_snippet(code, &code_occ, &highlight_occ, max_chars);
    }
    if body.trim().is_empty() {
        plain(
            code,
            max_chars,
            if code.trim().is_empty() { "" } else { CODE_PREFIX },
        )
    } else {
        plain(body, max_chars, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(terms: &[&str], max: usize) -> SnippetRequest {
        SnippetRequest {
            terms: terms.iter().map(|t| t.to_string()).collect(),
            highlight: terms.iter().map(|t| t.to_string()).collect(),
            phrases: Vec::new(),
            max_chars: max,
        }
    }

    fn slice(text: &str, r: &Range<usize>) -> String {
        text.chars().skip(r.start).take(r.end - r.start).collect()
    }

    #[test]
    fn ranges_point_at_the_matched_words() {
        let body = "First line.\nNow install the package with cargo install.\nLast line.";
        let out = make(body, "", &req(&["install"], 80));
        assert!(!out.ranges.is_empty());
        for r in &out.ranges {
            assert_eq!(slice(&out.text, r).to_lowercase(), "install");
        }
        assert!(out.text.contains("install"));
    }

    #[test]
    fn unicode_lowercase_keeps_ranges_aligned() {
        let body = "Die Größe der Datei. İstanbul ist weit weg. Größe zählt.";
        let out = make(body, "", &req(&["größe"], 200));
        assert!(!out.ranges.is_empty());
        for r in &out.ranges {
            assert_eq!(slice(&out.text, r).to_lowercase(), "größe");
        }
    }

    #[test]
    fn ellipses_are_added_at_the_edges() {
        let body = format!(
            "{} install {}",
            "lots of text here. ".repeat(40),
            "more text follows. ".repeat(40)
        );
        let out = make(&body, "", &req(&["install"], 60));
        assert!(out.text.starts_with('\u{2026}'), "{}", out.text);
        assert!(out.text.ends_with('\u{2026}'), "{}", out.text);
        assert!(char_len(&out.text) <= 64, "{}", char_len(&out.text));
    }

    #[test]
    fn code_matches_are_prefixed() {
        let code = "let x = 1;\nlet package = install_package();\n";
        let out = make("", code, &req(&["install"], 80));
        assert!(out.text.starts_with(CODE_PREFIX), "{}", out.text);
        assert!(!out.ranges.is_empty());
        for r in &out.ranges {
            assert_eq!(slice(&out.text, r).to_lowercase(), "install");
        }
    }

    #[test]
    fn body_wins_over_code() {
        let out = make(
            "You can install Lucid from source.",
            "// install notes",
            &req(&["install"], 80),
        );
        assert!(!out.text.starts_with(CODE_PREFIX));
    }

    #[test]
    fn fallback_snippet_when_nothing_matched() {
        let out = make("Just a plain sentence about nothing.", "", &req(&["zzz"], 80));
        assert!(out.text.starts_with("Just a plain sentence"));
        assert!(out.ranges.is_empty());
        assert!(make("", "", &req(&["x"], 80)).text.is_empty());
    }

    #[test]
    fn whitespace_is_collapsed_and_ranges_survive() {
        let body = "line one\n\n   install\t\there\nline three";
        let out = make(body, "", &req(&["install"], 60));
        assert!(!out.text.contains('\n'));
        assert!(out.text.contains("install here"), "{}", out.text);
        for r in &out.ranges {
            assert_eq!(slice(&out.text, r).to_lowercase(), "install");
        }
    }

    #[test]
    fn phrases_score_double() {
        let body = "unrelated install text here. the install gui wizard is nice. tail.";
        let mut request = req(&["install"], 120);
        request.phrases = vec![vec!["install".to_string(), "gui".to_string()]];
        let out = make(body, "", &request);
        assert!(out.text.contains("install gui"), "{}", out.text);
    }

    #[test]
    fn expansions_are_highlighted_when_the_raw_term_is_absent() {
        let body = "You can install Lucid from source easily.";
        let request = SnippetRequest {
            terms: vec!["instalation".to_string(), "install".to_string()],
            highlight: vec!["instalation".to_string()],
            phrases: Vec::new(),
            max_chars: 80,
        };
        let out = make(body, "", &request);
        assert!(!out.ranges.is_empty());
        for r in &out.ranges {
            let text = slice(&out.text, r).to_lowercase();
            assert_eq!(text, "install");
        }
    }

    #[test]
    fn ranges_in_ignores_word_interiors() {
        let hits = ranges_in("reinstall and install", &["install".to_string()]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(slice("reinstall and install", &hits[0]), "install");
        assert!(ranges_in("Install", &["zzz".into()]).is_empty());
        assert!(ranges_in("", &["x".into()]).is_empty());
    }

    #[test]
    fn no_panic_on_odd_input() {
        for body in [
            "",
            " ",
            "\n\n\n",
            "😀 install 🎉",
            &"install ".repeat(500),
            "\u{feff}bom install",
            "install",
        ] {
            let out = make(body, "", &req(&["install", ""], 10));
            for r in &out.ranges {
                assert!(r.end <= char_len(&out.text), "range {r:?} outside snippet");
            }
            let _ = make("", body, &req(&["install"], 0));
            let _ = make(body, body, &req(&["install"], usize::MAX));
        }
    }
}
