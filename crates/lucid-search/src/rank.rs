//! Ranking: BM25F with field weights, plus the boost/penalty constants of
//! `docs/DESIGN.md` §4.2.

use crate::{FIELD_BODY, FIELD_CODE, FIELD_HEADINGS, FIELD_KEYWORDS, FIELD_TITLE, NUM_FIELDS};

/// BM25 saturation parameter.
pub const K1: f32 = 1.2;
/// BM25 length-normalization parameter.
pub const B: f32 = 0.65;

/// Per-field weights. Title and keywords dominate, body text is the baseline, code samples
/// count least (they are noisy: identifiers repeat).
pub const FIELD_WEIGHTS: [f32; NUM_FIELDS] = {
    let mut weights = [1.0f32; NUM_FIELDS];
    weights[FIELD_TITLE as usize] = 8.0;
    weights[FIELD_HEADINGS as usize] = 3.0;
    weights[FIELD_KEYWORDS as usize] = 5.0;
    weights[FIELD_BODY as usize] = 1.0;
    weights[FIELD_CODE as usize] = 0.6;
    weights
};

/// Multiplier applied to prefix-expanded terms (they are a superset of the typed term).
pub const PREFIX_PENALTY: f32 = 0.9;
/// Multiplier applied to fuzzy-expanded terms.
pub const FUZZY_PENALTY: f32 = 0.5;
/// Multiplier when the term was found through its stem instead of exactly.
pub const STEM_PENALTY: f32 = 0.95;
/// Bonus when every positive term of the query matched.
pub const ALL_TERMS_BONUS: f32 = 1.2;
/// Bonus when a phrase matched adjacently in the title.
pub const PHRASE_TITLE_BONUS: f32 = 2.0;
/// Bonus when a phrase matched adjacently in the body.
pub const PHRASE_BODY_BONUS: f32 = 1.35;
/// Bonus when the title equals the whole query.
pub const EXACT_TITLE_BONUS: f32 = 3.0;

/// Inverse document frequency (always positive, never divides by zero).
pub fn idf(doc_count: usize, doc_freq: usize) -> f32 {
    let n = doc_count.max(1) as f32;
    let df = doc_freq.max(1) as f32;
    (1.0 + (n - df + 0.5) / (df + 0.5)).ln().max(0.0)
}

/// Length-normalized term frequency for one field of one document.
pub fn tf_norm(tf: f32, field_len: f32, avg_len: f32) -> f32 {
    if tf <= 0.0 {
        return 0.0;
    }
    let avg = if avg_len > 0.0 { avg_len } else { 1.0 };
    tf / (tf + K1 * (1.0 - B + B * (field_len / avg)))
}

/// Weighted contribution of a term to a document.
pub fn term_score(
    idf: f32,
    tf: &[u16; NUM_FIELDS],
    field_len: &[u32; NUM_FIELDS],
    avg_len: &[f32; NUM_FIELDS],
    allowed_fields: u8,
) -> f32 {
    let mut score = 0.0f32;
    for field in 0..NUM_FIELDS {
        if allowed_fields & (1 << field) == 0 {
            continue;
        }
        let tf = tf[field] as f32;
        if tf == 0.0 {
            continue;
        }
        score += FIELD_WEIGHTS[field]
            * tf_norm(tf, field_len[field] as f32, avg_len[field]);
    }
    idf * score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idf_grows_for_rarer_terms() {
        let common = idf(1000, 500);
        let rare = idf(1000, 1);
        assert!(rare > common);
        assert!(common > 0.0);
        assert_eq!(idf(0, 0).is_finite(), true);
    }

    #[test]
    fn tf_saturates_and_length_normalizes() {
        let one = tf_norm(1.0, 100.0, 100.0);
        let five = tf_norm(5.0, 100.0, 100.0);
        assert!(five > one, "more occurrences must score higher");
        assert!(five / one < 5.0, "BM25 saturates");

        let long_doc = tf_norm(1.0, 1000.0, 100.0);
        let short_doc = tf_norm(1.0, 10.0, 100.0);
        assert!(short_doc > long_doc, "shorter fields are more specific");
    }

    #[test]
    fn field_weights_prefer_the_title() {
        let mut tf = [0u16; NUM_FIELDS];
        tf[FIELD_TITLE as usize] = 1;
        let title_hit = term_score(1.0, &tf, &[10, 10, 10, 10, 10], &[10.0; NUM_FIELDS], 0xFF);

        let mut tf = [0u16; NUM_FIELDS];
        tf[FIELD_BODY as usize] = 1;
        let body_hit = term_score(1.0, &tf, &[10, 10, 10, 10, 10], &[10.0; NUM_FIELDS], 0xFF);

        assert!(title_hit > body_hit * 4.0);
    }

    #[test]
    fn field_masks_restrict_contributions() {
        let mut tf = [0u16; NUM_FIELDS];
        tf[FIELD_BODY as usize] = 1;
        let only_title = term_score(1.0, &tf, &[10; NUM_FIELDS], &[10.0; NUM_FIELDS], 1 << FIELD_TITLE);
        assert_eq!(only_title, 0.0);
    }
}
