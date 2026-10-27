//! Progress reporting for long-running loads (used by the GUI status bar and the CLI).

/// Coarse stages of opening a content set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Finding packages (folder / `.msha` setup input).
    Discovering,
    /// Reading the zip directory of one package.
    Opening,
    /// Parsing topic metadata.
    Scanning,
    /// Reconstructing the table of contents.
    BuildingToc,
    /// Extracting text for the search index.
    Extracting,
    /// Building the ranked index.
    Indexing,
    /// Finished.
    Done,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Discovering => "Discovering packages",
            Stage::Opening => "Opening package",
            Stage::Scanning => "Scanning topics",
            Stage::BuildingToc => "Building contents",
            Stage::Extracting => "Reading topics",
            Stage::Indexing => "Building search index",
            Stage::Done => "Ready",
        }
    }
}

/// One progress update.
#[derive(Debug, Clone)]
pub struct Progress {
    pub stage: Stage,
    pub done: usize,
    pub total: usize,
    pub detail: String,
}

impl Progress {
    pub fn new(stage: Stage, done: usize, total: usize, detail: impl Into<String>) -> Self {
        Progress {
            stage,
            done,
            total,
            detail: detail.into(),
        }
    }

    /// Completion in `0.0..=1.0` (`None` when the total is unknown).
    pub fn fraction(&self) -> Option<f32> {
        if self.total == 0 {
            None
        } else {
            Some((self.done as f32 / self.total as f32).clamp(0.0, 1.0))
        }
    }

    /// `"Scanning topics — 120/340 (foo.mshc)"`.
    pub fn display(&self) -> String {
        let mut out = self.stage.label().to_owned();
        if self.total > 0 {
            out.push_str(&format!(" — {}/{}", self.done, self.total));
        }
        if !self.detail.is_empty() {
            out.push_str(" (");
            out.push_str(&self.detail);
            out.push(')');
        }
        out
    }
}

/// Callback type used throughout the crate.
pub type ProgressFn<'a> = &'a mut dyn FnMut(Progress);
