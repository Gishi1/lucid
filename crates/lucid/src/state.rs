//! Persisted application state: recent files, bookmarks, history, preferences.
//!
//! Stored as JSON in `$XDG_CONFIG_HOME/lucid/state.json`. Nothing is written into the
//! packages themselves, so a read-only content store stays read-only.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

impl ThemeChoice {
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            ThemeChoice::Dark => ThemeChoice::Light,
            ThemeChoice::Light => ThemeChoice::Dark,
        }
    }
}

/// Preferred code language for `LanguageSpecificText` and snippet tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodeLanguage {
    Cs,
    Vb,
    Cpp,
    /// `LanguageSpecificText devLangnu` — a language the author invented.
    Other(String),
}

impl Default for CodeLanguage {
    fn default() -> Self {
        CodeLanguage::Cs
    }
}

impl CodeLanguage {
    pub fn label(&self) -> String {
        match self {
            CodeLanguage::Cs => "C#".to_owned(),
            CodeLanguage::Vb => "Visual Basic".to_owned(),
            CodeLanguage::Cpp => "C++".to_owned(),
            CodeLanguage::Other(name) => name.clone(),
        }
    }

    pub fn all() -> Vec<CodeLanguage> {
        vec![
            CodeLanguage::Cs,
            CodeLanguage::Vb,
            CodeLanguage::Cpp,
            CodeLanguage::Other("F#".to_owned()),
        ]
    }
}

/// A bookmarked topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    /// Content-set path the bookmark belongs to.
    pub source: String,
    pub topic_id: String,
    pub title: String,
    pub folder: String,
}

/// A visited topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub source: String,
    pub topic_id: String,
    pub title: String,
    /// Unix timestamp (seconds).
    pub at: u64,
}

/// Everything Lucid remembers between runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub recent: Vec<String>,
    pub bookmarks: Vec<Bookmark>,
    pub history: Vec<HistoryEntry>,
    pub theme: ThemeChoice,
    pub font_scale: f32,
    pub code_language: CodeLanguage,
    pub sidebar_width: f32,
    pub outline_width: f32,
    pub show_outline: bool,
    pub sidebar_tab: String,
    pub search_all_packages: bool,
    pub last_topic: HashMap<String, String>,
    pub window: Option<[f32; 2]>,
    /// Sidebar entries that are expanded (TOC node titles), per source.
    pub expanded: HashMap<String, Vec<String>>,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            recent: Vec::new(),
            bookmarks: Vec::new(),
            history: Vec::new(),
            theme: ThemeChoice::Dark,
            font_scale: 1.0,
            code_language: CodeLanguage::default(),
            sidebar_width: 300.0,
            outline_width: 200.0,
            show_outline: true,
            sidebar_tab: "contents".to_owned(),
            search_all_packages: true,
            last_topic: HashMap::new(),
            window: None,
            expanded: HashMap::new(),
        }
    }
}

impl AppState {
    /// Where the state file lives.
    pub fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("lucid").join("state.json"))
    }

    /// Load the state, falling back to defaults when it is missing or unreadable.
    pub fn load() -> AppState {
        let Some(path) = AppState::path() else {
            return AppState::default();
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return AppState::default();
        };
        serde_json::from_slice::<AppState>(&bytes).unwrap_or_else(|_| AppState::default())
    }

    /// Write the state (best effort: failures are ignored on purpose).
    pub fn save(&self) {
        let Some(path) = AppState::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Note that a content set was opened.
    pub fn touch_recent(&mut self, source: &str) {
        self.recent.retain(|entry| entry != source);
        self.recent.insert(0, source.to_owned());
        self.recent.truncate(10);
    }

    /// Remember the last topic of a content set.
    ///
    /// The title is stored too: history rows are labelled with it, and an empty title used to
    /// leave the Favorites pane listing blank, unclickable-looking rows.
    pub fn remember_topic(&mut self, source: &str, topic_id: &str, title: &str) {
        self.last_topic
            .insert(source.to_owned(), topic_id.to_owned());
        self.add_history(source, topic_id, title);
    }

    /// Add a history entry (deduplicated by topic, newest first).
    pub fn add_history(&mut self, source: &str, topic_id: &str, title: &str) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        self.history.retain(|entry| {
            !(entry.source == source && entry.topic_id == topic_id)
        });
        self.history.insert(
            0,
            HistoryEntry {
                source: source.to_owned(),
                topic_id: topic_id.to_owned(),
                title: title.to_owned(),
                at,
            },
        );
        self.history.truncate(200);
    }

    /// Is this topic bookmarked?
    pub fn is_bookmarked(&self, source: &str, topic_id: &str) -> bool {
        self.bookmarks
            .iter()
            .any(|bookmark| bookmark.source == source && bookmark.topic_id == topic_id)
    }

    /// Toggle a bookmark, returning true when it was added.
    pub fn toggle_bookmark(&mut self, source: &str, topic_id: &str, title: &str) -> bool {
        if self.is_bookmarked(source, topic_id) {
            self.bookmarks
                .retain(|bookmark| !(bookmark.source == source && bookmark.topic_id == topic_id));
            false
        } else {
            self.bookmarks.push(Bookmark {
                source: source.to_owned(),
                topic_id: topic_id.to_owned(),
                title: title.to_owned(),
                folder: String::new(),
            });
            true
        }
    }

    /// Bookmarks belonging to one content set (optionally one folder).
    pub fn bookmarks_for(&self, source: &str) -> Vec<&Bookmark> {
        self.bookmarks
            .iter()
            .filter(|bookmark| bookmark.source == source)
            .collect()
    }

    /// Folders used by the bookmarks of a content set, sorted.
    pub fn bookmark_folders(&self, source: &str) -> Vec<String> {
        let mut folders: Vec<String> = self
            .bookmarks
            .iter()
            .filter(|bookmark| bookmark.source == source)
            .map(|bookmark| bookmark.folder.clone())
            .collect();
        folders.sort();
        folders.dedup();
        folders
    }

    /// History entries for one content set, newest first.
    pub fn history_for(&self, source: &str) -> Vec<&HistoryEntry> {
        self.history
            .iter()
            .filter(|entry| entry.source == source)
            .collect()
    }

    /// Expanded TOC nodes of a content set.
    pub fn expanded_for(&self, source: &str) -> Vec<String> {
        self.expanded.get(source).cloned().unwrap_or_default()
    }

    /// Store the expanded TOC nodes of a content set.
    pub fn set_expanded(&mut self, source: &str, nodes: Vec<String>) {
        self.expanded.insert(source.to_owned(), nodes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_are_deduplicated_and_capped() {
        let mut state = AppState::default();
        for index in 0..15 {
            state.touch_recent(&format!("book{index}.mshc"));
        }
        assert_eq!(state.recent.len(), 10);
        assert_eq!(state.recent[0], "book14.mshc");
        state.touch_recent("book5.mshc");
        assert_eq!(state.recent[0], "book5.mshc");
        assert_eq!(state.recent.iter().filter(|entry| *entry == "book5.mshc").count(), 1);
    }

    #[test]
    fn bookmarks_toggle_and_group() {
        let mut state = AppState::default();
        assert!(state.toggle_bookmark("a.mshc", "Topic", "Topic title"));
        assert!(state.is_bookmarked("a.mshc", "Topic"));
        assert_eq!(state.bookmarks_for("a.mshc").len(), 1);
        assert!(!state.toggle_bookmark("a.mshc", "Topic", "Topic title"));
        assert!(!state.is_bookmarked("a.mshc", "Topic"));
    }

    #[test]
    fn history_keeps_the_topic_title() {
        let mut state = AppState::default();
        state.remember_topic("a.mshc", "intro", "Introduction to Lucid");
        let history = state.history_for("a.mshc");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "Introduction to Lucid");
        assert_eq!(state.last_topic.get("a.mshc").map(String::as_str), Some("intro"));
    }

    #[test]
    fn history_is_newest_first_and_bounded() {
        let mut state = AppState::default();
        for index in 0..250 {
            state.add_history("a.mshc", &format!("T{index}"), "title");
        }
        assert_eq!(state.history.len(), 200);
        assert_eq!(state.history[0].topic_id, "T249");
        state.add_history("a.mshc", "T100", "title");
        assert_eq!(state.history.len(), 200);
        assert_eq!(state.history.iter().filter(|e| e.topic_id == "T100").count(), 1);
    }

    #[test]
    fn state_round_trips_through_json() {
        let mut state = AppState::default();
        state.touch_recent("book.mshc");
        state.toggle_bookmark("book.mshc", "Id", "Title");
        state.remember_topic("book.mshc", "Id", "Title");
        state.theme = ThemeChoice::Light;
        let json = serde_json::to_vec(&state).expect("serialize");
        let restored: AppState = serde_json::from_slice(&json).expect("deserialize");
        assert_eq!(restored.recent, state.recent);
        assert_eq!(restored.bookmarks, state.bookmarks);
        assert_eq!(restored.theme, ThemeChoice::Light);
        assert_eq!(restored.last_topic.get("book.mshc").map(String::as_str), Some("Id"));
    }

    #[test]
    fn theme_toggles() {
        assert_eq!(ThemeChoice::Dark.toggled(), ThemeChoice::Light);
        assert_eq!(ThemeChoice::Light.toggled(), ThemeChoice::Dark);
    }
}
