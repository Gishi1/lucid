//! The inverted index and the search pipeline (`docs/DESIGN.md` §4.2–§4.4).
//!
//! Layout: a sorted term dictionary (so prefixes are a binary-search range), postings sorted
//! by document, per-field term frequencies and lengths, and — when asked for — token
//! positions so phrases can require real adjacency.
//!
//! Scoring is BM25F with the field weights in [`crate::rank`], then boosts (phrase adjacency,
//! exact title, all-terms-present) and penalties (prefix/stem/fuzzy expansion). Ties break on
//! `(score, title length, title, doc id)` so results never jitter between identical queries.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use bincode::Options;
use rust_stemmers::{Algorithm, Stemmer};
use serde::{Deserialize, Serialize};
use strsim::{jaro_winkler, levenshtein};

use crate::error::{Error, Result};
use crate::query::{self, ParsedQuery, Variant, VariantKind};
use crate::rank;
use crate::snippet;
use crate::tokenizer::{self, Token};
use crate::{
    DocId, DocInput, DocMeta, FuzzyMode, Hit, IndexOptions, IndexStats, SearchOptions,
    SearchResults, Sort, Suggestion, SuggestionKind, FIELD_CODE, FIELD_TITLE, FORMAT_VERSION,
    MAGIC, NUM_FIELDS,
};

/// Positions are encoded as `field * POSITION_STRIDE + token_index`.
pub const POSITION_STRIDE: u32 = 1 << 20;

/// One term occurrence list entry for a single document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Posting {
    pub doc: DocId,
    /// Term frequency per field.
    pub tf: [u16; NUM_FIELDS],
    /// Sorted `field * POSITION_STRIDE + token_index` values (empty when positions are off).
    pub positions: Vec<u32>,
}

impl Posting {
    fn new(doc: DocId) -> Self {
        Posting {
            doc,
            tf: [0; NUM_FIELDS],
            positions: Vec::new(),
        }
    }

    fn matches_in_fields(&self, mask: u8) -> bool {
        (0..NUM_FIELDS).any(|field| mask & (1 << field) != 0 && self.tf[field] > 0)
    }
}

/// Metadata kept for every document (plus its text when snippets are enabled).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredDoc {
    pub title: String,
    pub path: String,
    pub weight: f32,
    /// Lowercased title, for the exact-title boost.
    pub title_lower: String,
    /// Body text; `None` when the index was built with `keep_text: false`.
    pub text: Option<String>,
    /// Code text, used when the best match is inside a code sample.
    pub code: Option<String>,
}

/// An immutable, searchable index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    options: IndexOptions,
    docs: Vec<StoredDoc>,
    /// Sorted term dictionary.
    terms: Vec<String>,
    /// Postings, parallel to `terms`, sorted by document id.
    postings: Vec<Vec<Posting>>,
    field_len: Vec<[u32; NUM_FIELDS]>,
    avg_len: [f32; NUM_FIELDS],
    total_postings: usize,
}

/// Builds an [`Index`].
pub struct IndexBuilder {
    options: IndexOptions,
    stemmer: Stemmer,
    docs: Vec<StoredDoc>,
    field_len: Vec<[u32; NUM_FIELDS]>,
    postings: HashMap<String, HashMap<DocId, Posting>>,
}

impl IndexBuilder {
    /// Start a builder with the given options.
    pub fn new(options: IndexOptions) -> Self {
        IndexBuilder {
            options,
            stemmer: Stemmer::create(Algorithm::English),
            docs: Vec::new(),
            field_len: Vec::new(),
            postings: HashMap::new(),
        }
    }

    /// Number of documents added so far.
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Add a document; documents get consecutive ids starting at 0.
    pub fn add(&mut self, doc: DocInput<'_>) -> DocId {
        let id = self.docs.len() as DocId;
        let fields = [
            doc.title,
            doc.headings,
            doc.keywords,
            doc.body,
            doc.code,
        ];

        let mut lengths = [0u32; NUM_FIELDS];
        let mut per_field_tokens: Vec<Vec<Token>> = Vec::with_capacity(NUM_FIELDS);
        for text in fields {
            let tokens = tokenizer::tokenize(text);
            per_field_tokens.push(tokens);
        }
        for (field, tokens) in per_field_tokens.iter().enumerate() {
            lengths[field] = tokens.len().min(u32::MAX as usize) as u32;
        }

        for (field, tokens) in per_field_tokens.into_iter().enumerate() {
            for (position, token) in tokens.into_iter().enumerate() {
                let forms = self.forms(&token.text);
                for form in forms {
                    let entry = self
                        .postings
                        .entry(form)
                        .or_default()
                        .entry(id)
                        .or_insert_with(|| Posting::new(id));
                    entry.tf[field] = entry.tf[field].saturating_add(1);
                    if self.options.keep_positions && position < POSITION_STRIDE as usize {
                        entry
                            .positions
                            .push(field as u32 * POSITION_STRIDE + position as u32);
                    }
                }
            }
        }

        self.field_len.push(lengths);
        self.docs.push(StoredDoc {
            title: doc.title.trim().to_owned(),
            path: doc.path.to_owned(),
            weight: if doc.weight.is_finite() && doc.weight > 0.0 {
                doc.weight
            } else {
                1.0
            },
            title_lower: doc.title.trim().to_lowercase(),
            text: self.options.keep_text.then(|| doc.body.to_owned()),
            code: self.options.keep_text.then(|| doc.code.to_owned()),
        });
        id
    }

    /// The indexed forms of a token: the surface form and, optionally, its stem.
    fn forms(&self, token: &str) -> Vec<String> {
        let mut forms = vec![token.to_owned()];
        if self.options.stem && token.chars().count() > 2 {
            let stem = self.stemmer.stem(token).to_string();
            if stem != token && !stem.is_empty() {
                forms.push(stem);
            }
        }
        forms
    }

    /// Freeze the index.
    pub fn build(self) -> Index {
        let doc_count = self.docs.len().max(1);
        let mut avg_len = [0.0f32; NUM_FIELDS];
        for lengths in &self.field_len {
            for field in 0..NUM_FIELDS {
                avg_len[field] += lengths[field] as f32;
            }
        }
        for field in 0..NUM_FIELDS {
            avg_len[field] /= doc_count as f32;
        }

        let mut terms: Vec<String> = self.postings.keys().cloned().collect();
        terms.sort_unstable();
        let mut postings = Vec::with_capacity(terms.len());
        let mut total_postings = 0usize;
        for term in &terms {
            let mut list: Vec<Posting> = self
                .postings
                .get(term)
                .map(|map| map.values().cloned().collect())
                .unwrap_or_default();
            list.sort_by_key(|posting| posting.doc);
            total_postings += list.len();
            postings.push(list);
        }

        Index {
            options: self.options,
            docs: self.docs,
            terms,
            postings,
            field_len: self.field_len,
            avg_len,
            total_postings,
        }
    }
}

impl Default for IndexBuilder {
    fn default() -> Self {
        IndexBuilder::new(IndexOptions::default())
    }
}

impl Index {
    /// Build a one-document index (handy for tests and previews).
    pub fn single(doc: DocInput<'_>) -> Index {
        let mut builder = IndexBuilder::default();
        builder.add(doc);
        builder.build()
    }

    // ------------------------------------------------------------------ introspection

    pub fn doc_count(&self) -> usize {
        self.docs.len()
    }

    pub fn term_count(&self) -> usize {
        self.terms.len()
    }

    pub fn options(&self) -> &IndexOptions {
        &self.options
    }

    /// Metadata of a document.
    pub fn doc_meta(&self, id: DocId) -> Option<DocMeta> {
        self.docs.get(id as usize).map(|doc| DocMeta {
            title: doc.title.clone(),
            path: doc.path.clone(),
            weight: doc.weight,
        })
    }

    /// Stored body text, when the index kept it.
    pub fn doc_text(&self, id: DocId) -> Option<&str> {
        self.docs.get(id as usize).and_then(|doc| doc.text.as_deref())
    }

    /// Stored code text, when the index kept it.
    pub fn doc_code(&self, id: DocId) -> Option<&str> {
        self.docs.get(id as usize).and_then(|doc| doc.code.as_deref())
    }

    /// Index statistics (sizes are estimates of the in-memory footprint).
    pub fn stats(&self) -> IndexStats {
        let terms_bytes: usize = self.terms.iter().map(|term| term.len() + 24).sum();
        let docs_bytes: usize = self
            .docs
            .iter()
            .map(|doc| {
                doc.title.len()
                    + doc.path.len()
                    + doc.title_lower.len()
                    + doc.text.as_ref().map(|text| text.len()).unwrap_or(0)
                    + doc.code.as_ref().map(|code| code.len()).unwrap_or(0)
                    + 64
            })
            .sum();
        let postings_bytes = self.total_postings * (std::mem::size_of::<Posting>() + 8);
        IndexStats {
            docs: self.docs.len(),
            terms: self.terms.len(),
            postings: self.total_postings,
            bytes: terms_bytes + docs_bytes + postings_bytes,
        }
    }

    /// Vocabulary access (sorted) — used by the GUI's quick-open and by tests.
    pub fn vocabulary(&self) -> &[String] {
        &self.terms
    }

    // ------------------------------------------------------------------ search

    /// Run a query.
    pub fn search(&self, raw_query: &str, opts: &SearchOptions) -> SearchResults {
        let started = std::time::Instant::now();
        let mut results = SearchResults::default();
        if self.docs.is_empty() || raw_query.trim().is_empty() {
            results.took_us = started.elapsed().as_micros() as u64;
            return results;
        }

        let mut parsed = query::parse(raw_query);
        if opts.prefix_last_term {
            parsed.prefix_last_term();
        }
        parsed.restrict_fields(opts.fields);
        results.terms = parsed.positive_terms();

        if parsed.is_empty() {
            results.took_us = started.elapsed().as_micros() as u64;
            return results;
        }

        let stemmer = Stemmer::create(Algorithm::English);
        let plan = self.plan(&parsed, &stemmer, opts);
        let scope: Option<HashSet<DocId>> = opts
            .scope
            .as_ref()
            .map(|ids| ids.iter().copied().collect());
        let in_scope = |doc: DocId| scope.as_ref().is_none_or(|set| set.contains(&doc));

        // ---- matching + scoring: one pass over the postings of every alternative
        let mut scores: HashMap<DocId, f32> = HashMap::new();
        let mut masks: HashMap<DocId, u64> = HashMap::new();
        let mut matched_fields: HashMap<DocId, u8> = HashMap::new();
        for (condition_index, condition) in plan.conditions.iter().enumerate() {
            if condition_index >= 64 {
                results.truncated_groups = true;
                break;
            }
            let bit = 1u64 << condition_index;
            // Alternatives inside a condition are ORed: a document matching several of them
            // (say an exact term and its fuzzy neighbours) counts once, with its best score —
            // otherwise a pile of weak expansions would outrank a single exact match.
            let mut condition_scores: HashMap<DocId, f32> = HashMap::new();
            for alternative in &condition.alternatives {
                let Some(postings) = self.postings.get(alternative.term as usize) else {
                    continue;
                };
                let doc_freq = postings.len();
                let idf = rank::idf(self.docs.len(), doc_freq);
                // Per-alternative field masks: `title:a OR body:b` must not let `a` match in
                // the body just because the other alternative asked for it.
                let fields_mask = alternative.fields;
                for posting in postings {
                    if !in_scope(posting.doc) || !posting.matches_in_fields(fields_mask) {
                        continue;
                    }
                    let doc_index = posting.doc as usize;
                    if doc_index >= self.field_len.len() {
                        continue;
                    }
                    let weight = if condition.scoring {
                        rank::term_score(
                            idf,
                            &posting.tf,
                            &self.field_len[doc_index],
                            &self.avg_len,
                            fields_mask,
                        ) * alternative.penalty
                    } else {
                        0.0
                    };
                    let entry = condition_scores.entry(posting.doc).or_insert(0.0);
                    if weight > *entry {
                        *entry = weight;
                    }
                    *masks.entry(posting.doc).or_insert(0) |= bit;
                    let fields = matched_fields.entry(posting.doc).or_insert(0);
                    for field in 0..NUM_FIELDS {
                        if fields_mask & (1 << field) != 0 && posting.tf[field] > 0 {
                            *fields |= 1 << field;
                        }
                    }
                }
            }
            for (doc, best) in condition_scores {
                *scores.entry(doc).or_insert(0.0) += best;
            }
        }

        // ---- exclusions
        if !plan.excluded.is_empty() {
            let mut drop: HashSet<DocId> = HashSet::new();
            for alternative in &plan.excluded {
                if let Some(postings) = self.postings.get(alternative.term as usize) {
                    for posting in postings {
                        if posting.matches_in_fields(alternative.fields) {
                            drop.insert(posting.doc);
                        }
                    }
                }
            }
            for doc in drop {
                scores.remove(&doc);
                masks.remove(&doc);
            }
        }

        // ---- require every condition, then apply boosts
        let required: u64 = if plan.conditions.is_empty() {
            0
        } else if plan.conditions.len() >= 64 {
            u64::MAX
        } else {
            (1u64 << plan.conditions.len()) - 1
        };
        let mut candidates: Vec<(DocId, f32)> = scores
            .into_iter()
            .filter(|(doc, _)| masks.get(doc).copied().unwrap_or(0) & required == required)
            .map(|(doc, score)| (doc, score))
            .collect();

        // A phrase is a requirement, not a hint: with positions available, only documents
        // where the terms really are adjacent survive (without positions, the AND stands in).
        if !plan.phrases.is_empty() && self.options.keep_positions {
            candidates.retain(|(doc, _)| {
                plan.phrases
                    .iter()
                    .all(|phrase| self.phrase_strength(*doc, phrase) != PhraseStrength::None)
            });
        }

        if candidates.is_empty() {
            if matches!(opts.fuzzy, FuzzyMode::Auto | FuzzyMode::Always) {
                results.suggestions = self.suggest(raw_query, 5);
            }
            results.took_us = started.elapsed().as_micros() as u64;
            return results;
        }

        let query_lower = raw_query.trim().to_lowercase();
        for (doc, score) in candidates.iter_mut() {
            let index = *doc as usize;
            let doc_meta = &self.docs[index];
            let mut value = *score * doc_meta.weight;

            if plan.conditions.len() > 1 {
                value *= rank::ALL_TERMS_BONUS;
            }
            if !query_lower.is_empty() && doc_meta.title_lower == query_lower {
                value *= rank::EXACT_TITLE_BONUS;
            } else if !query_lower.is_empty() && doc_meta.title_lower.contains(&query_lower) {
                value *= 1.5;
            }
            if !plan.phrases.is_empty() {
                let mut phrase_multiplier = 1.0f32;
                for phrase in &plan.phrases {
                    match self.phrase_strength(*doc, phrase) {
                        PhraseStrength::Title => phrase_multiplier *= rank::PHRASE_TITLE_BONUS,
                        PhraseStrength::Body => phrase_multiplier *= rank::PHRASE_BODY_BONUS,
                        PhraseStrength::None => {}
                    }
                }
                value *= phrase_multiplier.min(4.0);
            }
            *score = value;
        }

        // ---- order (deterministic)
        match opts.sort {
            Sort::Relevance => {
                candidates.sort_by(|a, b| {
                    let (da, db) = (&self.docs[a.0 as usize], &self.docs[b.0 as usize]);
                    b.1.partial_cmp(&a.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| da.title.chars().count().cmp(&db.title.chars().count()))
                        .then_with(|| da.title.cmp(&db.title))
                        .then_with(|| a.0.cmp(&b.0))
                });
            }
            Sort::Title => {
                candidates.sort_by(|a, b| {
                    let (da, db) = (&self.docs[a.0 as usize], &self.docs[b.0 as usize]);
                    da.title
                        .to_lowercase()
                        .cmp(&db.title.to_lowercase())
                        .then_with(|| a.0.cmp(&b.0))
                });
            }
            Sort::Path => {
                candidates.sort_by(|a, b| {
                    let (da, db) = (&self.docs[a.0 as usize], &self.docs[b.0 as usize]);
                    da.path
                        .to_lowercase()
                        .cmp(&db.path.to_lowercase())
                        .then_with(|| a.0.cmp(&b.0))
                });
            }
        }

        results.total_matched = candidates.len();
        if opts.limit > 0 {
            candidates.truncate(opts.limit);
        }

        let highlight_terms = parsed.all_terms();
        // Snippet terms: the raw text the user typed (so literal matches are found, e.g.
        // `DocSet` inside `DocSet.Open`) plus the primary tokens (so identifier pieces hit).
        let mut search_terms: Vec<String> = Vec::new();
        for variant in parsed.positive_variants() {
            let raw = variant.text.trim().to_lowercase();
            if !raw.is_empty() && !search_terms.contains(&raw) {
                search_terms.push(raw);
            }
            for token in tokenizer::tokenize_words(&variant.text) {
                if !search_terms.contains(&token.text) {
                    search_terms.push(token.text);
                }
            }
        }
        let phrases = phrase_terms(&parsed);
        for (doc, score) in candidates {
            let index = doc as usize;
            let stored = &self.docs[index];
            let fields = matched_fields.get(&doc).copied().unwrap_or(0);
            let code_only = fields & (1 << FIELD_CODE) != 0 && fields & !(1 << FIELD_CODE) == 0;
            let (snippet_text, snippet_ranges) = if opts.snippet {
                self.snippet_for(doc, &search_terms, &highlight_terms, &phrases, code_only)
            } else {
                (String::new(), Vec::new())
            };
            results.hits.push(Hit {
                doc,
                score,
                title: stored.title.clone(),
                path: stored.path.clone(),
                title_ranges: if opts.snippet {
                    snippet::ranges_in(&stored.title, &highlight_terms)
                } else {
                    Vec::new()
                },
                snippet: snippet_text,
                snippet_ranges,
                fields: matched_fields.get(&doc).copied().unwrap_or(0),
            });
        }
        if results.hits.is_empty() && matches!(opts.fuzzy, FuzzyMode::Auto) {
            results.suggestions = self.suggest(raw_query, 5);
        }
        results.took_us = started.elapsed().as_micros() as u64;
        results
    }

    /// Build the snippet for a hit: body text first, code samples as the fallback.
    ///
    /// `code_only` means the document matched exclusively in its code field — then only the
    /// code is offered to the snippet builder, so the reader sees the line that matched
    /// instead of an unrelated body paragraph.
    fn snippet_for(
        &self,
        doc: DocId,
        search_terms: &[String],
        highlight_terms: &[String],
        phrases: &[Vec<String>],
        code_only: bool,
    ) -> (String, Vec<Range<usize>>) {
        let request = snippet::SnippetRequest {
            terms: search_terms.to_vec(),
            highlight: highlight_terms.to_vec(),
            phrases: phrases.to_vec(),
            max_chars: self.options.max_snippet_chars,
        };
        let body = if code_only {
            ""
        } else {
            self.doc_text(doc).unwrap_or("")
        };
        let code = self.doc_code(doc).unwrap_or("");
        if body.trim().is_empty() && code.trim().is_empty() {
            return (String::new(), Vec::new());
        }
        let output = snippet::make(body, code, &request);
        (output.text, output.ranges)
    }

    /// Is a phrase present *adjacently* in a document?
    fn phrase_strength(&self, doc: DocId, phrase: &PlannedPhrase) -> PhraseStrength {
        if !self.options.keep_positions {
            return PhraseStrength::None;
        }
        // Positions of every phrase term, restricted to the phrase's fields.
        let mut per_term: Vec<Vec<u32>> = Vec::with_capacity(phrase.terms.len());
        for term in &phrase.terms {
            let Some(postings) = self.postings.get(*term as usize) else {
                return PhraseStrength::None;
            };
            let Some(posting) = postings
                .binary_search_by_key(&doc, |posting| posting.doc)
                .ok()
                .map(|index| &postings[index])
            else {
                return PhraseStrength::None;
            };
            let positions: Vec<u32> = posting
                .positions
                .iter()
                .copied()
                .filter(|position| phrase.fields & (1 << (position / POSITION_STRIDE)) != 0)
                .collect();
            if positions.is_empty() {
                return PhraseStrength::None;
            }
            per_term.push(positions);
        }
        // Anchor on the shortest list.
        let (anchor_index, _) = per_term
            .iter()
            .enumerate()
            .min_by_key(|(_, positions)| positions.len())
            .map(|(index, positions)| (index, positions.len()))
            .unwrap_or((0, 0));
        let anchor = per_term[anchor_index].clone();
        for start in anchor {
            let mut matched = true;
            for (offset, positions) in per_term.iter().enumerate() {
                if offset == anchor_index {
                    continue;
                }
                // The phrase sits at `start - anchor_index` in the token stream.
                let wanted = if offset >= anchor_index {
                    start + (offset - anchor_index) as u32
                } else {
                    match start.checked_sub((anchor_index - offset) as u32) {
                        Some(value) => value,
                        None => {
                            matched = false;
                            break;
                        }
                    }
                };
                if positions.binary_search(&wanted).is_err() {
                    matched = false;
                    break;
                }
            }
            if matched {
                let field = start / POSITION_STRIDE;
                return if field == FIELD_TITLE as u32 {
                    PhraseStrength::Title
                } else {
                    PhraseStrength::Body
                };
            }
        }
        PhraseStrength::None
    }

    /// Spell-check style suggestions for a query that matched nothing.
    pub fn suggest(&self, raw_query: &str, limit: usize) -> Vec<Suggestion> {
        let mut out: Vec<Suggestion> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for token in tokenizer::tokenize(raw_query) {
            let text = token.text;
            if text.chars().count() < 3 {
                continue;
            }
            // Prefix completions first: cheap and usually what the user meant.
            let (start, end) = self.prefix_range(&text);
            for term in self.terms[start..end].iter().take(3) {
                if term == &text || !seen.insert(term.clone()) {
                    continue;
                }
                out.push(Suggestion {
                    text: term.clone(),
                    kind: SuggestionKind::Term,
                });
            }
            // Then spelling: close vocabulary terms.
            let mut best: Vec<(f32, String)> = Vec::new();
            for term in &self.terms {
                if term == &text || seen.contains(term) {
                    continue;
                }
                let length_delta = (term.chars().count() as i32 - text.chars().count() as i32).abs();
                if length_delta > 2 {
                    continue;
                }
                if term.chars().next() != text.chars().next() {
                    continue;
                }
                let distance = levenshtein(term, &text);
                if distance > 2 {
                    continue;
                }
                let similarity = jaro_winkler(term, &text) as f32;
                best.push((similarity - distance as f32 * 0.1, term.clone()));
            }
            best.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.cmp(&b.1))
            });
            for (_, term) in best.into_iter().take(3) {
                if seen.insert(term.clone()) {
                    out.push(Suggestion {
                        text: term,
                        kind: SuggestionKind::Spelling,
                    });
                }
            }
        }
        out.truncate(limit);
        out
    }

    // ------------------------------------------------------------------ internals

    fn prefix_range(&self, prefix: &str) -> (usize, usize) {
        let start = self.terms.partition_point(|term| term.as_str() < prefix);
        let mut end = start;
        while end < self.terms.len() && self.terms[end].starts_with(prefix) {
            end += 1;
        }
        (start, end)
    }

    fn term_id(&self, term: &str) -> Option<u32> {
        self.terms
            .binary_search_by(|probe| probe.as_str().cmp(term))
            .ok()
            .map(|index| index as u32)
    }

    fn stem_of(stemmer: &Stemmer, term: &str) -> Option<String> {
        if term.chars().count() <= 2 {
            return None;
        }
        let stem = stemmer.stem(term).to_string();
        (!stem.is_empty() && stem != term).then_some(stem)
    }

    /// Expand a variant into concrete index terms with penalties.
    fn expand_variant(
        &self,
        variant: &Variant,
        stemmer: &Stemmer,
        opts: &SearchOptions,
    ) -> Vec<(u32, f32)> {
        let mut out: Vec<(u32, f32)> = Vec::new();
        let push = |term: &str, penalty: f32, out: &mut Vec<(u32, f32)>| {
            if let Some(id) = self.term_id(term) {
                out.push((id, penalty));
            }
        };

        match variant.kind {
            VariantKind::Term => {
                let term = &variant.terms[0];
                push(term, 1.0, &mut out);
                // The stem is always offered as an extra alternative, not just as a
                // last resort: `title:install` must find the topic whose title says
                // "Installing" even though the exact word never appears there.
                if self.options.stem {
                    if let Some(stem) = Self::stem_of(stemmer, term) {
                        push(&stem, rank::STEM_PENALTY, &mut out);
                    }
                }
                if variant.prefix {
                    let (start, end) = self.prefix_range(term);
                    for id in start..end.min(start + 64) {
                        out.push((id as u32, rank::PREFIX_PENALTY));
                    }
                }
                // `term~` always expands fuzzily; `FuzzyMode::Always` does it even when the
                // exact term matched, so near-misses show up next to the exact hits.
                if variant.fuzzy || opts.fuzzy == FuzzyMode::Always {
                    if out.is_empty() || opts.fuzzy == FuzzyMode::Always {
                        for (id, _similarity) in self.fuzzy_matches(term, 8) {
                            out.push((id, rank::FUZZY_PENALTY));
                        }
                    }
                }
                // Nothing matched: fall back to a prefix expansion. Dotted and dashed
                // identifiers are indexed *joined* (`DocSet.Open` → `docsetopen`), so
                // typing the first word of one still finds it.
                if out.is_empty() {
                    let (start, end) = self.prefix_range(term);
                    for id in start..end.min(start + 16) {
                        out.push((id as u32, rank::PREFIX_PENALTY * rank::PREFIX_PENALTY));
                    }
                }
            }
            VariantKind::Phrase => {
                // Each term of the phrase is expanded on its own; adjacency is checked later.
                for term in &variant.terms {
                    let mut found = false;
                    if let Some(id) = self.term_id(term) {
                        out.push((id, 1.0));
                        found = true;
                    }
                    if !found && self.options.stem {
                        if let Some(stem) = Self::stem_of(stemmer, term) {
                            if let Some(id) = self.term_id(&stem) {
                                out.push((id, rank::STEM_PENALTY));
                            }
                        }
                    }
                }
            }
        }

        out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)));
        out.dedup_by_key(|entry| entry.0);
        out
    }

    /// Vocabulary terms within edit distance 2 (bounded work: length filter + first char).
    fn fuzzy_matches(&self, term: &str, limit: usize) -> Vec<(u32, f32)> {
        let mut candidates: Vec<(usize, u32, f32)> = Vec::new();
        let term_len = term.chars().count();
        let first = term.chars().next();
        for (index, candidate) in self.terms.iter().enumerate() {
            let length = candidate.chars().count();
            if length.abs_diff(term_len) > 2 {
                continue;
            }
            if candidate.chars().next() != first && !candidate.ends_with(term) {
                continue;
            }
            let distance = levenshtein(candidate, term);
            if distance <= 2 {
                candidates.push((distance, index as u32, jaro_winkler(candidate, term) as f32));
            }
            if candidates.len() > 512 {
                break;
            }
        }
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.1.cmp(&b.1))
        });
        candidates
            .into_iter()
            .take(limit)
            .map(|(distance, id, similarity)| (id, similarity / (distance as f32 + 1.0)))
            .collect()
    }

    /// Turn a parsed query into an executable plan.
    fn plan(&self, parsed: &ParsedQuery, stemmer: &Stemmer, opts: &SearchOptions) -> Plan {
        let mut plan = Plan::default();
        for group in &parsed.groups {
            let single_terms: Vec<&Variant> = group
                .variants
                .iter()
                .filter(|variant| variant.kind == VariantKind::Term)
                .collect();
            let phrases: Vec<&Variant> = group
                .variants
                .iter()
                .filter(|variant| variant.kind == VariantKind::Phrase)
                .collect();

            if group.negated {
                for variant in &group.variants {
                    for term in &variant.terms {
                        let mut forms = vec![term.clone()];
                        if self.options.stem {
                            if let Some(stem) = Self::stem_of(stemmer, term) {
                                forms.push(stem);
                            }
                        }
                        for form in forms {
                            if let Some(id) = self.term_id(&form) {
                                plan.excluded.push(PlannedAlternative {
                                    term: id,
                                    penalty: 1.0,
                                    fields: variant.fields,
                                });
                            }
                        }
                    }
                }
                continue;
            }

            if !single_terms.is_empty() {
                let mut alternatives: Vec<PlannedAlternative> = Vec::new();
                let mut fields = 0u8;
                let mut scoring = true;
                for variant in &single_terms {
                    fields |= variant.fields;
                    if variant.fields == 0 {
                        scoring = false;
                    }
                    for (term, penalty) in self.expand_variant(variant, stemmer, opts) {
                        alternatives.push(PlannedAlternative {
                            term,
                            penalty,
                            fields: variant.fields,
                        });
                    }
                }
                alternatives.sort_by_key(|alternative| alternative.term);
                alternatives.dedup_by_key(|alternative| (alternative.term, alternative.fields));
                if !alternatives.is_empty() {
                    plan.conditions.push(PlannedCondition {
                        alternatives,
                        fields,
                        scoring,
                    });
                }
            }

            for variant in phrases {
                let mut term_ids: Vec<u32> = Vec::new();
                for term in &variant.terms {
                    let mut found = None;
                    if let Some(id) = self.term_id(term) {
                        found = Some(id);
                    } else if self.options.stem {
                        if let Some(stem) = Self::stem_of(stemmer, term) {
                            found = self.term_id(&stem);
                        }
                    }
                    match found {
                        Some(id) => term_ids.push(id),
                        None => {
                            // A phrase term that is not in the vocabulary can never match.
                            term_ids.clear();
                            break;
                        }
                    }
                }
                if term_ids.is_empty() {
                    continue;
                }
                for id in &term_ids {
                    plan.conditions.push(PlannedCondition {
                        alternatives: vec![PlannedAlternative {
                            term: *id,
                            penalty: 1.0,
                            fields: variant.fields,
                        }],
                        fields: variant.fields,
                        scoring: true,
                    });
                }
                // CJK is tokenized per character with joined runs interleaved, so
                // `検索` cannot require adjacency — characters are ANDed instead.
                let cjk_characters = variant.terms.iter().all(|term| {
                    term.chars().count() == 1 && term.chars().all(tokenizer::is_cjk)
                });
                if !cjk_characters {
                    plan.phrases.push(PlannedPhrase {
                        terms: term_ids,
                        fields: variant.fields,
                    });
                }
            }
        }

        if plan.conditions.is_empty() && !plan.excluded.is_empty() {
            // A query made only of exclusions matches nothing; keep it empty on purpose.
        }
        plan
    }

    // ------------------------------------------------------------------ persistence

    /// Serialize the index (versioned blob, ready to be written to a cache file).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1024);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        match bincode_options().serialize(self) {
            Ok(payload) => out.extend_from_slice(&payload),
            Err(_) => {}
        }
        out
    }

    /// Deserialize an index produced by [`Index::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Index> {
        if bytes.len() < MAGIC.len() + 4 {
            return Err(Error::Format("index blob is too short".to_owned()));
        }
        if &bytes[..MAGIC.len()] != MAGIC {
            return Err(Error::Format("index blob has the wrong magic".to_owned()));
        }
        let mut version_bytes = [0u8; 4];
        version_bytes.copy_from_slice(&bytes[MAGIC.len()..MAGIC.len() + 4]);
        let version = u32::from_le_bytes(version_bytes);
        if version != FORMAT_VERSION {
            return Err(Error::Version {
                found: version,
                expected: FORMAT_VERSION,
            });
        }
        let payload = &bytes[MAGIC.len() + 4..];
        let index: Index = bincode_options()
            .with_limit(payload.len() as u64 * 64 + 4096)
            .deserialize(payload)
            .map_err(|err| Error::Format(err.to_string()))?;
        if index.terms.len() != index.postings.len() {
            return Err(Error::Format(
                "term dictionary and postings have different lengths".to_owned(),
            ));
        }
        Ok(index)
    }

    /// Write the index to a file.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let bytes = self.to_bytes();
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, &bytes)?;
        std::fs::rename(&temp, path)?;
        Ok(())
    }

    /// Read an index from a file.
    pub fn load(path: &std::path::Path) -> Result<Index> {
        let bytes = std::fs::read(path)?;
        Index::from_bytes(&bytes)
    }
}

fn bincode_options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_little_endian()
}

/// The phrase variants of a query, as term lists (used to score snippet windows).
fn phrase_terms(parsed: &ParsedQuery) -> Vec<Vec<String>> {
    parsed
        .groups
        .iter()
        .flat_map(|group| group.variants.iter())
        .filter(|variant| variant.kind == VariantKind::Phrase)
        .map(|variant| variant.terms.clone())
        .collect()
}

/// How strongly a phrase matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhraseStrength {
    None,
    Body,
    Title,
}

#[derive(Debug, Clone, Copy)]
struct PlannedAlternative {
    term: u32,
    penalty: f32,
    fields: u8,
}

#[derive(Debug, Clone)]
struct PlannedCondition {
    alternatives: Vec<PlannedAlternative>,
    /// Union of the alternatives' field masks (diagnostics and future filters).
    #[allow(dead_code)]
    fields: u8,
    /// False when the field mask is empty (nothing can match → keep the score at zero).
    scoring: bool,
}

#[derive(Debug, Clone)]
struct PlannedPhrase {
    terms: Vec<u32>,
    fields: u8,
}

#[derive(Debug, Default)]
struct Plan {
    conditions: Vec<PlannedCondition>,
    excluded: Vec<PlannedAlternative>,
    phrases: Vec<PlannedPhrase>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(docs: &[(&str, &str, &str, &str, &str)]) -> Index {
        let mut builder = IndexBuilder::default();
        for (title, headings, keywords, body, code) in docs {
            builder.add(DocInput {
                title,
                headings,
                keywords,
                body,
                code,
                path: title,
                weight: 1.0,
            });
        }
        builder.build()
    }

    #[test]
    fn exact_term_matches_and_ranks_by_field() {
        let idx = index(&[
            ("Install Lucid", "", "install", "How to install the tool.", ""),
            ("Other topic", "", "", "This page mentions install once in passing and then keeps talking.", ""),
        ]);
        let results = idx.search("install", &SearchOptions::default());
        assert_eq!(results.hits.len(), 2);
        assert_eq!(results.hits[0].title, "Install Lucid", "title match must win");
        assert!(results.hits[0].score > results.hits[1].score);
    }

    #[test]
    fn stemming_finds_inflections() {
        let idx = index(&[("Docs", "", "", "Installing is documented here.", "")]);
        let results = idx.search("install", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
        let results = idx.search("installs", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
    }

    #[test]
    fn stemming_applies_inside_field_filters() {
        // The exact word only appears in the keywords, while the title has "Installing".
        let idx = index(&[(
            "Installing Lucid",
            "",
            "install, setup",
            "Copy the binary and run it.",
            "",
        )]);
        let results = idx.search("install", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
        let results = idx.search("title:install", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1, "title:install must find `Installing`");
        assert_eq!(results.hits[0].matched_fields(), vec!["title"]);
        let results = idx.search("kw:install", &SearchOptions::default());
        assert_eq!(results.hits[0].matched_fields(), vec!["keywords"]);
        // A field that genuinely has no match stays empty.
        let results = idx.search("code:install", &SearchOptions::default());
        assert!(results.hits.is_empty());
    }

    #[test]
    fn identifiers_are_searchable_by_their_parts() {
        let idx = index(&[(
            "Format",
            "",
            "",
            "The meta tag Microsoft.Help.TocParent decides the placement.",
            "",
        )]);
        // The tokenizer indexes the joined form plus every piece, so all of these hit.
        for query in ["microsofthelptocparent", "microsoft", "toc", "parent"] {
            let results = idx.search(query, &SearchOptions::default());
            assert_eq!(results.hits.len(), 1, "query `{query}` must hit");
        }
        // A piece that does not exist as a token (`toc` + `parent` joined by hand) does not.
        let results = idx.search("tocparent", &SearchOptions::default());
        assert!(results.hits.is_empty());
    }

    #[test]
    fn and_semantics_and_exclusions() {
        let idx = index(&[
            ("Search", "", "", "phrase search and prefix search are supported.", ""),
            ("Legacy", "", "", "an older search implementation with legacy behaviour.", ""),
            ("Unrelated", "", "", "nothing to see here.", ""),
        ]);
        let results = idx.search("search prefix", &SearchOptions::default());
        assert_eq!(results.total_matched, 1);
        let results = idx.search("search -legacy", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].title, "Search");
        let results = idx.search("search OR legacy", &SearchOptions::default());
        assert_eq!(results.total_matched, 2);
    }

    #[test]
    fn phrase_requires_adjacency() {
        let idx = index(&[
            ("Good", "", "", "the phrase search engine ranks results", ""),
            ("Bad", "", "", "search for the phrase, engine and ranks in any order", ""),
        ]);
        let results = idx.search("\"phrase search\"", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].title, "Good");
        // Both words present in any order: still matched (phrase terms are required), but the
        // adjacent document must rank first.
        let results = idx.search("\"search engine\"", &SearchOptions::default());
        assert!(results.hits[0].title == "Good");
    }

    #[test]
    fn prefix_matching_including_last_term_mode() {
        let idx = index(&[("Searching", "", "", "search and searching and searcher", "")]);
        let results = idx.search("search*", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1, "an explicit `*` is a prefix match");

        let mut opts = SearchOptions::default();
        opts.prefix_last_term = true;
        let results = idx.search("sear", &opts);
        assert_eq!(results.hits.len(), 1, "prefix_last_term must match");

        // Without the flag the word still matches through the prefix fallback (which is how
        // partially typed identifiers are found), so the flag only changes the *penalty*.
        let results = idx.search("sear", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1, "fallback prefix match");
        // A word that is neither a term nor a prefix of one matches nothing.
        let results = idx.search("searchx", &SearchOptions::default());
        assert!(results.hits.is_empty());
    }

    #[test]
    fn fuzzy_terms_match_near_misses() {
        let idx = index(&[("Installing", "", "", "install lucid quickly", "")]);
        let results = idx.search("instal~", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1, "an edit distance of 1 must match");
    }

    #[test]
    fn fuzzy_expansions_rank_below_exact_matches() {
        let idx = index(&[
            ("Exact", "", "", "instal", ""),
            ("Near", "", "", "instill", ""),
        ]);
        // With `Auto`, an exact match suppresses fuzzy expansion entirely.
        let results = idx.search("instal", &SearchOptions::default());
        assert_eq!(results.total_matched, 1);

        // With `Always`, near-misses join the result list, ranked lower.
        let mut options = SearchOptions::default();
        options.fuzzy = FuzzyMode::Always;
        let results = idx.search("instal", &options);
        assert_eq!(results.total_matched, 2);
        assert_eq!(results.hits[0].title, "Exact");
        assert_eq!(results.hits[1].title, "Near");
    }

    #[test]
    fn suggestions_appear_when_nothing_matched() {
        let idx = index(&[("Installing", "", "", "install lucid quickly", "")]);
        let results = idx.search("insttall", &SearchOptions::default());
        assert!(results.hits.is_empty());
        assert!(!results.suggestions.is_empty(), "expected suggestions");
        assert!(
            results
                .suggestions
                .iter()
                .any(|suggestion| suggestion.text == "install"),
            "{:?}",
            results.suggestions
        );
    }

    #[test]
    fn field_filters() {
        let idx = index(&[
            ("Title mentions search", "", "", "nothing else here", ""),
            ("Plain", "", "", "body mentions search", ""),
        ]);
        let results = idx.search("title:search", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].title, "Title mentions search");
        let results = idx.search("body:search", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].title, "Plain");
    }

    #[test]
    fn snippets_highlight_the_actual_match() {
        let idx = index(&[(
            "Search",
            "",
            "",
            "Lucid supports phrase search with adjacency checks and highlighting.",
            "",
        )]);
        let results = idx.search("phrase", &SearchOptions::default());
        let hit = &results.hits[0];
        assert!(!hit.snippet.is_empty());
        let chars: Vec<char> = hit.snippet.chars().collect();
        for range in &hit.snippet_ranges {
            let text: String = chars[range.start..range.end].iter().collect();
            assert_eq!(text.to_lowercase(), "phrase");
        }
    }

    #[test]
    fn code_snippets_are_marked_when_only_code_matches() {
        let idx = index(&[(
            "Reference",
            "",
            "",
            "No mention of the identifier in prose.",
            "var set = DocSet.Open(\"docs.mshc\");",
        )]);
        // Dotted identifiers are indexed joined (`docsetopen`) plus their pieces.
        let results = idx.search("docsetopen", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
        assert!(
            results.hits[0].snippet.starts_with("⟨code⟩"),
            "snippet: {}",
            results.hits[0].snippet
        );
        // A query for the first word only still finds it (prefix fallback).
        let results = idx.search("docset", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1, "prefix fallback for identifiers");
    }

    #[test]
    fn deterministic_and_stable_across_rebuilds() {
        let docs = [
            ("A", "", "", "alpha beta gamma", ""),
            ("B", "", "", "beta gamma delta", ""),
            ("C", "", "", "gamma delta epsilon", ""),
        ];
        let first = index(&docs);
        let second = index(&docs);
        let opts = SearchOptions::default();
        let a: Vec<(DocId, i64)> = first
            .search("gamma", &opts)
            .hits
            .iter()
            .map(|hit| (hit.doc, (hit.score * 1000.0) as i64))
            .collect();
        let b: Vec<(DocId, i64)> = second
            .search("gamma", &opts)
            .hits
            .iter()
            .map(|hit| (hit.doc, (hit.score * 1000.0) as i64))
            .collect();
        assert_eq!(a, b);
    }

    #[test]
    fn persistence_round_trip() {
        let idx = index(&[
            ("Search", "Searching", "search", "phrase search ranks results", "code()"),
            ("Other", "", "", "unrelated text", ""),
        ]);
        let bytes = idx.to_bytes();
        let restored = Index::from_bytes(&bytes).expect("round trip");
        assert_eq!(restored.doc_count(), idx.doc_count());
        assert_eq!(restored.term_count(), idx.term_count());
        let opts = SearchOptions::default();
        let before = idx.search("phrase search", &opts);
        let after = restored.search("phrase search", &opts);
        assert_eq!(before.hits.len(), after.hits.len());
        assert_eq!(before.hits[0].title, after.hits[0].title);
        assert_eq!(before.hits[0].snippet, after.hits[0].snippet);
        assert_eq!(restored.doc_text(before.hits[0].doc), idx.doc_text(before.hits[0].doc));
    }

    #[test]
    fn persistence_rejects_corrupt_input() {
        assert!(Index::from_bytes(b"").is_err());
        assert!(Index::from_bytes(b"NOTANIDX\0\0\0\0junk").is_err());
        let mut bytes = Index::single(DocInput {
            title: "t",
            headings: "",
            keywords: "",
            body: "b",
            code: "",
            path: "t",
            weight: 1.0,
        })
        .to_bytes();
        bytes[8] = 99; // bump the version byte
        match Index::from_bytes(&bytes) {
            Err(Error::Version { .. }) => {}
            other => panic!("expected version error, got {other:?}"),
        }
    }

    #[test]
    fn scope_restricts_results() {
        let idx = index(&[
            ("A", "", "", "shared word", ""),
            ("B", "", "", "shared word", ""),
        ]);
        let mut opts = SearchOptions::default();
        opts.scope = Some(vec![1]);
        let results = idx.search("shared", &opts);
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].doc, 1);
    }

    #[test]
    fn limits_and_sorting() {
        let idx = index(&[
            ("Zeta", "", "", "term", ""),
            ("Alpha", "", "", "term", ""),
            ("Mid", "", "", "term", ""),
        ]);
        let mut opts = SearchOptions::default();
        opts.limit = 2;
        let results = idx.search("term", &opts);
        assert_eq!(results.hits.len(), 2);
        assert_eq!(results.total_matched, 3);

        opts.sort = Sort::Title;
        let results = idx.search("term", &opts);
        assert_eq!(results.hits[0].title, "Alpha");
    }

    #[test]
    fn empty_and_degenerate_queries() {
        let idx = index(&[("A", "", "", "body", "")]);
        for query in ["", "   ", "-", "\"\"", "~", "*"] {
            let results = idx.search(query, &SearchOptions::default());
            assert!(results.hits.is_empty(), "query `{query}`");
        }
    }

    #[test]
    fn cjk_search() {
        let idx = index(&[("日本語", "", "", "ドキュメントの検索エンジンです", "")]);
        // CJK is indexed per character (plus joined runs), so multi-character queries are
        // ANDed character by character instead of requiring adjacency.
        assert_eq!(idx.search("検", &SearchOptions::default()).hits.len(), 1);
        assert_eq!(idx.search("検索", &SearchOptions::default()).hits.len(), 1);
        assert_eq!(idx.search("ドキュメント", &SearchOptions::default()).hits.len(), 1);
        assert_eq!(idx.search("日本語", &SearchOptions::default()).hits.len(), 1);
        // Characters that are not in the document do not match.
        assert!(idx.search("猫", &SearchOptions::default()).hits.is_empty());
    }

    #[test]
    fn ten_thousand_docs_stay_fast() {
        let mut builder = IndexBuilder::default();
        for index in 0..10_000 {
            let body = format!(
                "topic {index} documents the widget pipeline stage {} and mentions search {} times",
                index % 37,
                index % 3
            );
            builder.add(DocInput {
                title: &format!("Topic {index}"),
                headings: "Widgets Pipeline Search",
                keywords: "widget, pipeline",
                body: &body,
                code: &format!("widget.run({index});"),
                path: &format!("topics/{index}.htm"),
                weight: 1.0,
            });
        }
        let started = std::time::Instant::now();
        let idx = builder.build();
        let build_ms = started.elapsed().as_millis();
        assert!(build_ms < 5_000, "build took {build_ms} ms");

        let opts = SearchOptions::default();
        let mut worst = 0u64;
        for query in ["widget", "pipeline OR search", "\"widget pipeline\"", "title:topic", "widget*"] {
            let started = std::time::Instant::now();
            let results = idx.search(query, &opts);
            let took = started.elapsed().as_micros() as u64;
            worst = worst.max(took);
            assert!(!results.hits.is_empty(), "query `{query}` returned nothing");
        }
        println!("10k build {build_ms} ms, worst query {worst} µs");
        assert!(worst < 200_000, "worst query took {worst} µs");
    }

    #[test]
    fn documents_without_text_do_not_panic() {
        let mut builder = IndexBuilder::new(IndexOptions {
            keep_text: false,
            ..Default::default()
        });
        builder.add(DocInput {
            title: "T",
            headings: "",
            keywords: "",
            body: "some body text",
            code: "",
            path: "t",
            weight: 1.0,
        });
        let idx = builder.build();
        assert!(idx.doc_text(0).is_none());
        let results = idx.search("body", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1);
        assert!(results.hits[0].snippet.is_empty(), "no stored text → no snippet");
    }

    #[test]
    fn positions_can_be_disabled() {
        let mut builder = IndexBuilder::new(IndexOptions {
            keep_positions: false,
            ..Default::default()
        });
        builder.add(DocInput {
            title: "T",
            headings: "",
            keywords: "",
            body: "phrase search",
            code: "",
            path: "t",
            weight: 1.0,
        });
        let idx = builder.build();
        let results = idx.search("\"phrase search\"", &SearchOptions::default());
        assert_eq!(results.hits.len(), 1, "co-occurrence still matches without positions");
    }
}
