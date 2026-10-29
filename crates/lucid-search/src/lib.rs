#![forbid(unsafe_code)]
//! # lucid-search
//!
//! The ranked full-text engine behind Lucid's search pane (see `docs/DESIGN.md` §4).
//!
//! * **Fast**: an inverted index with BM25F scoring, sub-millisecond queries on books of a
//!   few thousand topics, and a persisted index so reopening a package is instant.
//! * **Typing-friendly**: prefix matching, live queries, and a fallback chain
//!   (exact → prefix → fuzzy → spelling suggestions) so the user never sees a dead end.
//! * **Honest snippets**: highlight ranges are character indices into the returned snippet,
//!   so a GUI can highlight them without re-searching.
//! * **Content-blind**: it knows about documents and fields, nothing about help formats.
//!
//! ```
//! use lucid_search::{DocInput, IndexBuilder, SearchOptions};
//!
//! let mut builder = IndexBuilder::default();
//! builder.add(DocInput {
//!     title: "Searching",
//!     headings: "Query syntax",
//!     keywords: "search, query",
//!     body: "Phrase search requires adjacency inside one field.",
//!     code: "",
//!     path: "Guides › Searching",
//!     weight: 1.0,
//! });
//! let index = builder.build();
//!
//! let results = index.search("phrase search", &SearchOptions::default());
//! assert_eq!(results.hits[0].title, "Searching");
//! assert!(!results.hits[0].snippet_ranges.is_empty());
//! ```

pub mod error;
pub mod index;
pub mod query;
pub mod rank;
pub mod snippet;
pub mod tokenizer;

use serde::{Deserialize, Serialize};

pub use error::{Error, Result};
pub use index::{Index, IndexBuilder};
pub use query::{field_mask_for, parse as parse_query, ParsedQuery, VariantKind};
pub use snippet::{make as make_snippet, ranges_in as find_all, SnippetOutput, SnippetRequest};
pub use tokenizer::{is_cjk_text, is_stopword as tokenizer_is_stopword, tokenize, Token};

/// Document identifier inside an index (assigned in insertion order).
pub type DocId = u32;

/// Field indices used by masks and weights.
pub const FIELD_TITLE: u8 = 0;
pub const FIELD_HEADINGS: u8 = 1;
pub const FIELD_KEYWORDS: u8 = 2;
pub const FIELD_BODY: u8 = 3;
pub const FIELD_CODE: u8 = 4;

/// Number of indexed fields.
pub const NUM_FIELDS: usize = 5;

/// Field names, for diagnostics and the search pane's filter chips.
pub const FIELD_NAMES: [&str; NUM_FIELDS] = ["title", "headings", "keywords", "body", "code"];

/// Magic bytes at the start of a persisted index.
pub const MAGIC: &[u8; 8] = b"LUCIDIDX";

/// Persisted index format version (bumped when parsing semantics change).
pub const FORMAT_VERSION: u32 = 1;

/// A document to index.
#[derive(Debug, Clone, Copy)]
pub struct DocInput<'a> {
    /// Topic title — the strongest field.
    pub title: &'a str,
    /// Concatenated headings and section titles.
    pub headings: &'a str,
    /// Index keywords and F1 keywords.
    pub keywords: &'a str,
    /// Visible body text.
    pub body: &'a str,
    /// Code samples.
    pub code: &'a str,
    /// Human breadcrumb ("Guides › Searching › Queries"), shown in results.
    pub path: &'a str,
    /// Document prior; `1.0` is neutral. Used to favour, say, shallower TOC nodes.
    pub weight: f32,
}

/// Index build options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexOptions {
    /// Apply the English stemmer (`installing` finds `install`).
    pub stem: bool,
    /// Ignore stopwords when scoring (they stay searchable and phrases keep working).
    pub stopwords: bool,
    /// Keep each document's text so [`Index::search`] can build snippets itself.
    pub keep_text: bool,
    /// Keep token positions so phrases require real adjacency.
    pub keep_positions: bool,
    /// Target snippet length in characters.
    pub max_snippet_chars: usize,
}

impl Default for IndexOptions {
    fn default() -> Self {
        IndexOptions {
            stem: true,
            stopwords: true,
            keep_text: true,
            keep_positions: true,
            max_snippet_chars: 240,
        }
    }
}

impl IndexOptions {
    /// Minimal footprint: no stored text, no positions — for very large content sets where
    /// the application builds snippets on demand (see `make_snippet`).
    pub fn cheap() -> Self {
        IndexOptions {
            keep_text: false,
            keep_positions: false,
            ..Default::default()
        }
    }
}

/// Metadata of an indexed document.
#[derive(Debug, Clone, PartialEq)]
pub struct DocMeta {
    pub title: String,
    pub path: String,
    pub weight: f32,
}

/// Result ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Relevance,
    Title,
    Path,
}

/// When to use fuzzy matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FuzzyMode {
    Never,
    /// Use fuzzy terms only when the exact terms matched nothing (default).
    #[default]
    Auto,
    /// Also add fuzzy expansions next to exact matches.
    Always,
}

/// What a suggestion is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionKind {
    /// A near-miss vocabulary term (`insttall` → `install`).
    Spelling,
    /// A prefix completion.
    Term,
}

/// One suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub text: String,
    pub kind: SuggestionKind,
}

/// Search options.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Maximum number of hits (`0` = unlimited).
    pub limit: usize,
    pub sort: Sort,
    /// Bitmask of fields to search (default: all).
    pub fields: u8,
    pub fuzzy: FuzzyMode,
    /// Treat the last positive term as a prefix (set while the user is typing).
    pub prefix_last_term: bool,
    /// Restrict results to these documents (`None` = everything).
    pub scope: Option<Vec<DocId>>,
    /// Build snippets and highlight ranges.
    pub snippet: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            limit: 200,
            sort: Sort::Relevance,
            fields: (1 << NUM_FIELDS) - 1,
            fuzzy: FuzzyMode::Auto,
            prefix_last_term: false,
            scope: None,
            snippet: true,
        }
    }
}

/// One search result.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub doc: DocId,
    pub score: f32,
    pub title: String,
    /// Breadcrumb path of the topic.
    pub path: String,
    /// Snippet around the best match (empty when the index kept no text).
    pub snippet: String,
    /// Character ranges inside `snippet` to highlight.
    pub snippet_ranges: Vec<std::ops::Range<usize>>,
    /// Character ranges inside `title` to highlight.
    pub title_ranges: Vec<std::ops::Range<usize>>,
    /// Bitmask of fields that matched ([`FIELD_NAMES`]).
    pub fields: u8,
}

impl Hit {
    /// Names of the fields that matched.
    pub fn matched_fields(&self) -> Vec<&'static str> {
        (0..NUM_FIELDS)
            .filter(|field| self.fields & (1 << field) != 0)
            .map(|field| FIELD_NAMES[field])
            .collect()
    }
}

/// Results of a query.
#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    pub hits: Vec<Hit>,
    /// Documents that matched, before the limit was applied.
    pub total_matched: usize,
    pub took_us: u64,
    /// Filled when nothing matched (or when fuzzy suggestions were requested).
    pub suggestions: Vec<Suggestion>,
    /// Effective query terms, for "find in topic" chaining.
    pub terms: Vec<String>,
    /// True when the query had more than 64 conditions and the extras were ignored.
    pub truncated_groups: bool,
}

/// Index statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub docs: usize,
    pub terms: usize,
    pub postings: usize,
    /// Estimated in-memory footprint in bytes.
    pub bytes: usize,
}

/// Very small English stopword list, used to suppress scoring (never to drop postings).
pub const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be",
    "before", "but", "by", "can", "could", "did", "do", "does", "for", "from", "has", "have",
    "he", "if", "in", "into", "is", "it", "its", "may", "might", "more", "most", "must", "no",
    "not", "of", "on", "only", "or", "other", "over", "own", "same", "should", "so", "some",
    "such", "than", "that", "the", "their", "them", "then", "there", "these", "they", "this",
    "to", "too", "very", "was", "we", "were", "will", "with", "would", "you", "your",
];

/// Is `term` a scoring stopword?
pub fn is_stopword(term: &str) -> bool {
    STOPWORDS.binary_search(&term).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopword_list_is_sorted_for_binary_search() {
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, STOPWORDS, "STOPWORDS must stay sorted");
        assert!(is_stopword("the"));
        assert!(!is_stopword("install"));
    }

    #[test]
    fn field_names_match_the_constants() {
        assert_eq!(FIELD_NAMES[FIELD_TITLE as usize], "title");
        assert_eq!(FIELD_NAMES[FIELD_CODE as usize], "code");
        assert_eq!(NUM_FIELDS, FIELD_NAMES.len());
    }

    #[test]
    fn default_options_are_the_documented_ones() {
        let options = SearchOptions::default();
        assert_eq!(options.limit, 200);
        assert_eq!(options.sort, Sort::Relevance);
        assert_eq!(options.fields, 0b11111);
        assert_eq!(options.fuzzy, FuzzyMode::Auto);
        assert!(options.snippet);

        let index_options = IndexOptions::default();
        assert!(index_options.stem);
        assert!(index_options.keep_text);
        assert!(index_options.keep_positions);
        assert_eq!(index_options.max_snippet_chars, 240);
    }

    #[test]
    fn matched_field_names() {
        let hit = Hit {
            doc: 0,
            score: 1.0,
            title: String::new(),
            path: String::new(),
            snippet: String::new(),
            snippet_ranges: Vec::new(),
            title_ranges: Vec::new(),
            fields: (1 << FIELD_TITLE) | (1 << FIELD_BODY),
        };
        assert_eq!(hit.matched_fields(), vec!["title", "body"]);
    }
}
