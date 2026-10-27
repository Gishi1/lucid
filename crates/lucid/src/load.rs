//! Loading content sets and building search indexes — always off the UI thread.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use lucid_mshc::{DocSet, Progress, TopicKey};
use lucid_search::{DocInput, Index, IndexBuilder, IndexStats};

use crate::cli::index_options_for;
use crate::index_cache;

/// A fully loaded content set, ready to browse.
pub struct Loaded {
    pub set: DocSet,
    pub index: Option<Index>,
    /// True when the index came from the cache (shown in the status bar).
    pub index_from_cache: bool,
    pub index_stats: Option<IndexStats>,
    /// Topics that could not be read while indexing.
    pub warnings: Vec<String>,
}

/// Messages a loader thread sends to the UI.
pub enum LoadMessage {
    Progress(Progress),
    Done(Box<Loaded>),
    Failed(String),
}

/// Spawn a loader thread: open the content set, load or build the index.
pub fn spawn(path: PathBuf, no_index: bool) -> Receiver<LoadMessage> {
    let (sender, receiver) = channel();
    std::thread::Builder::new()
        .name("lucid-load".to_owned())
        .spawn(move || load_into(path, no_index, sender))
        .ok();
    receiver
}

fn load_into(path: PathBuf, no_index: bool, sender: Sender<LoadMessage>) {
    let progress_sender = sender.clone();
    let mut progress = move |update: Progress| {
        let _ = progress_sender.send(LoadMessage::Progress(update));
    };

    let set = match DocSet::open(&path, &mut progress) {
        Ok(set) => set,
        Err(err) => {
            let _ = sender.send(LoadMessage::Failed(err.to_string()));
            return;
        }
    };

    let mut index = None;
    let mut from_cache = false;
    let mut stats = None;
    let mut warnings = set.warnings.clone();

    if !no_index {
        progress(Progress::new(lucid_mshc::Stage::Indexing, 0, 1, "cache"));
        if let Some(cached) = index_cache::load(&set) {
            stats = Some(cached.stats());
            index = Some(cached);
            from_cache = true;
        } else {
            let (built, build_stats) = build_index_with(&set, &mut progress);
            let _ = index_cache::save(&set, &built);
            stats = Some(build_stats);
            index = Some(built);
        }
    }
    warnings.retain(|warning| !warning.is_empty());

    let _ = sender.send(LoadMessage::Done(Box::new(Loaded {
        set,
        index,
        index_from_cache: from_cache,
        index_stats: stats,
        warnings,
    })));
}

/// Build the ranked index for a content set (blocking).
pub fn build_index(set: &DocSet) -> (Index, IndexStats) {
    let mut progress = |_update: Progress| {};
    build_index_with(set, &mut progress)
}

/// [`build_index`] with progress reporting.
pub fn build_index_with(
    set: &DocSet,
    progress: &mut dyn FnMut(Progress),
) -> (Index, IndexStats) {
    let paths = topic_paths(set);
    let mut builder = IndexBuilder::new(index_options_for(set));

    let warnings = set.extract_all(progress, |_package, topic, document| {
        let key = TopicKey::new(topic.key.pkg, topic.key.topic);
        let path = paths.get(&key).cloned().unwrap_or_else(|| topic.title.clone());
        let keywords = join_keywords(&topic.keywords, &topic.f1, topic.description.as_deref());
        builder.add(DocInput {
            title: &topic.title,
            headings: &document.headings_text,
            keywords: &keywords,
            body: &document.text,
            code: &document.code_text,
            path: &path,
            weight: 1.0,
        });
    });

    let index = builder.build();
    let stats = index.stats();
    for warning in &warnings {
        progress(Progress::new(
            lucid_mshc::Stage::Indexing,
            1,
            1,
            warning.clone(),
        ));
    }
    (index, stats)
}

/// Breadcrumb path ("Guides › Search › Queries") for every topic in the TOC, in one pass.
pub fn topic_paths(set: &DocSet) -> HashMap<TopicKey, String> {
    let mut out: HashMap<TopicKey, String> = HashMap::new();
    fn walk(
        nodes: &[lucid_mshc::TocNode<TopicKey>],
        prefix: &mut Vec<String>,
        out: &mut HashMap<TopicKey, String>,
    ) {
        for node in nodes {
            prefix.push(node.title.clone());
            if let Some(key) = node.key {
                out.insert(key, prefix.join(" › "));
            }
            walk(&node.children, prefix, out);
            prefix.pop();
        }
    }
    let mut prefix = Vec::new();
    walk(&set.toc, &mut prefix, &mut out);
    out
}

/// Keywords for the search index: index keywords, F1 keywords and the description.
fn join_keywords(keywords: &[String], f1: &[String], description: Option<&str>) -> String {
    let mut out = String::new();
    for keyword in keywords.iter().chain(f1.iter()) {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(keyword);
    }
    if let Some(description) = description {
        if !description.trim().is_empty() {
            if !out.is_empty() {
                out.push_str(", ");
            }
            out.push_str(description);
        }
    }
    out
}


