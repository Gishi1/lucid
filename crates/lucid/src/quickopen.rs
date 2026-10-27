//! Ctrl+P quick open: fuzzy match over every topic title in the content set.
//!
//! The matcher is a small subsequence matcher with the bonuses that matter in practice:
//! matches at the start of the title, at word boundaries and in camelCase humps score higher,
//! and exact substrings beat scattered characters.

use std::collections::HashMap;

use lucid_mshc::{DocSet, TopicKey};

/// One candidate entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: TopicKey,
    pub title: String,
    pub title_lower: String,
    pub path: String,
    pub keywords: String,
}

/// A scored match.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub key: TopicKey,
    pub title: String,
    pub path: String,
    pub score: f32,
    /// Character positions of the matched characters in `title`.
    pub positions: Vec<usize>,
}

/// Build the candidate list for a content set.
pub fn entries(set: &DocSet, paths: &HashMap<TopicKey, String>) -> Vec<Entry> {
    let mut out = Vec::with_capacity(set.topic_count());
    for (package_index, package) in set.packages.iter().enumerate() {
        for topic in &package.topics {
            let key = TopicKey::new(package_index as u32, topic.key.topic);
            out.push(Entry {
                key,
                title: topic.title.clone(),
                title_lower: topic.title.to_lowercase(),
                path: paths
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| topic.title.clone()),
                keywords: topic.keywords.join(", ").to_lowercase(),
            });
        }
    }
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

/// Filter and rank entries for a query.
pub fn filter(entries: &[Entry], query: &str, limit: usize) -> Vec<Match> {
    let query = query.trim();
    if query.is_empty() {
        return entries
            .iter()
            .take(limit)
            .map(|entry| Match {
                key: entry.key,
                title: entry.title.clone(),
                path: entry.path.clone(),
                score: 0.0,
                positions: Vec::new(),
            })
            .collect();
    }
    let needle = query.to_lowercase();
    let mut matches: Vec<Match> = entries
        .iter()
        .filter_map(|entry| {
            let (score, positions) = score(&needle, &entry.title_lower, &entry.keywords)?;
            Some(Match {
                key: entry.key,
                title: entry.title.clone(),
                path: entry.path.clone(),
                score,
                positions,
            })
        })
        .collect();
    matches.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.chars().count().cmp(&b.title.chars().count()))
            .then_with(|| a.title.cmp(&b.title))
    });
    matches.truncate(limit);
    matches
}

/// Score a needle against a haystack, returning the score and matched character positions.
fn score(needle: &str, haystack: &str, keywords: &str) -> Option<(f32, Vec<usize>)> {
    // Exact substring: strong, and the positions are trivial.
    if let Some(byte_index) = haystack.find(needle) {
        let start = haystack[..byte_index].chars().count();
        let length = needle.chars().count();
        let mut score = 100.0 - start as f32 * 0.5;
        if start == 0 {
            score += 60.0;
        } else {
            // Word-boundary bonus.
            let before = haystack[..byte_index].chars().next_back();
            if before.map(|ch| !ch.is_alphanumeric()).unwrap_or(false) {
                score += 25.0;
            }
        }
        let positions = (start..start + length).collect();
        return Some((score, positions));
    }

    // Subsequence match with bonuses.
    let hay: Vec<char> = haystack.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let mut positions = Vec::with_capacity(pat.len());
    let mut cursor = 0usize;
    let mut score = 0.0f32;
    for (needle_index, needle_char) in pat.iter().enumerate() {
        let mut found = None;
        while cursor < hay.len() {
            let candidate = hay[cursor];
            if candidate == *needle_char {
                found = Some(cursor);
                break;
            }
            cursor += 1;
        }
        let index = found?;
        // Bonuses: start of title, word boundary, consecutive characters.
        if index == 0 {
            score += 12.0;
        }
        if index > 0 && !hay[index - 1].is_alphanumeric() {
            score += 8.0;
        }
        if index > 0 && hay[index - 1].is_lowercase() && candidate_is_upper(&haystack, index) {
            score += 6.0;
        }
        if positions.last().map(|last| *last + 1 == index).unwrap_or(false) {
            score += 10.0;
        }
        score += 4.0 - (index.saturating_sub(positions.last().copied().unwrap_or(0)) as f32).min(4.0);
        let _ = needle_index;
        positions.push(index);
        cursor = index + 1;
    }
    // Only the title is matched here; keywords give a small bonus for tie-breaking.
    if !keywords.is_empty() && keywords.contains(needle) {
        score += 10.0;
    }
    // Prefer shorter titles when everything else is equal.
    score -= hay.len() as f32 * 0.05;
    Some((score, positions))
}

fn candidate_is_upper(haystack: &str, index: usize) -> bool {
    haystack
        .chars()
        .nth(index)
        .map(|ch| ch.is_uppercase())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, keywords: &str) -> Entry {
        Entry {
            key: TopicKey::new(0, 0),
            title: title.to_owned(),
            title_lower: title.to_lowercase(),
            path: title.to_owned(),
            keywords: keywords.to_owned(),
        }
    }

    #[test]
    fn exact_prefix_wins() {
        let entries = vec![
            entry("Installing Lucid", "setup"),
            entry("Uninstall", ""),
            entry("First run", "install"),
        ];
        let matches = filter(&entries, "install", 10);
        assert_eq!(matches[0].title, "Installing Lucid");
        assert!(matches.iter().any(|m| m.title == "Uninstall"));
        assert!(matches[0].positions.len() == 7);
    }

    #[test]
    fn subsequence_matches_are_found() {
        let entries = vec![entry("Query syntax", ""), entry("Queries in depth", "")];
        let matches = filter(&entries, "qs", 10);
        assert!(matches.iter().any(|m| m.title == "Query syntax"));
        let matches = filter(&entries, "qry", 10);
        assert!(matches.iter().any(|m| m.title == "Query syntax"));
    }

    #[test]
    fn unmatched_entries_are_dropped() {
        let entries = vec![entry("Searching", ""), entry("Browsing", "")];
        let matches = filter(&entries, "zzz", 10);
        assert!(matches.is_empty());
    }

    #[test]
    fn empty_query_lists_everything_up_to_the_limit() {
        let entries = vec![entry("B", ""), entry("A", "")];
        let matches = filter(&entries, "", 1);
        assert_eq!(matches.len(), 1);
        let matches = filter(&entries, "   ", 10);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn ranking_is_stable_for_equal_scores() {
        let entries = vec![entry("Alpha", ""), entry("Alphb", "")];
        let first = filter(&entries, "alph", 10);
        let second = filter(&entries, "alph", 10);
        assert_eq!(
            first.iter().map(|m| m.title.clone()).collect::<Vec<_>>(),
            second.iter().map(|m| m.title.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unicode_titles_are_handled() {
        let entries = vec![entry("日本語のドキュメント", ""), entry("Documentation", "")];
        let matches = filter(&entries, "日", 10);
        assert_eq!(matches[0].title, "日本語のドキュメント");
        let matches = filter(&entries, "doc", 10);
        assert_eq!(matches[0].title, "Documentation");
    }
}
