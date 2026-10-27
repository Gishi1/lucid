//! The query language (see `docs/DESIGN.md` §4.1).
//!
//! ```text
//! install                     → AND of one term
//! phrase search               → AND of two terms
//! "phrase search"             → exact adjacency (same field)
//! sear* / (prefix_last_term)  → prefix match
//! serach~                     → fuzzy match (edit distance ≤ 2)
//! -legacy                     → exclusion
//! search OR index             → union
//! title:install kw:"getting started"   → field filters
//! ```
//!
//! Parsing never fails: anything unparseable becomes an ordinary term, so a half-typed
//! query behaves sensibly while you type.

use crate::tokenizer;
use crate::{FIELD_BODY, FIELD_CODE, FIELD_HEADINGS, FIELD_KEYWORDS, FIELD_TITLE, NUM_FIELDS};

/// One alternative inside a query group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantKind {
    Term,
    Phrase,
}

/// A matchable alternative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub kind: VariantKind,
    /// Source text (for diagnostics and snippets).
    pub text: String,
    /// Tokens to match, in order (a phrase has more than one).
    pub terms: Vec<String>,
    /// Field bitmask this variant is restricted to (all fields by default).
    pub fields: u8,
    /// `term*` — match any term with this prefix.
    pub prefix: bool,
    /// `term~` — allow an edit distance of up to 2.
    pub fuzzy: bool,
    /// The text was quoted by the user: it stays literal, and never becomes a prefix when
    /// the search box is in "typing" mode.
    pub quoted: bool,
}

impl Variant {
    fn new(kind: VariantKind, text: String, terms: Vec<String>) -> Self {
        Variant {
            kind,
            text,
            terms,
            fields: ALL_FIELDS,
            prefix: false,
            fuzzy: false,
            quoted: false,
        }
    }

    /// Number of tokens in the variant.
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
}

/// A group: its variants are alternatives (satisfied by any of them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub variants: Vec<Variant>,
    /// `-term` — the group must *not* match.
    pub negated: bool,
}

/// A parsed query: groups are ANDed, variants inside a group are ORed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedQuery {
    pub raw: String,
    pub groups: Vec<Group>,
}

/// Bitmask with every field set.
pub const ALL_FIELDS: u8 = (1 << NUM_FIELDS) - 1;

/// Field name aliases accepted by `field:` filters.
pub fn field_mask_for(name: &str) -> Option<u8> {
    Some(match name.to_ascii_lowercase().as_str() {
        "title" | "t" => 1 << FIELD_TITLE,
        "heading" | "headings" | "h" => 1 << FIELD_HEADINGS,
        "kw" | "keyword" | "keywords" | "k" => 1 << FIELD_KEYWORDS,
        "body" | "b" | "text" => 1 << FIELD_BODY,
        "code" | "c" | "snippet" => 1 << FIELD_CODE,
        "id" | "path" | "p" => 1 << FIELD_TITLE,
        _ => return None,
    })
}

impl ParsedQuery {
    /// Every term mentioned by any variant (positive or negative), for highlighting.
    pub fn all_terms(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for group in &self.groups {
            for variant in &group.variants {
                for term in &variant.terms {
                    if !out.contains(term) {
                        out.push(term.clone());
                    }
                }
            }
        }
        out
    }

    /// Terms of the positive (required) groups.
    pub fn positive_terms(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for group in self.groups.iter().filter(|g| !g.negated) {
            for variant in &group.variants {
                for term in &variant.terms {
                    if !out.contains(term) {
                        out.push(term.clone());
                    }
                }
            }
        }
        out
    }

    /// Positive variants, in query order.
    pub fn positive_variants(&self) -> impl Iterator<Item = &Variant> {
        self.groups
            .iter()
            .filter(|group| !group.negated)
            .flat_map(|group| group.variants.iter())
    }

    /// True when the query has no positive requirement (i.e. matches nothing useful).
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
            || !self
                .groups
                .iter()
                .any(|group| !group.negated && group.variants.iter().any(|v| !v.is_empty()))
    }

    /// Turn the last positive term into a prefix match (search-as-you-type behaviour).
    ///
    /// Quoted text and phrases stay literal — the user was explicit about those.
    pub fn prefix_last_term(&mut self) {
        for group in self.groups.iter_mut().rev() {
            if group.negated {
                continue;
            }
            if let Some(variant) = group.variants.iter_mut().rev().find(|v| !v.is_empty()) {
                if variant.kind == VariantKind::Term && !variant.quoted {
                    variant.prefix = true;
                }
                return;
            }
        }
    }

    /// Restrict every variant to the given field mask (intersection).
    pub fn restrict_fields(&mut self, mask: u8) {
        for group in &mut self.groups {
            for variant in &mut group.variants {
                variant.fields &= mask;
            }
        }
    }
}

/// Parse a query string.
pub fn parse(raw: &str) -> ParsedQuery {
    let mut query = ParsedQuery {
        raw: raw.to_owned(),
        groups: Vec::new(),
    };
    let mut current: Option<Group> = None;
    let mut pending_or = false;

    for atom in lex(raw) {
        if atom.text.trim().is_empty() {
            continue;
        }
        if atom.text == "OR" && !atom.quoted {
            pending_or = true;
            continue;
        }
        let variant = match build_variant(&atom) {
            Some(variant) => variant,
            None => continue,
        };

        if atom.negated {
            // Flush the pending group first so group order matches query order.
            if let Some(group) = current.take() {
                query.groups.push(group);
            }
            query.groups.push(Group {
                variants: vec![variant],
                negated: true,
            });
            pending_or = false;
            continue;
        }

        if pending_or && current.is_some() {
            if let Some(group) = current.as_mut() {
                group.variants.push(variant);
            }
        } else {
            if let Some(group) = current.take() {
                query.groups.push(group);
            }
            current = Some(Group {
                variants: vec![variant],
                negated: false,
            });
        }
        pending_or = false;
    }
    if let Some(group) = current.take() {
        query.groups.push(group);
    }
    query
}

/// One lexical atom of the query.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Atom {
    text: String,
    quoted: bool,
    negated: bool,
    field: Option<u8>,
}

/// Split a query into atoms, honouring quotes, `-` negation and `field:` prefixes.
fn lex(raw: &str) -> Vec<Atom> {
    let mut atoms = Vec::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        // Skip separators.
        if chars[index].is_whitespace() {
            index += 1;
            continue;
        }
        let mut negated = false;
        if chars[index] == '-' && index + 1 < chars.len() && !chars[index + 1].is_whitespace() {
            negated = true;
            index += 1;
        }
        // Field prefix: `name:` right before a word or a quote.
        let mut field = None;
        let save = index;
        let mut probe = String::new();
        while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
            probe.push(chars[index]);
            index += 1;
        }
        if index < chars.len() && chars[index] == ':' && !probe.is_empty() {
            if let Some(mask) = field_mask_for(&probe) {
                field = Some(mask);
                index += 1;
            } else {
                // Not a field: rewind, the colon belongs to the term (URLs, code names).
                index = save;
            }
        } else {
            index = save;
        }

        if index < chars.len() && chars[index] == '"' {
            index += 1;
            let mut text = String::new();
            while index < chars.len() && chars[index] != '"' {
                text.push(chars[index]);
                index += 1;
            }
            index += 1; // closing quote (or end of input)
            atoms.push(Atom {
                text,
                quoted: true,
                negated,
                field,
            });
            continue;
        }

        let mut text = String::new();
        while index < chars.len() && !chars[index].is_whitespace() {
            text.push(chars[index]);
            index += 1;
        }
        if !text.is_empty() {
            atoms.push(Atom {
                text,
                quoted: false,
                negated,
                field,
            });
        }
    }
    atoms
}

/// Turn an atom into a matchable variant, or `None` when it carries no searchable text.
fn build_variant(atom: &Atom) -> Option<Variant> {
    let mut text = atom.text.trim().to_owned();
    let mut prefix = false;
    let mut fuzzy = false;

    if !atom.quoted {
        if let Some(stripped) = text.strip_suffix('*') {
            prefix = true;
            text = stripped.to_owned();
        } else if let Some(stripped) = text.strip_suffix('~') {
            fuzzy = true;
            text = stripped.to_owned();
        }
    }
    if text.is_empty() {
        return None;
    }

    // The tokenizer is generative: `GetWindowRect` yields `getwindowrect` plus `get`,
    // `window`, `rect`, and CJK runs yield one token per character plus a joined token.
    //
    // * a single word (anything but quoted text or CJK) is looked up by its joined form,
    //   which is how identifiers are indexed;
    // * phrases and CJK text use the *primary* tokens, which line up with the token
    //   positions inside a field — that is what makes adjacency checks meaningful.
    let single_word = text.split_whitespace().count() == 1;
    let contains_cjk = text.chars().any(tokenizer::is_cjk);
    let terms: Vec<String> = if !atom.quoted && single_word && !contains_cjk {
        tokenizer::tokenize(&text)
            .into_iter()
            .next()
            .map(|token| token.text)
            .into_iter()
            .collect()
    } else {
        tokenizer::tokenize_words(&text)
            .into_iter()
            .map(|token| token.text)
            .collect()
    };
    if terms.is_empty() {
        // Something like `title:"..."` with no words, or pure punctuation.
        return None;
    }

    let kind = if terms.len() > 1 {
        VariantKind::Phrase
    } else {
        VariantKind::Term
    };
    let mut variant = Variant::new(kind, text, terms);
    variant.prefix = prefix;
    variant.fuzzy = fuzzy;
    variant.quoted = atom.quoted;
    if let Some(field) = atom.field {
        variant.fields = field;
    }
    Some(variant)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variants(query: &ParsedQuery) -> Vec<(&str, bool, bool, u8, VariantKind)> {
        query
            .groups
            .iter()
            .flat_map(|group| {
                group
                    .variants
                    .iter()
                    .map(move |variant| {
                        (
                            variant.text.as_str(),
                            variant.prefix,
                            group.negated,
                            variant.fields,
                            variant.kind,
                        )
                    })
            })
            .collect()
    }

    #[test]
    fn and_of_terms() {
        let query = parse("install lucid");
        assert_eq!(query.groups.len(), 2);
        assert_eq!(query.positive_terms(), vec!["install", "lucid"]);
    }

    #[test]
    fn phrases_keep_all_terms_in_order() {
        let query = parse("\"phrase search\"");
        assert_eq!(query.groups.len(), 1);
        let variant = &query.groups[0].variants[0];
        assert_eq!(variant.kind, VariantKind::Phrase);
        assert_eq!(variant.terms, vec!["phrase", "search"]);
    }

    #[test]
    fn prefix_and_fuzzy_markers() {
        let query = parse("sear* serach~");
        let variants = variants(&query);
        assert!(variants[0].1, "prefix flag");
        assert_eq!(variants[0].0, "sear");
        assert_eq!(query.groups[1].variants[0].text, "serach");
        assert!(query.groups[1].variants[0].fuzzy);
    }

    #[test]
    fn negation_is_its_own_group() {
        let query = parse("search -legacy");
        assert_eq!(query.groups.len(), 2);
        assert!(!query.groups[0].negated);
        assert!(query.groups[1].negated);
        assert_eq!(query.positive_terms(), vec!["search"]);
        assert_eq!(query.all_terms(), vec!["search", "legacy"]);
    }

    #[test]
    fn or_joins_variants_into_one_group() {
        let query = parse("search OR index OR lookup");
        assert_eq!(query.groups.len(), 1);
        assert_eq!(query.groups[0].variants.len(), 3);
        // `or` in lowercase is just a term.
        let query = parse("search or index");
        assert_eq!(query.groups.len(), 3);
    }

    #[test]
    fn field_filters() {
        let query = parse("title:install body:\"getting started\" kw:setup");
        let variants = variants(&query);
        assert_eq!(variants[0].4, VariantKind::Term);
        assert_eq!(variants[0].3, 1 << FIELD_TITLE);
        assert_eq!(variants[1].4, VariantKind::Phrase);
        assert_eq!(variants[1].3, 1 << FIELD_BODY);
        assert_eq!(variants[2].3, 1 << FIELD_KEYWORDS);
        // Unknown prefixes stay part of the term.
        let query = parse("https://example.com");
        assert_eq!(query.groups.len(), 1);
        assert!(query.groups[0].variants[0].text.contains("example.com"));
    }

    #[test]
    fn prefix_last_term_helper() {
        let mut query = parse("install luc");
        query.prefix_last_term();
        let variants = variants(&query);
        assert!(!variants[0].1);
        assert!(variants[1].1);

        // Quoted phrases are never turned into prefixes.
        let mut query = parse("install \"luc\"");
        query.prefix_last_term();
        assert!(!query.groups[1].variants[0].prefix);
    }

    #[test]
    fn field_restriction_intersects() {
        let mut query = parse("title:install");
        query.restrict_fields(1 << FIELD_BODY);
        assert_eq!(query.groups[0].variants[0].fields, 0);
    }

    #[test]
    fn degenerate_queries_are_empty_not_panics() {
        for raw in ["", "   ", "-", "*", "~", "\"\"", "OR", "title:", "-OR x"] {
            let query = parse(raw);
            let _ = query.all_terms();
            let _ = query.is_empty();
        }
        assert!(parse("").is_empty());
        assert!(parse("   ").is_empty());
        assert!(!parse("x").is_empty());
    }

    #[test]
    fn compound_expressions_parse() {
        let query = parse("title:content \"wiki pages\" -legacy archive OR lookup");
        assert_eq!(query.groups.len(), 4, "one group per positive atom, plus the negation");
        assert!(query.groups[2].negated, "the negation keeps its own group");
        assert_eq!(query.groups[3].variants.len(), 2, "OR joins into one group");
        assert_eq!(query.positive_terms().len(), 5);
        // Group order follows query order, so the UI can show the filters in the order typed.
        assert_eq!(query.groups[0].variants[0].text, "content");
        assert_eq!(query.groups[1].variants[0].text, "wiki pages");
    }
}
