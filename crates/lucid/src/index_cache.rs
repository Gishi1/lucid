//! The persisted search-index cache.
//!
//! Path: `$XDG_CACHE_HOME/lucid/index/<fingerprint>.idx`, where the fingerprint covers the
//! package paths, sizes, modification times and the index format version (see
//! `DocSet::fingerprint`). Any change invalidates the entry, so a stale index can never be
//! served silently.

use std::path::PathBuf;

use lucid_mshc::DocSet;
use lucid_search::Index;

/// Directory holding cached indexes.
pub fn cache_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("lucid").join("index"))
}

/// Cache file for a content set.
pub fn cache_path(set: &DocSet) -> Option<PathBuf> {
    Some(cache_dir()?.join(format!("{}.idx", set.fingerprint())))
}

/// Load the cached index for a content set, if it exists and matches this build.
pub fn load(set: &DocSet) -> Option<Index> {
    let path = cache_path(set)?;
    match Index::load(&path) {
        Ok(index) => {
            // A cache built for a different document count is useless (and suspicious).
            if index.doc_count() == set.topic_count() {
                Some(index)
            } else {
                let _ = std::fs::remove_file(&path);
                None
            }
        }
        Err(_) => {
            // Corrupt or from an older format: drop it and let the caller rebuild.
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

/// Persist an index for a content set.
pub fn save(set: &DocSet, index: &Index) -> std::io::Result<()> {
    let Some(path) = cache_path(set) else {
        return Ok(());
    };
    index.save(&path)
}

/// Forget the cached index of a content set (used by "Rebuild index").
pub fn clear(set: &DocSet) {
    if let Some(path) = cache_path(set) {
        let _ = std::fs::remove_file(path);
    }
}

/// Total size of the cache directory, for the settings pane.
pub fn cache_size() -> u64 {
    let Some(dir) = cache_dir() else {
        return 0;
    };
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.metadata().ok())
                .map(|metadata| metadata.len())
                .sum()
        })
        .unwrap_or(0)
}
