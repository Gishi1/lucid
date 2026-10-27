//! Unicode-aware tokenizer shared by the index, the query parser and the GUI.
//!
//! The tokenizer is deliberately *generative*: a word produces the joined identifier
//! (`GetWindowRect` → `getwindowrect`), the camelCase/snake_case pieces (`get`,
//! `window`, `rect`) and, for CJK text, one token per Han/Hiragana/Katakana character
//! plus a joined token for the run. Every token keeps the **char range** it came from
//! so the GUI can highlight matches in the original text.
//!
//! Tokens are emitted in the order they occupy *positions* in the field, and the
//! joined form always precedes its pieces. That ordering is what makes phrases such
//! as `"get window"` match `GetWindowRect` while ordinary word adjacency still works.

use std::borrow::Cow;

use rust_stemmers::{Algorithm, Stemmer};
use unicode_segmentation::UnicodeSegmentation;

/// A normalized token and the char range it was produced from.
///
/// `start`/`end` are **char** indices into the text that was tokenized (`end` is
/// exclusive), so `text.chars().skip(t.start).take(t.len())` returns the original
/// spelling of the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Lowercased token text.
    pub text: String,
    /// Char index of the first character of the token in the original text.
    pub start: usize,
    /// Char index one past the last character of the token in the original text.
    pub end: usize,
}

impl Token {
    /// Number of characters the token covers in the original text.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// True when the token covers no characters (never produced by [`tokenize`]).
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// Characters that glue an identifier together (`ms-xhelp`, `snake_case`, `a.b`).
const CONNECTORS: [char; 6] = ['-', '_', '.', '\'', '\u{2019}', '/'];

/// Longest CJK run that still gets a joined token (keeps the dictionary sane).
const MAX_CJK_RUN: usize = 8;

/// True for the connector characters listed above.
fn is_connector(c: char) -> bool {
    CONNECTORS.contains(&c)
}

/// ASCII punctuation that always ends a word (`title:phrase`, `a@b`, `1+2`, `x?`).
///
/// Unicode word segmentation keeps some of those glued to their letters, and they must
/// never become part of a term. Connectors are the exception: they end a *piece* but
/// keep the identifier run (`ms-xhelp` → `msxhelp`, `ms`, `xhelp`).
fn is_hard_separator(c: char) -> bool {
    c.is_ascii_punctuation() && !is_connector(c)
}

/// Chunks of a word-bound segment, split at hard separators.
///
/// Each entry is `(byte start, byte end, char start, char end)`. Chunks without a single
/// alphanumeric character are dropped (they can only be combining marks).
fn chunks(seg: &str, seg_start: usize) -> Vec<(usize, usize, usize, usize)> {
    let mut out = Vec::new();
    let mut byte_start = 0usize;
    let mut char_start = seg_start;
    let mut char_index = seg_start;
    let mut has_alnum = false;
    for (byte, c) in seg.char_indices() {
        if is_hard_separator(c) {
            if has_alnum {
                out.push((byte_start, byte, char_start, char_index));
            }
            byte_start = byte + c.len_utf8();
            char_start = char_index + 1;
            has_alnum = false;
        } else {
            has_alnum |= c.is_alphanumeric();
        }
        char_index += 1;
    }
    if has_alnum {
        out.push((byte_start, seg.len(), char_start, char_index));
    }
    out
}

/// True for Han, Hiragana and Katakana code points.
///
/// Those are indexed one character per token (they are not written with spaces), plus
/// a joined token for the run.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3041..=0x30FF        // Hiragana + Katakana
        | 0x31F0..=0x31FF      // Katakana phonetic extensions
        | 0x3400..=0x4DBF      // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF      // CJK Unified Ideographs
        | 0xF900..=0xFAFF      // CJK Compatibility Ideographs
        | 0x20000..=0x2FA1F)   // CJK Unified Ideographs Extensions B–F
}

/// True when every character of `text` is CJK (and `text` is not empty).
pub fn is_cjk_text(text: &str) -> bool {
    let mut any = false;
    for c in text.chars() {
        if !is_cjk(c) {
            return false;
        }
        any = true;
    }
    any
}

/// One piece of a word: a token candidate that still belongs to a "run" of pieces glued
/// together by connectors (`ms` + `xhelp` in `ms-xhelp`).
///
/// `extra` marks the pieces produced by the identifier heuristic below; they share the
/// run but are left out of the joined token.
struct Piece {
    text: String,
    start: usize,
    end: usize,
    run: usize,
    extra: bool,
}

impl Piece {
    fn chars(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// Split one chunk into pieces (camelCase, digits, CJK, connectors).
///
/// A connector ends the current piece but keeps the pieces in the same *run*, so the
/// caller can also emit the joined identifier form.
fn push_pieces(seg: &str, seg_start: usize, run: usize, out: &mut Vec<Piece>) {
    let chars: Vec<char> = seg.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if is_cjk(c) {
            out.push(Piece {
                text: c.to_lowercase().collect(),
                start: seg_start + i,
                end: seg_start + i + 1,
                run,
                extra: false,
            });
            i += 1;
            continue;
        }
        if is_connector(c) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() {
            let prev = chars[j - 1];
            let cur = chars[j];
            let boundary = is_cjk(cur)
                || is_connector(cur)
                || ((prev.is_lowercase() || prev.is_numeric()) && cur.is_uppercase())
                || (prev.is_uppercase()
                    && cur.is_uppercase()
                    && chars.get(j + 1).is_some_and(|n| n.is_lowercase()))
                || (prev.is_alphabetic() && cur.is_numeric())
                || (prev.is_numeric() && cur.is_alphabetic());
            if boundary {
                break;
            }
            j += 1;
        }
        let text: String = chars[i..j].iter().collect::<String>().to_lowercase();
        if !text.is_empty() {
            out.push(Piece {
                text,
                start: seg_start + i,
                end: seg_start + j,
                run,
                extra: false,
            });
        }
        i = j;
    }
}

/// MSHC identifier namespaces (`ms-xhelp`, `x-topic`, `n-glossary`).
///
/// A *short* piece followed by a connector and a longer all-lowercase piece is split
/// after its leading consonant as well, so searching for the words `help` or `x` finds
/// `ms-xhelp`. Requiring a short preceding piece keeps normal hyphenated words
/// (`user-friendly`, `cross-platform`) intact.
fn namespace_extras(pieces: &[Piece]) -> Vec<Piece> {
    let mut extras = Vec::new();
    for (k, piece) in pieces.iter().enumerate() {
        let glued = match k.checked_sub(1) {
            Some(prev) => pieces[prev].chars() <= 3 && pieces[prev].end < piece.start,
            None => false,
        };
        if !glued || piece.chars() < 4 {
            continue;
        }
        let chars: Vec<char> = piece.text.chars().collect();
        let (c0, c1, c2) = (chars[0], chars[1], chars[2]);
        let is_namespace = c0.is_ascii_lowercase()
            && !"aeiou".contains(c0)
            && c1.is_ascii_lowercase()
            && c2.is_ascii_lowercase();
        if !is_namespace {
            continue;
        }
        extras.push(Piece {
            text: chars[0].to_string(),
            start: piece.start,
            end: piece.start + 1,
            run: piece.run,
            extra: true,
        });
        extras.push(Piece {
            text: chars[1..].iter().collect(),
            start: piece.start + 1,
            end: piece.end,
            run: piece.run,
            extra: true,
        });
    }
    extras
}

/// Split `text` into pieces, grouping connector-glued (and adjacent CJK) pieces into
/// runs that additionally yield a joined token.
fn scan(text: &str) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut run = 0usize;
    let mut run_open = false;
    let mut prev_connector = false;
    let mut prev_cjk = false;
    let mut run_chars = 0usize;
    let mut char_pos = 0usize;
    for (_, seg) in text.split_word_bound_indices() {
        let seg_start = char_pos;
        char_pos += seg.chars().count();
        if !seg.chars().any(|c| c.is_alphanumeric()) {
            let connector_only = !seg.is_empty() && seg.chars().all(is_connector);
            if connector_only && run_open {
                prev_connector = true;
            } else {
                prev_connector = false;
                prev_cjk = false;
                run_open = false;
                run_chars = 0;
            }
            continue;
        }
        for (byte_start, byte_end, start, end) in chunks(seg, seg_start) {
            let chunk = &seg[byte_start..byte_end];
            let chunk_cjk = is_cjk_text(chunk);
            let continue_run = run_open
                && (prev_connector
                    || (prev_cjk && chunk_cjk && run_chars + (end - start) <= MAX_CJK_RUN));
            if !continue_run {
                run += 1;
                run_chars = 0;
            }
            run_open = true;
            prev_connector = false;
            prev_cjk = chunk_cjk;
            run_chars += end - start;
            push_pieces(chunk, start, run, &mut out);
        }
    }
    // Second pass: identifier namespaces work across word-bound pieces (`ms` + `-` +
    // `xhelp`), so the extra pieces have to be derived from whole runs.
    let mut result: Vec<Piece> = Vec::with_capacity(out.len() + 8);
    let mut iter = out.into_iter().peekable();
    while let Some(first) = iter.next() {
        let run = first.run;
        let mut group = vec![first];
        while iter.peek().is_some_and(|p| p.run == run) {
            if let Some(next) = iter.next() {
                group.push(next);
            }
        }
        let extras = namespace_extras(&group);
        result.append(&mut group);
        result.extend(extras);
    }
    result
}

/// Emit the joined token of every multi-piece run, then the pieces themselves.
fn flatten(pieces: Vec<Piece>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(pieces.len() + 4);
    let mut i = 0usize;
    while i < pieces.len() {
        let run = pieces[i].run;
        let mut j = i + 1;
        while j < pieces.len() && pieces[j].run == run {
            j += 1;
        }
        let span = &pieces[i..j];
        let plain = span.iter().filter(|p| !p.extra).count();
        if plain > 1 {
            let mut joined = String::new();
            for p in span.iter().filter(|p| !p.extra) {
                joined.push_str(&p.text);
            }
            let first = span.first().expect("span is never empty");
            let last = span.last().expect("span is never empty");
            out.push(Token {
                text: joined,
                start: first.start,
                end: last.end,
            });
        }
        for p in span {
            out.push(Token {
                text: p.text.clone(),
                start: p.start,
                end: p.end,
            });
        }
        i = j;
    }
    out
}

/// Tokenize `text` into searchable tokens, keeping char ranges.
///
/// - `GetWindowRect` yields `getwindowrect`, `get`, `window`, `rect`.
/// - `ms-xhelp` yields `msxhelp`, `ms`, `xhelp`, `x`, `help`.
/// - Han/Hiragana/Katakana characters become one token each (+ a joined token).
/// - Numbers are kept (`windows 11` → `windows`, `11`; `3.14` → `314`, `3`, `14`).
pub fn tokenize(text: &str) -> Vec<Token> {
    flatten(scan(text))
}

/// Tokenize `text` into *primary* tokens: no joined identifier forms, one token per word
/// piece. Used for phrases, which must line up with the token positions inside a field
/// (the joined form of `GetWindowRect` sits on the position before `get`).
pub fn tokenize_words(text: &str) -> Vec<Token> {
    scan(text)
        .into_iter()
        .map(|p| Token {
            text: p.text,
            start: p.start,
            end: p.end,
        })
        .collect()
}

/// Normalized token texts of `text` (see [`tokenize`]).
pub fn words(text: &str) -> Vec<String> {
    tokenize(text).into_iter().map(|t| t.text).collect()
}

/// Lowercase `text` and return a map from lowered char index → original char index.
///
/// Unicode lowercasing can change the number of characters (`İ` → `i̇`), so matches
/// must be searched in the lowered string and reported as ranges of the original one.
pub fn lowercase_with_map(text: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.chars().count());
    for (i, c) in text.chars().enumerate() {
        for lc in c.to_lowercase() {
            out.push(lc);
            map.push(i);
        }
    }
    (out, map)
}

/// Lowercase `text` (simple Unicode case folding).
pub fn fold(text: &str) -> String {
    text.to_lowercase()
}

/// English stopwords, used **only** to suppress scoring — postings are always kept so
/// that phrases containing stopwords still match.
pub const STOPWORDS: [&str; 74] = [
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be", "been",
    "but", "by", "can", "could", "did", "do", "does", "for", "from", "had", "has", "have", "he",
    "her", "here", "his", "how", "i", "if", "in", "into", "is", "it", "its", "may", "me", "might",
    "more", "most", "must", "my", "no", "not", "of", "on", "one", "only", "or", "other", "our",
    "out", "over", "same", "she", "should", "so", "some", "such", "than", "that", "the", "their",
    "them", "then", "there", "these", "they", "this", "to", "was", "were",
];

/// True when `word` is an English stopword (case-insensitive).
pub fn is_stopword(word: &str) -> bool {
    let lower = word.to_lowercase();
    STOPWORDS.binary_search(&lower.as_str()).is_ok()
}

/// Stem a single word with the English Porter stemmer.
///
/// Convenience wrapper for callers outside this crate; the index keeps a single stemmer
/// alive for the duration of a build.
pub fn stem_term(word: &str) -> String {
    Stemmer::create(Algorithm::English).stem(word).into_owned()
}

/// Stem `word` when `stem` is true (borrowing when no stemming is configured).
/// Stem `word` when `stem` is true, borrowing otherwise (kept for callers that build their
/// own index; the search index keeps one stemmer alive for a whole build).
#[allow(dead_code)]
pub(crate) fn stem_cow(word: &str, stem: bool) -> Cow<'_, str> {
    if stem {
        Stemmer::create(Algorithm::English).stem(word)
    } else {
        Cow::Borrowed(word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(text: &str) -> Vec<String> {
        tokenize(text).into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn camel_case_is_split_and_joined() {
        assert_eq!(
            texts("GetWindowRect"),
            vec!["getwindowrect", "get", "window", "rect"]
        );
    }

    #[test]
    fn snake_case_is_split_and_joined() {
        assert_eq!(
            texts("snake_case_name"),
            vec!["snakecasename", "snake", "case", "name"]
        );
    }

    #[test]
    fn ms_xhelp_identifier() {
        let t = texts("ms-xhelp");
        for want in ["msxhelp", "ms", "xhelp", "x", "help"] {
            assert!(t.contains(&want.to_string()), "{want} missing from {t:?}");
        }
    }

    #[test]
    fn ordinary_hyphenated_words_are_not_over_split() {
        assert_eq!(
            texts("user-friendly"),
            vec!["userfriendly", "user", "friendly"],
            "namespace heuristic must not fire for long prefixes"
        );
    }

    #[test]
    fn punctuation_never_glues_words_together() {
        assert_eq!(texts("title:phrase"), vec!["title", "phrase"]);
        assert_eq!(texts("id=abc"), vec!["id", "abc"]);
        assert_eq!(texts("a@b"), vec!["a", "b"]);
        assert_eq!(texts("(foo)"), vec!["foo"]);
        assert_eq!(texts("ms-xhelp:///?Id=abc"), vec![
            "msxhelp",
            "ms",
            "xhelp",
            "x",
            "help",
            "id",
            "abc"
        ]);
    }

    #[test]
    fn words_has_no_joined_forms() {
        let t: Vec<String> = tokenize_words("GetWindowRect")
            .into_iter()
            .map(|t| t.text)
            .collect();
        assert_eq!(t, vec!["get", "window", "rect"]);
    }

    #[test]
    fn char_ranges_point_at_the_original_text() {
        let src = "Run GetWindowRect(0xC0, \"ms-xhelp\") please";
        let chars: Vec<char> = src.chars().collect();
        for token in tokenize(src) {
            let original: String = chars[token.start..token.end].iter().collect();
            assert!(!original.is_empty(), "empty token {token:?}");
            // joined identifiers also span the connectors they were glued by
            let collapsed: String = original
                .to_lowercase()
                .chars()
                .filter(|c| !"-_.'/".contains(*c))
                .collect();
            assert_eq!(
                collapsed, token.text,
                "range {token:?} does not cover its text in {src:?}"
            );
        }
    }

    #[test]
    fn cjk_is_one_token_per_character_plus_joined() {
        assert_eq!(texts("中文文档"), vec!["中文文档", "中", "文", "文", "档"]);
        let mixed = texts("setup 安装 guide");
        for want in ["setup", "安装", "安", "装", "guide"] {
            assert!(mixed.contains(&want.to_string()), "{want} in {mixed:?}");
        }
        let kana = texts("ひらがな カタカナ");
        for want in ["ひらがな", "ひ", "カタカナ", "カ"] {
            assert!(kana.contains(&want.to_string()), "{want} in {kana:?}");
        }
    }

    #[test]
    fn is_cjk_text_detects_scripts() {
        assert!(is_cjk_text("中文"));
        assert!(is_cjk_text("ひらがな"));
        assert!(!is_cjk_text("中文abc"));
        assert!(!is_cjk_text(""));
        assert!(!is_cjk_text("latin"));
    }

    #[test]
    fn numbers_are_kept_and_split_from_letters() {
        assert_eq!(texts("windows11"), vec!["windows11", "windows", "11"]);
        assert_eq!(texts("3.14"), vec!["314", "3", "14"]);
        assert_eq!(texts("100%"), vec!["100"]);
        assert_eq!(texts("versions 2"), vec!["versions", "2"]);
    }

    #[test]
    fn uppercase_acronyms_split_before_lowercase_run() {
        assert_eq!(texts("HTTPServer"), vec!["httpserver", "http", "server"]);
    }

    #[test]
    fn case_folding_is_unicode_aware() {
        assert_eq!(texts("CAFÉ"), vec!["café"]);
        assert_eq!(texts("СТРАНИЦА"), vec!["страница"]);
    }

    #[test]
    fn empty_and_punctuation_only_inputs_yield_no_tokens() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   ").is_empty());
        assert!(tokenize("!!! --- ???").is_empty());
    }

    #[test]
    fn positions_put_the_joined_form_first() {
        let t = tokenize("GetWindowRect");
        let positions: Vec<&str> = t.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(positions, vec!["getwindowrect", "get", "window", "rect"]);
    }

    #[test]
    fn stopword_list_is_sorted_and_recognises_words() {
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, STOPWORDS.to_vec(), "STOPWORDS must stay sorted");
        assert!(is_stopword("The"));
        assert!(is_stopword("and"));
        assert!(!is_stopword("install"));
    }

    #[test]
    fn lowercase_map_points_back_at_the_original() {
        let (lower, map) = lowercase_with_map("AİB");
        assert_eq!(lower.chars().count(), map.len());
        assert_eq!(map, vec![0, 1, 1, 2]);
    }

    #[test]
    fn stemming_is_applied_on_request() {
        assert_eq!(stem_term("installation"), stem_term("installing"));
        assert_eq!(stem_cow("install", false), "install");
    }

    #[test]
    fn no_panic_on_odd_input() {
        for src in [
            "\u{feff}bom",
            "\u{0}\u{1}\u{2}",
            "a\u{0301}\u{0301}",
            "😀 emoji 🎉",
            "\u{202e}reversed",
            "::",
            "://",
            &"x".repeat(10_000),
        ] {
            let _ = tokenize(src);
            let _ = tokenize_words(src);
            let _ = words(src);
            let _ = lowercase_with_map(src);
        }
    }
}
