//! The application: toolbar, browser panes, tabs, topic view, outline and status bar.
//!
//! Interaction model
//! -----------------
//! Drawing never mutates application state directly: every pane pushes [`Action`] values that
//! are applied once per frame, after the UI has been drawn. That keeps the borrow checker
//! happy (panes borrow `self` immutably while the action list is built) and makes the
//! behaviour of every widget visible in one place ([`LucidApp::apply`]).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::Receiver;

use egui::{Align, Color32, Layout, RichText, ScrollArea, Sense, Ui, Vec2};

use lucid_mshc::{DocSet, Document, LinkTarget, Progress, Stage, TocNode, TopicKey};
use lucid_search::{field_mask_for, Hit, Index, SearchOptions, SearchResults, Sort};

use crate::load::{self, LoadMessage, Loaded};
use crate::quickopen::{self, Entry as QuickEntry, Match as QuickMatch};
use crate::render::{AssetCache, FindPattern, FindState, RenderAction, Renderer, ViewState};
use crate::state::{AppState, CodeLanguage, ThemeChoice};
use crate::theme;

/// Below this window width the outline pane hides itself (the document needs the room).
const OUTLINE_MIN_WIDTH: f32 = 1080.0;

/// Widest text column. On a 2560 px display an uncapped column runs to ~170 characters per
/// line, which is uncomfortable to read; capping it centres the text instead.
const MAX_DOCUMENT_WIDTH: f32 = 1120.0;

/// Which browser pane is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarTab {
    #[default]
    Contents,
    Index,
    Search,
    Favorites,
}

impl SidebarTab {
    fn label(self) -> &'static str {
        match self {
            SidebarTab::Contents => "Contents",
            SidebarTab::Index => "Index",
            SidebarTab::Search => "Search",
            SidebarTab::Favorites => "Favorites",
        }
    }

    fn id(self) -> &'static str {
        match self {
            SidebarTab::Contents => "contents",
            SidebarTab::Index => "index",
            SidebarTab::Search => "search",
            SidebarTab::Favorites => "favorites",
        }
    }

    fn from_id(id: &str) -> Self {
        match id {
            "index" => SidebarTab::Index,
            "search" => SidebarTab::Search,
            "favorites" => SidebarTab::Favorites,
            _ => SidebarTab::Contents,
        }
    }
}

/// An open topic.
struct Tab {
    key: TopicKey,
    title: String,
    document: Document,
    view: ViewState,
}

/// Everything a frame can ask the application to do.
enum Action {
    Load(PathBuf),
    OpenTopic {
        key: TopicKey,
        anchor: Option<String>,
        highlight: Vec<String>,
    },
    CloseTab(usize),
    ActivateTab(usize),
    CloseAllTabs,
    Reindex,
    SetTheme(ThemeChoice),
    ToggleBookmark,
    Copy(String),
    Message(String),
    SetZoom(f32),
    SetSidebarTab(SidebarTab),
    SetSearchQuery(String),
    ShowRecent,
    ShowOpenDialog,
    CloseDialog,
    OpenDialogFile(PathBuf),
    ClearKeywordSelection,
}

/// What the command line asked for on top of the saved state.
#[derive(Debug, Clone, Default)]
pub struct Startup {
    /// Pane to show first (`--pane contents|index|search|favorites`).
    pub pane: Option<SidebarTab>,
    /// Theme override (`--theme dark|light`).
    pub theme: Option<String>,
}

/// Parse a pane name from the command line.
pub fn pane_from_name(name: &str) -> Option<SidebarTab> {
    match name.to_ascii_lowercase().as_str() {
        "contents" | "toc" => Some(SidebarTab::Contents),
        "index" | "keywords" => Some(SidebarTab::Index),
        "search" => Some(SidebarTab::Search),
        "favorites" | "bookmarks" => Some(SidebarTab::Favorites),
        _ => None,
    }
}

/// How the window should open (and, for verification runs, what should be captured).
#[derive(Debug, Clone)]
pub struct ViewOptions {
    /// Exact window size; `None` means "use the screen" (maximized).
    pub size: Option<[f32; 2]>,
    /// Open maximized (default: yes — a help viewer is read full screen).
    pub maximized: bool,
    /// Write a PNG of the window once the content set is loaded, then exit.
    pub screenshot: Option<PathBuf>,
    /// Extra frames to draw before taking the screenshot (lets panels settle).
    pub frames: u32,
}

impl Default for ViewOptions {
    fn default() -> Self {
        ViewOptions {
            size: None,
            maximized: true,
            screenshot: None,
            frames: 6,
        }
    }
}

/// Pending screenshot request.
#[derive(Debug, Clone)]
struct ScreenshotPlan {
    path: PathBuf,
    frames_to_wait: u32,
    frames_drawn: u32,
    requested: bool,
}

/// In-app file browser (no native dialog dependency).
#[derive(Debug, Clone)]
struct OpenDialog {
    dir: PathBuf,
    entries: Vec<(String, bool)>,
    /// Selected file, if the user clicked one.
    selected: Option<PathBuf>,
    /// Path typed by the user (navigate on Enter / Go).
    typed: String,
}

impl OpenDialog {
    fn new(start: &std::path::Path) -> Self {
        let dir = start.to_path_buf();
        let mut dialog = OpenDialog {
            typed: dir.display().to_string(),
            dir,
            entries: Vec::new(),
            selected: None,
        };
        dialog.refresh();
        dialog
    }

    fn refresh(&mut self) {
        self.entries.clear();
        let Ok(reader) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in reader.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            let ext = entry
                .path()
                .extension()
                .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let interesting = is_dir || matches!(ext.as_str(), "mshc" | "msha" | "mshi" | "cab");
            if interesting {
                self.entries.push((name, is_dir));
            }
        }
        self.entries.sort_by(|a, b| match (a.1, b.1) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.0.to_lowercase().cmp(&b.0.to_lowercase()),
        });
    }

    /// Enter a subdirectory of the current directory.
    fn enter(&mut self, name: &str) {
        let target = self.dir.join(name);
        self.go_to(&target);
    }

    /// Go to the parent directory (a no-op at the filesystem root).
    fn up(&mut self) {
        if let Some(parent) = self.dir.parent().map(std::path::Path::to_path_buf) {
            self.go_to(&parent);
        }
    }

    /// Go to an arbitrary directory.
    fn go_to(&mut self, dir: &std::path::Path) {
        if dir.is_dir() {
            self.dir = dir.to_path_buf();
            self.typed = self.dir.display().to_string();
            self.selected = None;
            self.refresh();
        }
    }
}

/// Navigation requested by the file dialog's widgets.
enum DialogNav {
    Up,
    Into(String),
    To(PathBuf),
}

/// Ctrl+P quick open state.
struct QuickOpen {
    query: String,
    matches: Vec<QuickMatch>,
    selected: usize,
    focus: bool,
}

/// The application.
pub struct LucidApp {
    state: AppState,
    no_index: bool,

    // loading
    loaded: Option<Loaded>,
    loader: Option<Receiver<LoadMessage>>,
    progress: Option<Progress>,
    loading_path: Option<PathBuf>,
    load_error: Option<String>,

    // browsing
    tabs: Vec<Tab>,
    active_tab: usize,
    session_history: Vec<(TopicKey, String)>,
    history_pos: usize,

    // panes
    sidebar_tab: SidebarTab,
    toc_filter: String,
    toc_expanded: HashSet<String>,
    keyword_filter: String,
    keyword_selection: Vec<usize>,
    search_options: SearchOptions,
    search_results: SearchResults,
    search_selected: usize,
    search_all_packages: bool,
    /// The query string bound to the toolbar/search box.
    search_query_cache: String,
    find: Option<FindState>,
    find_matches: Vec<Range<usize>>,

    // quick open
    quick: Option<QuickOpen>,
    quick_entries: Vec<QuickEntry>,

    // misc
    paths: HashMap<TopicKey, String>,
    assets: AssetCache,
    dialog: Option<OpenDialog>,
    status: String,
    toast: Option<(String, f64)>,
    theme_applied: Option<(ThemeChoice, i32)>,
    /// Pane requested on the command line (`--pane`).
    pending_pane: Option<SidebarTab>,
    /// Pending topic from the command line (`--topic`, `--f1`).
    pending_topic: Option<String>,
    pending_keyword: Option<String>,
    pending_search: Option<String>,
    focus_search: bool,
    show_about: bool,
    screenshot: Option<ScreenshotPlan>,
}

impl LucidApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        state: AppState,
        no_index: bool,
        path: Option<PathBuf>,
        topic: Option<String>,
        f1: Option<String>,
        search: Option<String>,
        view: ViewOptions,
        startup: Startup,
    ) -> Self {
        // Read what we need out of the persisted state before it is moved into the struct.
        // `--theme` overrides the saved preference for this session (and is remembered).
        let mut state = state;
        if let Some(theme) = startup.theme.as_deref() {
            state.theme = if theme.eq_ignore_ascii_case("light") {
                ThemeChoice::Light
            } else {
                ThemeChoice::Dark
            };
        }
        state.sidebar_tab = startup
            .pane
            .map(|pane| pane.id().to_owned())
            .unwrap_or_else(|| state.sidebar_tab.clone());
        let sidebar_tab = SidebarTab::from_id(&state.sidebar_tab);
        let mut app = LucidApp {
            state,
            no_index,
            loaded: None,
            loader: None,
            progress: None,
            loading_path: None,
            load_error: None,
            tabs: Vec::new(),
            active_tab: 0,
            session_history: Vec::new(),
            history_pos: 0,
            sidebar_tab,
            toc_filter: String::new(),
            toc_expanded: HashSet::new(),
            keyword_filter: String::new(),
            keyword_selection: Vec::new(),
            search_options: SearchOptions::default(),
            search_results: SearchResults::default(),
            search_selected: 0,
            search_all_packages: true,
            search_query_cache: String::new(),
            find: None,
            find_matches: Vec::new(),
            quick: None,
            quick_entries: Vec::new(),
            paths: HashMap::new(),
            assets: AssetCache::default(),
            dialog: None,
            status: String::new(),
            toast: None,
            theme_applied: None,
            pending_pane: startup.pane,
            pending_topic: topic,
            pending_keyword: f1,
            pending_search: search,
            focus_search: false,
            show_about: false,
            screenshot: view.screenshot.clone().map(|path| ScreenshotPlan {
                path,
                frames_to_wait: view.frames,
                frames_drawn: 0,
                requested: false,
            }),
        };
        theme::apply(&cc.egui_ctx, app.state.theme, app.state.font_scale);
        app.theme_applied = Some((app.state.theme, (app.state.font_scale * 100.0) as i32));
        if let Some(path) = path {
            app.start_load(&path, &cc.egui_ctx);
        }
        app
    }

    // ------------------------------------------------------------------ loading

    fn start_load(&mut self, path: &std::path::Path, ctx: &egui::Context) {
        self.loading_path = Some(path.to_path_buf());
        self.load_error = None;
        self.progress = Some(Progress::new(Stage::Discovering, 0, 0, "starting"));
        self.loader = Some(load::spawn(path.to_path_buf(), self.no_index));
        self.state.touch_recent(&path.display().to_string());
        self.status = format!("opening {}", path.display());
        ctx.request_repaint();
    }

    fn poll_loader(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.loader.as_ref() else {
            return;
        };
        let mut messages = Vec::new();
        while let Ok(message) = receiver.try_recv() {
            messages.push(message);
        }
        let mut done = false;
        let mut failed = None;
        for message in messages {
            match message {
                LoadMessage::Progress(update) => self.progress = Some(update),
                LoadMessage::Done(loaded) => {
                    done = true;
                    self.adopt(*loaded, ctx);
                }
                LoadMessage::Failed(error) => {
                    failed = Some(error);
                }
            }
        }
        if let Some(error) = failed {
            self.load_error = Some(error.clone());
            self.status = format!("could not open the package: {error}");
            self.loading_path = None;
            self.loader = None;
            return;
        }
        if done {
            self.loader = None;
            self.loading_path = None;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
    }

    fn adopt(&mut self, loaded: Loaded, ctx: &egui::Context) {
        // (The loader thread boxes the payload so a large content set does not sit on the
        // thread's stack while it is being sent.)
        let source = loaded.set.path.display().to_string();
        self.paths = load::topic_paths(&loaded.set);
        self.quick_entries = quickopen::entries(&loaded.set, &self.paths);
        self.warnings_status(&loaded);
        // The index line in the status bar already shows terms and cache state; keep this
        // one about the content set itself.
        let packages = loaded.set.packages.len();
        let mut status = match loaded.set.kind {
            lucid_mshc::SourceKind::Package => "single package".to_owned(),
            lucid_mshc::SourceKind::Folder => format!(
                "content set · {packages} package{}",
                if packages == 1 { "" } else { "s" }
            ),
            lucid_mshc::SourceKind::Setup => "setup document".to_owned(),
        };
        if loaded.index.is_none() {
            status.push_str(" · no index (--no-index)");
        }
        self.status = status;
        self.assets.clear();
        self.tabs.clear();
        self.active_tab = 0;
        self.session_history.clear();
        self.search_results = SearchResults::default();
        self.search_selected = 0;
        // Restore the last topic of this source.
        let last_topic = self.state.last_topic.get(&source).cloned();
        // Restore the expanded TOC nodes.
        self.toc_expanded = self.state.expanded_for(&source).into_iter().collect();
        self.loaded = Some(loaded);

        // Command-line requests first, then the remembered topic, then the first TOC node.
        let mut opened = false;
        if let Some(id) = self.pending_topic.take() {
            opened = self.open_by_id(&id, None, Vec::new());
            if !opened {
                self.toast = Some((format!("no topic with id `{id}`"), now()));
            }
        }
        if !opened {
            if let Some(keyword) = self.pending_keyword.take() {
                opened = self.open_by_keyword(&keyword);
                if !opened {
                    self.toast = Some((format!("no topic for F1 keyword `{keyword}`"), now()));
                }
            }
        }
        if !opened {
            if let Some(id) = last_topic {
                opened = self.open_by_id(&id, None, Vec::new());
            }
        }
        if !opened {
            if let Some(key) = self.first_topic() {
                self.open_topic(key, None, Vec::new());
            }
        }
        if let Some(query) = self.pending_search.take() {
            self.set_search_query(&query);
            self.sidebar_tab = SidebarTab::Search;
        }
        if let Some(pane) = self.pending_pane.take() {
            self.sidebar_tab = pane;
        }
        ctx.request_repaint();
    }

    fn warnings_status(&mut self, loaded: &Loaded) {
        let count = loaded.warnings.len()
            + loaded
                .set
                .packages
                .iter()
                .map(|package| package.warnings.len())
                .sum::<usize>();
        if count > 0 {
            self.toast = Some((format!("{count} warning(s) — see `lucid info`"), now()));
        }
    }

    fn set(&self) -> Option<&DocSet> {
        self.loaded.as_ref().map(|loaded| &loaded.set)
    }

    fn index(&self) -> Option<&Index> {
        self.loaded.as_ref().and_then(|loaded| loaded.index.as_ref())
    }

    fn source_id(&self) -> String {
        self.set()
            .map(|set| set.path.display().to_string())
            .unwrap_or_default()
    }

    fn first_topic(&self) -> Option<TopicKey> {
        let set = self.set()?;
        for node in &set.toc {
            if let Some(key) = first_key(node) {
                return Some(key);
            }
        }
        None
    }

    fn open_by_id(&mut self, id: &str, anchor: Option<String>, highlight: Vec<String>) -> bool {
        let Some(set) = self.set() else {
            return false;
        };
        match set.find_by_id(id) {
            Some(key) => {
                self.open_topic(key, anchor, highlight);
                true
            }
            None => false,
        }
    }

    fn open_by_keyword(&mut self, keyword: &str) -> bool {
        let Some(key) = self
            .set()
            .and_then(|set| set.find_by_f1(keyword).or_else(|| set.find_by_id(keyword)))
        else {
            return false;
        };
        self.open_topic(key, None, Vec::new());
        true
    }

    /// Open a topic (reusing the tab when it is already open).
    fn open_topic(&mut self, key: TopicKey, anchor: Option<String>, highlight: Vec<String>) {
        let Some(set) = self.set() else {
            return;
        };
        let document = match set.document(key) {
            Ok(document) => document,
            Err(error) => {
                self.toast = Some((format!("could not read the topic: {error}"), now()));
                return;
            }
        };
        let title = set.topic_title(key).to_owned();
        let id = set.topic_id(key).to_owned();

        if let Some(index) = self.tabs.iter().position(|tab| tab.key == key) {
            self.active_tab = index;
            let tab = &mut self.tabs[index];
            tab.document = document;
            tab.view.scroll_to_anchor = anchor;
            if !highlight.is_empty() {
                tab.view.highlight_terms = highlight;
            }
        } else {
            let mut view = ViewState::new();
            view.scroll_to_anchor = anchor;
            view.highlight_terms = highlight;
            view.scale = self.tabs.last().map(|tab| tab.view.scale).unwrap_or(1.0);
            self.tabs.push(Tab {
                key,
                title: title.clone(),
                document,
                view,
            });
            self.active_tab = self.tabs.len() - 1;
        }
        // Session history (browser-style back/forward).
        self.session_history.truncate(self.history_pos);
        self.session_history.push((key, title.clone()));
        self.history_pos = self.session_history.len();
        if !id.is_empty() {
            self.state.remember_topic(&self.source_id(), &id, &title);
        }
        self.recompute_find();
    }

    fn navigate(&mut self, target: LinkTarget) {
        let Some(set) = self.set() else {
            return;
        };
        let from = match self.tabs.get(self.active_tab) {
            Some(tab) => tab.key,
            None => return,
        };
        match target {
            LinkTarget::Topic(key) => self.open_topic(key, None, Vec::new()),
            LinkTarget::Anchor(key, anchor) => {
                let anchor = if anchor.is_empty() { None } else { Some(anchor) };
                self.open_topic(key, anchor, Vec::new());
            }
            LinkTarget::External(url) => {
                if let Err(error) = open_external(&url) {
                    self.toast = Some((format!("could not open {url}: {error}"), now()));
                }
            }
            // `RenderAction::Link` hands us the raw href: resolve it here, because only the
            // content set knows how a reference maps to a topic.
            LinkTarget::Missing(href) => match set.resolve(from, &href) {
                LinkTarget::Missing(_) => {
                    self.toast = Some((format!("link target not found: {href}"), now()))
                }
                other => self.navigate(other),
            },
        }
    }

    // ------------------------------------------------------------------ search

    fn set_search_query(&mut self, query: &str) {
        if self.index().is_none() {
            self.search_results = SearchResults::default();
            return;
        }
        let mut options = self.search_options.clone();
        // Search-as-you-type: the last term is a prefix until the user picks a result.
        options.prefix_last_term = true;
        if !self.search_all_packages {
            let package = self.tabs.get(self.active_tab).map(|tab| tab.key.pkg);
            options.scope = package.and_then(|package| self.scope_for_package(package));
        }
        let results = match self.index() {
            Some(index) => index.search(query, &options),
            None => SearchResults::default(),
        };
        self.search_options.prefix_last_term = true;
        self.search_results = results;
        self.search_selected = 0;
    }

    /// Index document ids of one package (documents are numbered in package order).
    fn scope_for_package(&self, package: u32) -> Option<Vec<u32>> {
        let set = self.set()?;
        if set.packages.len() <= 1 {
            return None;
        }
        let mut scope = Vec::new();
        let mut offset = 0u32;
        for (index, candidate) in set.packages.iter().enumerate() {
            let count = candidate.topics.len() as u32;
            if index as u32 == package {
                scope.extend(offset..offset + count);
            }
            offset += count;
        }
        Some(scope)
    }

    fn open_hit(&mut self, hit: &Hit) {
        let Some(set) = self.set() else {
            return;
        };
        // Index document ids follow the flattened topic order of the content set.
        let mut offset = 0u32;
        for (package_index, package) in set.packages.iter().enumerate() {
            let count = package.topics.len() as u32;
            if hit.doc < offset + count {
                let key = TopicKey::new(package_index as u32, hit.doc - offset);
                let terms = self.search_results.terms.clone();
                self.open_topic(key, None, terms);
                return;
            }
            offset += count;
        }
    }

    // ------------------------------------------------------------------ find in topic

    fn recompute_find(&mut self) {
        self.find_matches.clear();
        let Some(find) = self.find.as_ref() else {
            return;
        };
        if find.needle.trim().is_empty() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return;
        };
        self.find_matches = find_in_text(&tab.document.text, &find.needle, find.case_sensitive);
        if let Some(find) = self.find.as_mut() {
            if find.current >= self.find_matches.len() {
                find.current = 0;
            }
        }
    }

    fn find_step(&mut self, forward: bool) {
        if self.find_matches.is_empty() {
            return;
        }
        let count = self.find_matches.len();
        if let Some(find) = self.find.as_mut() {
            find.current = if forward {
                (find.current + 1) % count
            } else {
                (find.current + count - 1) % count
            };
        }
        // Scroll to the match.
        if let (Some(find), Some(index)) = (self.find.as_ref(), self.tabs.get_mut(self.active_tab))
        {
            let offset = self
                .find_matches
                .get(find.current)
                .map(|range| range.start)
                .unwrap_or(0);
            index.view.scroll_to_offset = Some(offset);
        }
    }

    // ------------------------------------------------------------------ drawing

    /// The toolbar: a row of controls plus a full-width search row.
    ///
    /// Two rows rather than one because at the minimum window size a single row either
    /// overflows or squeezes the search box into an unusable width (see the layout checks in
    /// `docs/DESIGN.md` §6).
    fn toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.add_space(3.0);
        self.toolbar_controls(ui, actions);
        ui.add_space(3.0);
        self.toolbar_search(ui, actions);
        ui.add_space(3.0);
    }

    fn toolbar_controls(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Lucid").strong().size(16.0));
            ui.separator();

            ui.menu_button("Open", |ui| {
                if ui.button("Open file or folder…").clicked() {
                    actions.push(Action::ShowOpenDialog);
                    ui.close();
                }
                if ui.button("Recent…").clicked() {
                    actions.push(Action::ShowRecent);
                    ui.close();
                }
                if ui.button("Reload current").clicked() {
                    if let Some(path) = self.loading_path.clone().or_else(|| {
                        self.set().map(|set| set.path.clone())
                    }) {
                        actions.push(Action::Load(path));
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            let can_act = self.set().is_some();
            ui.add_enabled_ui(can_act, |ui| {
                if ui.button("Reindex").on_hover_text("Rebuild the search index (F5)").clicked() {
                    actions.push(Action::Reindex);
                }
            });

            ui.separator();
            let quick_label = self
                .quick
                .as_ref()
                .map(|quick| {
                    if quick.query.is_empty() {
                        "Go to topic (Ctrl+P)".to_owned()
                    } else {
                        format!("🔍 {}", quick.query)
                    }
                })
                .unwrap_or_else(|| "Go to topic (Ctrl+P)".to_owned());
            if ui.button(quick_label).clicked() {
                actions.push(Action::Message("quick-open".to_owned()));
            }

            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                let theme_label = match self.state.theme {
                    ThemeChoice::Dark => "☀",
                    ThemeChoice::Light => "🌙",
                };
                if ui
                    .button(theme_label)
                    .on_hover_text(format!(
                        "Theme: {} — click to switch",
                        self.state.theme.label()
                    ))
                    .clicked()
                {
                    actions.push(Action::SetTheme(self.state.theme.toggled()));
                }
                // Preferred code language for snippet tabs and LanguageSpecificText.
                let mut language = self.state.code_language.clone();
                egui::ComboBox::from_id_salt("code-language")
                    .selected_text(RichText::new(language.label()).small())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for option in CodeLanguage::all() {
                            if ui
                                .selectable_label(option == language, option.label())
                                .clicked()
                            {
                                language = option.clone();
                            }
                        }
                    });
                if language != self.state.code_language {
                    self.state.code_language = language;
                    self.state.save();
                }
                let outline_shown = self.outline_visible(ui.ctx());
                let outline_hint = if self.state.show_outline && !outline_shown {
                    "The outline is hidden automatically in narrow windows"
                } else {
                    "Show the outline of the current topic"
                };
                if ui
                    .selectable_label(outline_shown, "Outline")
                    .on_hover_text(outline_hint)
                    .clicked()
                {
                    self.state.show_outline = !self.state.show_outline;
                }
                // Added in reverse: this is a right-to-left layout, so "+" is added first
                // to end up on the right of "−".
                if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                    let scale = (self.current_scale() + 0.1).min(1.8);
                    actions.push(Action::SetZoom(scale));
                }
                if ui.small_button("−").on_hover_text("Zoom out").clicked() {
                    let scale = (self.current_scale() - 0.1).max(0.8);
                    actions.push(Action::SetZoom(scale));
                }
                if ui
                    .small_button("⟲")
                    .on_hover_text("Reset zoom (Ctrl+0)")
                    .clicked()
                {
                    actions.push(Action::SetZoom(1.0));
                }
                ui.label(
                    RichText::new(format!("{:.0}%", self.current_scale() * 100.0))
                        .small()
                        .color(theme::muted(ui.ctx())),
                );
            });
        });
    }

    /// The global search box (Ctrl+K focuses it), always on its own full-width row.
    fn toolbar_search(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let mut query = self.search_query_cache.clone();
        let response = ui.add_sized(
            [ui.available_width(), 26.0],
            egui::TextEdit::singleline(&mut query)
                .hint_text("Search (Ctrl+K) — try: \"phrase\", term*, field:value, -exclude"),
        );
        if response.changed() {
            actions.push(Action::SetSearchQuery(query.clone()));
        }
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            if let Some(hit) = self.search_results.hits.first().cloned() {
                self.open_hit(&hit);
            }
        }
        if self.focus_search {
            response.request_focus();
            self.focus_search = false;
        }
    }

    fn search_query(&self) -> String {
        self.search_query_cache.clone()
    }

    fn current_scale(&self) -> f32 {
        self.tabs
            .get(self.active_tab)
            .map(|tab| tab.view.scale)
            .unwrap_or(1.0)
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let (source, topics) = match self.set() {
                Some(set) => (
                    set.path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| set.path.display().to_string()),
                    set.topic_count(),
                ),
                None => ("no content".to_owned(), 0),
            };
            ui.label(RichText::new(source).small());
            ui.separator();
            ui.label(RichText::new(format!("{topics} topics")).small());
            if let Some(loaded) = self.loaded.as_ref() {
                if let Some(stats) = loaded.index_stats {
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "index {} terms{}{}",
                            stats.terms,
                            if loaded.index_from_cache { " (cached)" } else { "" },
                            if self.search_results.took_us > 0 {
                                format!(" · last query {} µs", self.search_results.took_us)
                            } else {
                                String::new()
                            }
                        ))
                        .small(),
                    );
                }
            }
            if let Some(progress) = self.progress.clone() {
                if self.loading_path.is_some() {
                    ui.separator();
                    ui.label(RichText::new(progress.display()).small().color(theme::ACCENT));
                    if let Some(fraction) = progress.fraction() {
                        ui.add(
                            egui::ProgressBar::new(fraction)
                                .desired_width(120.0)
                                .desired_height(6.0),
                        );
                    }
                }
            }
            if !self.status.is_empty() {
                ui.separator();
                ui.label(RichText::new(self.status.clone()).small());
            }
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                let zoom = self.tabs.get(self.active_tab).map(|tab| {
                    let id = self
                        .set()
                        .map(|set| set.topic_id(tab.key).to_owned())
                        .unwrap_or_default();
                    (tab.title.clone(), id)
                });
                if let Some((title, id)) = zoom {
                    // `Microsoft.Help.Id` strings are long; without `truncate` the label
                    // grew leftwards over the source/index labels at 900px.
                    ui.add(
                        egui::Label::new(
                            RichText::new(if id.is_empty() { title } else { id })
                                .small()
                                .color(theme::muted(ui.ctx())),
                        )
                        .truncate(),
                    );
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for tab in [
                SidebarTab::Contents,
                SidebarTab::Index,
                SidebarTab::Search,
                SidebarTab::Favorites,
            ] {
                if ui
                    .selectable_label(self.sidebar_tab == tab, tab.label())
                    .clicked()
                {
                    actions.push(Action::SetSidebarTab(tab));
                }
            }
        });
        ui.separator();
        match self.sidebar_tab {
            SidebarTab::Contents => self.contents_pane(ui, actions),
            SidebarTab::Index => self.index_pane(ui, actions),
            SidebarTab::Search => self.search_pane(ui, actions),
            SidebarTab::Favorites => self.favorites_pane(ui, actions),
        }
    }

    fn contents_pane(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if self.set().is_none() {
            ui.label(RichText::new("Open a package to browse its contents.").color(theme::muted(ui.ctx())));
            return;
        }
        // Buttons first, right-to-left, so the text field takes exactly the room that is
        // left. Reserving a guessed number of pixels instead made the row a couple of
        // pixels wider than the pane every frame, and egui then widened the side panel a
        // little more on each repaint.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("None").on_hover_text("Collapse everything").clicked() {
                    self.toc_expanded.clear();
                }
                if ui.small_button("All").on_hover_text("Expand everything").clicked() {
                    let titles: Vec<String> = self
                        .set()
                        .map(|set| all_titles(&set.toc))
                        .unwrap_or_default();
                    self.toc_expanded.extend(titles);
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.toc_filter)
                        .hint_text("Filter contents")
                        .desired_width(ui.available_width().max(60.0)),
                );
            });
        });
        ui.add_space(4.0);

        let filter = self.toc_filter.trim().to_lowercase();
        let nodes = match self.set() {
            Some(set) => set.toc.clone(),
            None => return,
        };
        let active = self.tabs.get(self.active_tab).map(|tab| tab.key);
        let expanded = self.toc_expanded.clone();
        let mut new_expanded = expanded.clone();

        ScrollArea::vertical()
            .id_salt("toc-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for node in &nodes {
                    self.toc_row(ui, node, 0, &filter, active, &expanded, &mut new_expanded, actions);
                }
            });
        if new_expanded != self.toc_expanded {
            self.toc_expanded = new_expanded;
            let source = self.source_id();
            let expanded: Vec<String> = self.toc_expanded.iter().cloned().collect();
            self.state.set_expanded(&source, expanded);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn toc_row(
        &self,
        ui: &mut Ui,
        node: &TocNode<TopicKey>,
        depth: usize,
        filter: &str,
        active: Option<TopicKey>,
        expanded: &HashSet<String>,
        new_expanded: &mut HashSet<String>,
        actions: &mut Vec<Action>,
    ) {
        let is_open = expanded.contains(&node.title);
        let children_visible = filter.is_empty() || subtree_matches(node, filter);

        // Filtering keeps ancestors of matches visible.
        if !children_visible {
            return;
        }

        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 12.0);
            if node.children.is_empty() {
                ui.add_space(16.0);
            } else {
                // U+23F7/U+23F5: the small triangles that the bundled fonts actually have
                // (U+25BE/U+25B8 render as empty boxes). See `docs/DESIGN.md` §6.
                let arrow = if is_open { "⏷" } else { "⏵" };
                if ui
                    .add(egui::Button::new(arrow).small().frame(false))
                    .clicked()
                {
                    if is_open {
                        new_expanded.remove(&node.title);
                    } else {
                        new_expanded.insert(node.title.clone());
                    }
                }
            }
            let selected = node.key.is_some() && node.key == active;
            let highlight = filter_matches(&node.title, filter);
            let mut text = RichText::new(&node.title).size(13.5);
            if node.synthetic {
                text = text.italics().color(theme::muted(ui.ctx()));
            }
            if highlight && !filter.is_empty() {
                text = text.strong().color(theme::ACCENT);
            }
            if selected {
                text = text.color(theme::strong_text(ui.ctx()));
            }
            // A tree label must not be wider than the pane: it makes the side panel's
            // content rect overflow, which (in egui) pushes the next panel away and leaves
            // an unpainted gap at narrow window widths.
            let response = ui
                .add(egui::Button::selectable(selected, text).truncate())
                // The tree is narrow; the tooltip still spells the whole title out.
                .on_hover_text(&node.title);
            if response.clicked() {
                match node.key {
                    Some(key) => actions.push(Action::OpenTopic {
                        key,
                        anchor: None,
                        highlight: Vec::new(),
                    }),
                    None => {
                        if is_open {
                            new_expanded.remove(&node.title);
                        } else {
                            new_expanded.insert(node.title.clone());
                        }
                    }
                }
            }
            if response.double_clicked() {
                if let Some(key) = node.key {
                    actions.push(Action::OpenTopic {
                        key,
                        anchor: None,
                        highlight: Vec::new(),
                    });
                }
            }
        });

        let show_children = is_open || (!filter.is_empty() && children_visible);
        if show_children {
            for child in &node.children {
                self.toc_row(
                    ui,
                    child,
                    depth + 1,
                    filter,
                    active,
                    expanded,
                    new_expanded,
                    actions,
                );
            }
        }
    }

    fn index_pane(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if self.set().is_none() {
            ui.label(
                RichText::new("Open a package to browse its index.").color(theme::muted(ui.ctx())),
            );
            return;
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.keyword_filter)
                    .hint_text("Filter keywords")
                    .desired_width(ui.available_width() - 20.0),
            );
        });
        // Collect everything the list needs, then drop the borrow of the content set.
        let (keyword_count, matches, selected_keys) = {
            let Some(set) = self.set() else { return };
            let needle = self.keyword_filter.trim().to_lowercase();
            let matches: Vec<(usize, String, bool, TopicKey)> = set
                .keywords
                .iter()
                .enumerate()
                .filter(|(_, entry)| needle.is_empty() || entry.keyword.to_lowercase().contains(&needle))
                .take(500)
                .map(|(index, entry)| (index, entry.keyword.clone(), entry.f1, entry.key))
                .collect();
            let keys: Vec<TopicKey> = self
                .keyword_selection
                .iter()
                .filter_map(|index| set.keywords.get(*index).map(|entry| entry.key))
                .collect();
            (set.keywords.len(), matches, keys)
        };
        ui.label(
            RichText::new(format!(
                "{keyword_count} keyword{}",
                if keyword_count == 1 { "" } else { "s" }
            ))
                .small()
                .color(theme::muted(ui.ctx())),
        );
        ui.add_space(2.0);

        if !self.keyword_selection.is_empty() {
            ui.horizontal(|ui| {
                if ui
                    .button(format!("Open {} topic(s)", selected_keys.len()))
                    .clicked()
                {
                    for key in selected_keys.iter().take(8) {
                        actions.push(Action::OpenTopic {
                            key: *key,
                            anchor: None,
                            highlight: Vec::new(),
                        });
                    }
                }
                if ui.button("Clear").clicked() {
                    actions.push(Action::ClearKeywordSelection);
                }
            });
        }

        let selection = self.keyword_selection.clone();
        let mut clicked: Option<TopicKey> = None;
        let mut toggled: Option<usize> = None;
        ScrollArea::vertical()
            .id_salt("index-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, keyword, is_f1, key) in &matches {
                    let selected = selection.contains(index);
                    ui.horizontal(|ui| {
                        let mut label = RichText::new(keyword).size(13.0);
                        if *is_f1 {
                            label = label.color(theme::ACCENT_ALT);
                        }
                        let response = ui.selectable_label(selected, label);
                        if response.clicked() {
                            clicked = Some(*key);
                        }
                        if response.secondary_clicked() {
                            toggled = Some(*index);
                        }
                    });
                }
                if matches.is_empty() {
                    ui.label(
                        RichText::new("no keyword matches")
                            .small()
                            .color(theme::muted(ui.ctx())),
                    );
                }
            });
        if let Some(key) = clicked {
            actions.push(Action::OpenTopic {
                key,
                anchor: None,
                highlight: Vec::new(),
            });
        }
        if let Some(index) = toggled {
            if let Some(position) = self.keyword_selection.iter().position(|value| *value == index) {
                self.keyword_selection.remove(position);
            } else {
                self.keyword_selection.push(index);
            }
        }
    }

    fn search_pane(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if self.index().is_none() {
            ui.label(
                RichText::new(
                    "Search needs an index. Open a package without --no-index, or press Reindex.",
                )
                .color(theme::muted(ui.ctx())),
            );
            return;
        }
        // Field filter chips.
        ui.horizontal_wrapped(|ui| {
            let all = self.search_options.fields == all_fields();
            if ui.selectable_label(all, "All").clicked() {
                self.search_options.fields = all_fields();
                let query = self.search_query();
                self.set_search_query(&query);
            }
            for (label, name) in [("Title", "title"), ("Code", "code"), ("Keywords", "kw")] {
                let mask = field_mask_for(name).unwrap_or(0);
                if mask == 0 {
                    continue;
                }
                let active = self.search_options.fields == mask;
                if ui.selectable_label(active, label).clicked() {
                    self.search_options.fields = if active { all_fields() } else { mask };
                    let query = self.search_query();
                    self.set_search_query(&query);
                }
            }
            ui.separator();
            let mut sort = self.search_options.sort;
            egui::ComboBox::from_id_salt("search-sort")
                .selected_text(match sort {
                    Sort::Relevance => "Relevance",
                    Sort::Title => "Title",
                    Sort::Path => "Path",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut sort, Sort::Relevance, "Relevance");
                    ui.selectable_value(&mut sort, Sort::Title, "Title");
                    ui.selectable_value(&mut sort, Sort::Path, "Path");
                });
            if sort != self.search_options.sort {
                self.search_options.sort = sort;
                let query = self.search_query();
                self.set_search_query(&query);
            }
        });
        ui.separator();

        let total = self.search_results.total_matched;
        let took = self.search_results.took_us;
        ui.label(
            RichText::new(if total == 0 {
                "no results".to_owned()
            } else {
                format!(
                    "{total} result{} in {}",
                    if total == 1 { "" } else { "s" },
                    if took > 1000 {
                        format!("{:.1} ms", took as f64 / 1000.0)
                    } else {
                        format!("{took} µs")
                    }
                )
            })
            .small()
            .color(theme::muted(ui.ctx())),
        );
        if !self.search_results.suggestions.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("did you mean:").small());
                for suggestion in self.search_results.suggestions.clone() {
                    if ui.small_button(&suggestion.text).clicked() {
                        actions.push(Action::SetSearchQuery(suggestion.text.clone()));
                    }
                }
            });
        }
        ui.add_space(4.0);

        let hits = self.search_results.hits.clone();
        let selected = self.search_selected;
        let mut clicked: Option<Hit> = None;
        ScrollArea::vertical()
            .id_salt("search-results")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, hit) in hits.iter().enumerate() {
                    let frame = theme::card_frame(ui.ctx(), index == selected);
                    let response = frame
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&hit.title).strong().size(13.5));
                            });
                            ui.label(
                                RichText::new(&hit.path)
                                    .small()
                                    .color(theme::muted(ui.ctx())),
                            );
                            ui.add_space(2.0);
                            self.snippet_label(ui, hit);
                            ui.label(
                                RichText::new(hit.matched_fields().join(" · "))
                                    .small()
                                    .color(theme::muted(ui.ctx())),
                            );
                        })
                        .response;
                    if response.interact(Sense::click()).clicked() {
                        clicked = Some(hit.clone());
                    }
                }
            });
        if let Some(hit) = clicked {
            self.open_hit(&hit);
        }
    }

    /// Draw a snippet with the engine's character ranges highlighted.
    fn snippet_label(&self, ui: &mut Ui, hit: &Hit) {
        if hit.snippet.is_empty() {
            return;
        }
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = ui.available_width().max(64.0);
        job.wrap.max_rows = 3;
        let chars: Vec<char> = hit.snippet.chars().collect();
        let mut cursor = 0usize;
        let mut ranges = hit.snippet_ranges.clone();
        ranges.sort_by_key(|range| range.start);
        let base = egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            color: ui.visuals().text_color(),
            ..Default::default()
        };
        for range in ranges {
            let start = range.start.min(chars.len());
            let end = range.end.min(chars.len());
            if start > cursor {
                let chunk: String = chars[cursor..start].iter().collect();
                job.append(&chunk, 0.0, base.clone());
            }
            if end > start {
                let chunk: String = chars[start..end].iter().collect();
                let mut highlighted = base.clone();
                highlighted.background = theme::highlight_colour(ui.ctx());
                highlighted.color = ui.visuals().strong_text_color();
                job.append(&chunk, 0.0, highlighted);
            }
            cursor = end.max(cursor);
        }
        if cursor < chars.len() {
            let chunk: String = chars[cursor..].iter().collect();
            job.append(&chunk, 0.0, base);
        }
        ui.add(egui::Label::new(job).selectable(false));
    }

    fn favorites_pane(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let source = self.source_id();
        let can_bookmark = self.tabs.get(self.active_tab).is_some();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    can_bookmark,
                    egui::Button::new("☆ Bookmark this topic"),
                )
                .clicked()
            {
                actions.push(Action::ToggleBookmark);
            }
        });
        ui.separator();

        // Bookmarks are grouped by folder ("(none)" first), the way Help Viewer groups them.
        let mut folders = self.state.bookmark_folders(&source);
        folders.retain(|folder| !folder.is_empty());
        let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
        groups.push((
            "(none)".to_owned(),
            self.state
                .bookmarks_for(&source)
                .into_iter()
                .filter(|bookmark| bookmark.folder.is_empty())
                .map(|bookmark| (bookmark.topic_id.clone(), bookmark.title.clone()))
                .collect(),
        ));
        for folder in folders {
            groups.push((
                folder.clone(),
                self.state
                    .bookmarks_for(&source)
                    .into_iter()
                    .filter(|bookmark| bookmark.folder == folder)
                    .map(|bookmark| (bookmark.topic_id.clone(), bookmark.title.clone()))
                    .collect(),
            ));
        }
        let count: usize = groups.iter().map(|(_, items)| items.len()).sum();
        ui.label(RichText::new(format!("Bookmarks ({count})")).strong());
        if count == 0 {
            ui.label(
                RichText::new("no bookmarks yet — press Ctrl+D to keep a topic here")
                    .small()
                    .color(theme::muted(ui.ctx())),
            );
        } else {
        ScrollArea::vertical()
            .id_salt("bookmarks")
            .max_height(220.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut remove: Option<String> = None;
                let mut open: Option<String> = None;
                for (folder, items) in &groups {
                    if items.is_empty() {
                        continue;
                    }
                    ui.label(
                        RichText::new(folder)
                            .small()
                            .color(theme::muted(ui.ctx())),
                    );
                    for (id, title) in items {
                        ui.horizontal(|ui| {
                            if ui.small_button("✖").on_hover_text("Remove").clicked() {
                                remove = Some(id.clone());
                            }
                            if ui.selectable_label(false, title).clicked() {
                                open = Some(id.clone());
                            }
                        });
                    }
                }
                if let Some(id) = remove {
                    self.state
                        .bookmarks
                        .retain(|bookmark| !(bookmark.source == source && bookmark.topic_id == id));
                    self.state.save();
                }
                if let Some(id) = open {
                    if let Some(key) = self.set().and_then(|set| set.find_by_id(&id)) {
                        actions.push(Action::OpenTopic {
                            key,
                            anchor: None,
                            highlight: Vec::new(),
                        });
                    }
                }
            });
        }

        ui.separator();
        let history: Vec<(String, String)> = self
            .state
            .history_for(&source)
            .into_iter()
            .take(50)
            .map(|entry| (entry.topic_id.clone(), entry.title.clone()))
            .collect();
        ui.label(RichText::new(format!("History ({})", history.len())).strong());
        if history.is_empty() {
            ui.label(
                RichText::new("no history yet")
                    .small()
                    .color(theme::muted(ui.ctx())),
            );
        } else {
        ScrollArea::vertical()
            .id_salt("history")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (id, title) in &history {
                    if ui.selectable_label(false, title).clicked() {
                        if let Some(key) = self.set().and_then(|set| set.find_by_id(id)) {
                            actions.push(Action::OpenTopic {
                                key,
                                anchor: None,
                                highlight: Vec::new(),
                            });
                        }
                    }
                }
            });
        }
    }

    /// Is the outline pane worth its width right now?
    ///
    /// The user's preference wins when there is room; in a narrow window the document needs
    /// the space more than the outline does, and the toolbar button says so.
    fn outline_visible(&self, ctx: &egui::Context) -> bool {
        self.state.show_outline && ctx.screen_rect().width() >= OUTLINE_MIN_WIDTH
    }

    fn outline_panel(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.add_space(6.0);
        ui.label(RichText::new("On this page").strong());
        ui.separator();
        let Some(tab) = self.tabs.get(self.active_tab) else {
            ui.label(RichText::new("—").color(theme::muted(ui.ctx())));
            return;
        };
        // A leading H1 that just repeats the topic title is noise in the outline.
        let title = tab.title.clone();
        let outline: Vec<lucid_mshc::OutlineEntry> = tab
            .document
            .outline
            .iter()
            .filter(|entry| {
                !(entry.level == 1 && entry.text.eq_ignore_ascii_case(title.trim()))
            })
            .cloned()
            .collect();
        let key = tab.key;
        ScrollArea::vertical()
            .id_salt("outline-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for entry in &outline {
                    ui.horizontal(|ui| {
                        ui.add_space((entry.level.saturating_sub(1) as f32) * 10.0);
                        let label = RichText::new(&entry.text).size(12.5);
                        if ui.selectable_label(false, label).clicked() {
                            actions.push(Action::OpenTopic {
                                key,
                                anchor: Some(entry.anchor.clone()),
                                highlight: Vec::new(),
                            });
                        }
                    });
                }
                if outline.is_empty() {
                    ui.label(
                        RichText::new("this topic has no headings")
                            .small()
                            .color(theme::muted(ui.ctx())),
                    );
                }
            });
    }

    fn central(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if self.set().is_none() {
            self.welcome_screen(ui, actions);
            return;
        }
        self.tab_bar(ui, actions);
        self.find_bar(ui);
        ui.separator();
        self.topic_view(ui, actions);
    }

    fn welcome_screen(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Lucid").size(34.0).strong());
            ui.label(
                RichText::new("A fast, modern reader for Microsoft Help Container (.mshc) packages")
                    .color(theme::muted(ui.ctx())),
            );
            ui.add_space(20.0);
            if ui.button("Open a package, folder or setup document…").clicked() {
                actions.push(Action::ShowOpenDialog);
            }
            ui.add_space(4.0);
            if !self.state.recent.is_empty() {
                ui.label(RichText::new("Recent").strong());
                for path in self.state.recent.clone().iter().take(5) {
                    if ui.link(path).clicked() {
                        actions.push(Action::Load(PathBuf::from(path)));
                    }
                }
            }
            ui.add_space(20.0);
            ui.label(
                RichText::new(
                    "Tip: drop a file anywhere in this window, or run `lucid info <path>` for a summary.",
                )
                .small()
                .color(theme::muted(ui.ctx())),
            );
            if let Some(error) = self.load_error.clone() {
                ui.add_space(10.0);
                ui.label(RichText::new(error).color(theme::ERROR));
            }
        });
    }

    fn tab_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if self.tabs.is_empty() {
            return;
        }
        let mut close: Option<usize> = None;
        let mut activate: Option<usize> = None;
        // The tabs scroll sideways; `Close all` stays pinned so it cannot be half-clipped
        // by a long tab title at a narrow window width.
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 84.0).max(96.0);
            // Shorten titles to the room a tab really has, so its close button is not the
            // widget that gets scrolled out of sight. Many tabs still scroll sideways.
            let per_tab = width / self.tabs.len().max(1) as f32;
            let title_room = if self.tabs.len() <= 3 {
                (per_tab - 48.0).max(40.0)
            } else {
                f32::INFINITY
            };
            ScrollArea::horizontal()
                .id_salt("tab-bar")
                .max_height(30.0)
                .max_width(width)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (index, tab) in self.tabs.iter().enumerate() {
                            let selected = index == self.active_tab;
                            let title = fit_label(ui, &tab.title, title_room);
                            if ui.selectable_label(selected, title).clicked() {
                                activate = Some(index);
                            }
                            if ui
                                .small_button("✖")
                                .on_hover_text("Close tab (Ctrl+W)")
                                .clicked()
                            {
                                close = Some(index);
                            }
                            ui.add_space(2.0);
                        }
                    });
                });
            if ui.small_button("Close all").clicked() {
                actions.push(Action::CloseAllTabs);
            }
        });
        if let Some(index) = activate {
            actions.push(Action::ActivateTab(index));
        }
        if let Some(index) = close {
            actions.push(Action::CloseTab(index));
        }
        // Breadcrumb — skipped for TOC roots, where it would only repeat the tab title.
        if let Some(tab) = self.tabs.get(self.active_tab) {
            if let Some(set) = self.set() {
                let crumbs = set.breadcrumb(tab.key);
                if crumbs.len() > 1 {
                    // The trailing bookmark/copy buttons are laid out right-to-left. egui
                    // starts that child at the current cursor, so the crumb run has to be
                    // limited up front; otherwise a long path draws its last crumb through
                    // the buttons (seen on a 1280px `iapi.mshc` window).
                    ui.horizontal(|ui| {
                        const TRAILING: f32 = 120.0;
                        let room = (ui.available_width() - TRAILING).max(96.0);
                        let font = egui::FontId::proportional(12.5);
                        let measure = |text: &str| {
                            ui.fonts(|fonts| {
                                fonts
                                    .layout_no_wrap(
                                        text.to_owned(),
                                        font.clone(),
                                        ui.visuals().text_color(),
                                    )
                                    .rect
                                    .width()
                            })
                        };
                        let sep = measure("›") + 8.0;
                        let mut cap = 48usize;
                        let mut first = 0usize;
                        let width = |first: usize, cap: usize| -> f32 {
                            let mut width = if first > 0 { sep + measure("…") } else { 0.0 };
                            for (index, (title, _)) in crumbs.iter().enumerate().skip(first) {
                                if index > first {
                                    width += sep;
                                }
                                width += measure(&shorten(title, cap));
                            }
                            width
                        };
                        // Keep at least the last two crumbs; drop the ancestors first, then
                        // shorten what is left.
                        while width(first, cap) > room && first + 2 < crumbs.len() {
                            first += 1;
                        }
                        while width(first, cap) > room && cap > 12 {
                            cap = (cap / 2).max(12);
                        }
                        if first > 0 {
                            let muted = RichText::new("…").size(12.5).color(theme::muted(ui.ctx()));
                            ui.label(muted);
                            ui.label(RichText::new("›").color(theme::muted(ui.ctx())));
                        }
                        for (index, (title, key)) in crumbs.iter().enumerate().skip(first) {
                            if index > first {
                                ui.label(RichText::new("›").color(theme::muted(ui.ctx())));
                            }
                            let is_last = index + 1 == crumbs.len();
                            let label = RichText::new(shorten(title, cap)).size(12.5);
                            let label = if is_last {
                                label.strong()
                            } else {
                                label.color(theme::muted(ui.ctx()))
                            };
                            if !is_last {
                                if let Some(key) = key {
                                    if ui.link(label).clicked() {
                                        actions.push(Action::OpenTopic {
                                            key: *key,
                                            anchor: None,
                                            highlight: Vec::new(),
                                        });
                                    }
                                }
                            } else {
                                ui.label(label);
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .small_button("Copy link")
                                .on_hover_text("Copy ms-xhelp link to the clipboard")
                                .clicked()
                            {
                                let id = set.topic_id(tab.key);
                                actions.push(Action::Copy(format!("ms-xhelp:///?Id={id}")));
                            }
                            let bookmarked = self
                                .state
                                .is_bookmarked(&self.source_id(), set.topic_id(tab.key));
                            let star = if bookmarked { "★" } else { "☆" };
                            if ui
                                .small_button(star)
                                .on_hover_text("Bookmark (Ctrl+D)")
                                .clicked()
                            {
                                actions.push(Action::ToggleBookmark);
                            }
                        });
                    });
                }
            }
        }
    }

    fn find_bar(&mut self, ui: &mut Ui) {
        let Some(find_state) = self.find.as_ref() else {
            return;
        };
        let mut needle = find_state.needle.clone();
        let mut case_sensitive = find_state.case_sensitive;
        let count = self.find_matches.len();
        let current = find_state.current;
        let mut changed = false;
        let mut step_forward = false;
        let mut step_back = false;
        let mut close = false;

        ui.horizontal(|ui| {
            ui.label("Find in topic:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut needle)
                    .hint_text("text to find")
                    .desired_width(220.0),
            );
            if response.changed() {
                changed = true;
            }
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                step_forward = true;
            }
            if ui.selectable_label(case_sensitive, "Aa").clicked() {
                case_sensitive = !case_sensitive;
                changed = true;
            }
            ui.label(
                RichText::new(if count == 0 {
                    "no matches".to_owned()
                } else {
                    format!("{}/{}", current + 1, count)
                })
                .small()
                .color(theme::muted(ui.ctx())),
            );
            if ui
                .small_button("⏶")
                .on_hover_text("Previous match (Shift+Enter)")
                .clicked()
            {
                step_back = true;
            }
            if ui
                .small_button("⏷")
                .on_hover_text("Next match (Enter)")
                .clicked()
            {
                step_forward = true;
            }
            if ui.small_button("✖").clicked() {
                close = true;
            }
        });

        if let Some(find) = self.find.as_mut() {
            find.needle = needle;
            find.case_sensitive = case_sensitive;
            find.current = find.current.min(count.saturating_sub(1));
        }
        if changed {
            self.recompute_find();
        }
        if step_forward {
            self.find_step(true);
        }
        if step_back {
            self.find_step(false);
        }
        if close {
            self.find = None;
            self.find_matches.clear();
        }
    }

    fn topic_view(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if self.tabs.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(
                    RichText::new("Choose a topic in the Contents pane")
                        .color(theme::muted(ui.ctx())),
                );
            });
            return;
        }
        let active = self.active_tab;

        let mut render_actions: Vec<RenderAction> = Vec::new();
        {
            let Loaded { set, .. } = match self.loaded.as_ref() {
                Some(loaded) => loaded,
                None => return,
            };
            let tab = &mut self.tabs[active];
            let find_pattern = self.find.as_ref().and_then(|find| {
                (!find.needle.trim().is_empty()).then(|| FindPattern {
                    needle: find.needle.clone(),
                    case_sensitive: find.case_sensitive,
                })
            });
            let code_language = self.state.code_language.clone();
            let frame = theme::document_frame(ui.ctx());
            let assets = &mut self.assets;
            // Reading measure: a centred column instead of lines running the full width of a
            // 2560 px display. `top_down(Center)` + an explicit width is what makes `Frame`
            // produce a card of that width (a horizontal layout let the frame shrink to its
            // content and the text overflow it).
            ScrollArea::vertical()
                .id_salt(format!("topic-scroll:{}", tab.key))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let available = ui.available_width();
                    let column = available.min(MAX_DOCUMENT_WIDTH);
                    let inner = (column - 48.0).max(200.0);
                    ui.with_layout(Layout::top_down(egui::Align::Center), |ui| {
                    ui.set_max_width(column);
                    frame.show(ui, |ui| {
                        ui.set_min_width(inner);
                        ui.set_max_width(inner);
                        // The centring above applies to the *card*; inside it everything is
                        // left-aligned like a normal document.
                        ui.with_layout(Layout::top_down(egui::Align::Min), |ui| {
                            let document = tab.document.clone();
                            let mut renderer = Renderer::new(
                                set,
                                tab.key,
                                assets,
                                &mut tab.view,
                                find_pattern,
                                code_language.clone(),
                                &mut render_actions,
                            );
                            renderer.draw(ui, &document);
                        });
                    });
                    });
                    // Resolve pending scroll requests against the freshly recorded positions.
                    let target: Option<f32> = if let Some(anchor) = tab.view.scroll_to_anchor.take()
                    {
                        tab.view.anchors.get(&anchor).copied()
                    } else if let Some(offset) = tab.view.scroll_to_offset.take() {
                        tab.view
                            .blocks
                            .iter()
                            .filter(|block| block.text_offset <= offset)
                            .max_by_key(|block| block.text_offset)
                            .map(|block| block.top)
                    } else {
                        None
                    };
                    if let Some(top) = target {
                        let width = ui.max_rect().width();
                        ui.scroll_to_rect(
                            egui::Rect::from_min_size(
                                egui::pos2(ui.max_rect().left(), top),
                                Vec2::new(width, 1.0),
                            ),
                            Some(Align::TOP),
                        );
                    }
                });
        }

        // Handle what the renderer reported.
        for action in render_actions {
            match action {
                RenderAction::Link(href) => {
                    let Some(set) = self.set() else { continue };
                    let from = match self.tabs.get(active) {
                        Some(tab) => tab.key,
                        None => continue,
                    };
                    match set.resolve(from, &href) {
                        LinkTarget::Missing(_) => {
                            self.toast = Some((format!("link target not found: {href}"), now()))
                        }
                        target => self.navigate(target),
                    }
                }
                RenderAction::Copy(text) => actions.push(Action::Copy(text)),
            }
        }
    }

    fn quick_open_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.quick.is_none() {
            return;
        }
        let mut close = false;
        let mut accepted: Option<TopicKey> = None;
        let entries = self.quick_entries.clone();
        egui::Window::new("Go to topic")
            .id(egui::Id::new("quick-open"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .default_width(560.0)
            .show(ctx, |ui| {
                let quick = self.quick.as_mut().expect("checked above");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut quick.query)
                        .hint_text("type a topic title")
                        .desired_width(f32::INFINITY),
                );
                if quick.focus {
                    response.request_focus();
                    quick.focus = false;
                }
                if response.changed() {
                    quick.matches = quickopen::filter(&entries, &quick.query, 40);
                    quick.selected = 0;
                }
                let moved_down = ui.input(|input| input.key_pressed(egui::Key::ArrowDown));
                let moved_up = ui.input(|input| input.key_pressed(egui::Key::ArrowUp));
                if moved_down && !quick.matches.is_empty() {
                    quick.selected = (quick.selected + 1) % quick.matches.len();
                }
                if moved_up && !quick.matches.is_empty() {
                    quick.selected = (quick.selected + quick.matches.len() - 1) % quick.matches.len();
                }
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    accepted = quick.matches.get(quick.selected).map(|entry| entry.key);
                }
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
                ui.separator();
                ScrollArea::vertical()
                    .id_salt("quick-open")
                    .max_height(320.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let selected = quick.selected;
                        for (index, entry) in quick.matches.iter().enumerate() {
                            let frame = theme::card_frame(ui.ctx(), index == selected);
                            let response = frame
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(&entry.title).strong());
                                    ui.label(
                                        RichText::new(&entry.path)
                                            .small()
                                            .color(theme::muted(ui.ctx())),
                                    );
                                })
                                .response;
                            if response.interact(Sense::click()).clicked()
                                || (index == selected
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                            {
                                accepted = Some(entry.key);
                            }
                        }
                        if quick.matches.is_empty() {
                            ui.label(
                                RichText::new("nothing matches")
                                    .color(theme::muted(ui.ctx())),
                            );
                        }
                    });
                let _ = actions;
            });
        if let Some(key) = accepted {
            actions.push(Action::OpenTopic {
                key,
                anchor: None,
                highlight: Vec::new(),
            });
            close = true;
        }
        if close {
            self.quick = None;
        }
    }

    fn dialog_window(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let mut close = false;
        let mut navigate: Option<DialogNav> = None;
        let mut open: Option<PathBuf> = None;
        egui::Window::new("Open help package")
            .id(egui::Id::new("open-dialog"))
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 420.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("⬆ Up").clicked() {
                        navigate = Some(DialogNav::Up);
                    }
                    ui.label(RichText::new(dialog.dir.display().to_string()).small());
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let mut typed = dialog.typed.clone();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut typed)
                            .hint_text("type a path and press Enter")
                            .desired_width(ui.available_width() - 60.0),
                    );
                    if response.changed() {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.typed = typed.clone();
                        }
                    }
                    let submit = (response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                        || ui.button("Go").clicked();
                    if submit {
                        navigate = Some(DialogNav::To(PathBuf::from(&typed)));
                    }
                });
                ui.separator();
                ScrollArea::vertical()
                    .id_salt("file-dialog")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (name, is_dir) in &dialog.entries {
                            let label = if *is_dir {
                                format!("📁 {name}")
                            } else if name.to_lowercase().ends_with(".mshc") {
                                format!("📦 {name}")
                            } else {
                                format!("📄 {name}")
                            };
                            let selected = dialog.selected.as_ref().map(|path| {
                                path.file_name().map(|file| file.to_string_lossy().into_owned())
                                    == Some(name.clone())
                            });
                            if ui
                                .selectable_label(selected.unwrap_or(false), label)
                                .clicked()
                            {
                                if *is_dir {
                                    navigate = Some(DialogNav::Into(name.clone()));
                                } else {
                                    open = Some(dialog.dir.join(name));
                                }
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        if let Some(selected) = dialog.selected.clone() {
                            open = Some(selected);
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.label(
                        RichText::new("Tip: folders are content stores — open the folder itself.")
                            .small()
                            .color(theme::muted(ui.ctx())),
                    );
                });
            });
        if let Some(nav) = navigate {
            if let Some(dialog) = self.dialog.as_mut() {
                match nav {
                    DialogNav::Up => dialog.up(),
                    DialogNav::Into(name) => dialog.enter(&name),
                    DialogNav::To(path) => dialog.go_to(&path),
                }
            }
        }
        if let Some(path) = open {
            if let Some(dialog) = self.dialog.as_mut() {
                dialog.selected = Some(path.clone());
            }
            actions.push(Action::OpenDialogFile(path));
        }
        if close {
            actions.push(Action::CloseDialog);
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let Some((message, at)) = self.toast.clone() else {
            return;
        };
        if now() - at > 6.0 {
            self.toast = None;
            return;
        }
        egui::Area::new(egui::Id::new("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -48.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::mix(ui.visuals().window_fill, theme::ACCENT, 0.15))
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .corner_radius(egui::CornerRadius::same(8))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&message));
                    });
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let mut close = false;
        egui::Window::new("About Lucid")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("Lucid {}", env!("CARGO_PKG_VERSION"))).strong());
                ui.label("A fast, modern viewer for Microsoft Help Container (.mshc) packages.");
                ui.separator();
                ui.label("Contents, Index, Search and Favorites panes; ranked search with phrases, prefixes, fuzzy matching and field filters; native topic rendering — no browser engine, no .NET.");
                ui.separator();
                let cache = crate::index_cache::cache_size();
                ui.label(RichText::new(format!(
                    "Search-index cache: {} ({})",
                    human_bytes(cache),
                    crate::index_cache::cache_dir()
                        .map(|dir| dir.display().to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ))
                .small());
                ui.separator();
                ui.label("Keyboard: Ctrl+O open · Ctrl+P go to topic · Ctrl+K search · Ctrl+F find in topic · Ctrl+W close tab · Ctrl+D bookmark · Alt+Left/Right history · Ctrl+±  zoom · F5 reindex");
                ui.separator();
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if close {
            self.show_about = false;
        }
    }

    // ------------------------------------------------------------------ actions

    fn apply(&mut self, actions: Vec<Action>, ctx: &egui::Context) {
        for action in actions {
            match action {
                Action::Load(path) => self.start_load(&path, ctx),
                Action::OpenTopic {
                    key,
                    anchor,
                    highlight,
                } => self.open_topic(key, anchor, highlight),
                Action::CloseTab(index) => {
                    if index < self.tabs.len() {
                        self.tabs.remove(index);
                        if self.active_tab >= self.tabs.len() && !self.tabs.is_empty() {
                            self.active_tab = self.tabs.len() - 1;
                        }
                        self.recompute_find();
                    }
                }
                Action::ActivateTab(index) => {
                    if index < self.tabs.len() {
                        self.active_tab = index;
                        self.recompute_find();
                    }
                }
                Action::CloseAllTabs => {
                    self.tabs.clear();
                    self.active_tab = 0;
                    self.find_matches.clear();
                }
                Action::Reindex => self.reindex(),
                Action::SetTheme(theme_choice) => {
                    self.state.theme = theme_choice;
                    self.state.save();
                }
                Action::ToggleBookmark => {
                    if let (Some(set), Some(tab)) = (self.set(), self.tabs.get(self.active_tab)) {
                        let id = set.topic_id(tab.key).to_owned();
                        if id.is_empty() {
                            self.toast = Some((
                                "this topic has no Microsoft.Help.Id".to_owned(),
                                now(),
                            ));
                        } else {
                            let title = tab.title.clone();
                            let added = self
                                .state
                                .toggle_bookmark(&self.source_id(), &id, &title);
                            self.state.save();
                            self.toast = Some((
                                if added { "bookmarked" } else { "bookmark removed" }.to_owned(),
                                now(),
                            ));
                        }
                    }
                }
                Action::Copy(text) => copy_to_clipboard(ctx, &text),
                Action::Message(message) => {
                    if message == "quick-open" {
                        self.quick = Some(QuickOpen {
                            query: String::new(),
                            matches: quickopen::filter(&self.quick_entries, "", 40),
                            selected: 0,
                            focus: true,
                        });
                    } else {
                        self.toast = Some((message, now()));
                    }
                }
                Action::SetZoom(scale) => {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                        tab.view.scale = scale.clamp(0.8, 1.8);
                    }
                    self.state.font_scale = scale.clamp(0.8, 1.8);
                    self.state.save();
                }
                Action::SetSidebarTab(tab) => {
                    self.sidebar_tab = tab;
                    self.state.sidebar_tab = tab.id().to_owned();
                    if tab == SidebarTab::Search {
                        self.focus_search = true;
                    }
                }
                Action::SetSearchQuery(query) => {
                    self.search_query_cache = query.clone();
                    self.set_search_query(&query);
                    self.sidebar_tab = SidebarTab::Search;
                    self.state.sidebar_tab = SidebarTab::Search.id().to_owned();
                }
                Action::ShowRecent => {
                    self.dialog = Some(OpenDialog::new(&std::env::current_dir().unwrap_or_default()));
                }
                Action::ShowOpenDialog => {
                    let start = self
                        .set()
                        .map(|set| {
                            set.path
                                .parent()
                                .map(|parent| parent.to_path_buf())
                                .unwrap_or_else(|| set.path.clone())
                        })
                        .or_else(|| std::env::current_dir().ok())
                        .unwrap_or_default();
                    self.dialog = Some(OpenDialog::new(&start));
                }
                Action::CloseDialog => self.dialog = None,
                Action::OpenDialogFile(path) => {
                    self.dialog = None;
                    self.start_load(&path, ctx);
                }
                Action::ClearKeywordSelection => self.keyword_selection.clear(),
            }
        }
    }

    fn reindex(&mut self) {
        let Some(set) = self.set() else {
            return;
        };
        crate::index_cache::clear(set);
        let path = set.path.clone();
        self.status = "rebuilding index…".to_owned();
        self.loader = Some(load::spawn(path, false));
    }

    // ------------------------------------------------------------------ input

    fn handle_shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let (open, quick, search, find, close_tab, bookmark, back, forward, reindex, zoom_in, zoom_out, zoom_reset, escape) =
            ctx.input(|input| {
                let command = input.modifiers.command;
                (
                    command && input.key_pressed(egui::Key::O),
                    command && input.key_pressed(egui::Key::P),
                    command && input.key_pressed(egui::Key::K),
                    command && input.key_pressed(egui::Key::F),
                    command && input.key_pressed(egui::Key::W),
                    command && input.key_pressed(egui::Key::D),
                    input.modifiers.alt && input.key_pressed(egui::Key::ArrowLeft),
                    input.modifiers.alt && input.key_pressed(egui::Key::ArrowRight),
                    input.key_pressed(egui::Key::F5),
                    command && input.key_pressed(egui::Key::Plus),
                    command && input.key_pressed(egui::Key::Minus),
                    command && input.key_pressed(egui::Key::Num0),
                    input.key_pressed(egui::Key::Escape),
                )
            });

        if open {
            actions.push(Action::ShowOpenDialog);
        }
        if quick {
            actions.push(Action::Message("quick-open".to_owned()));
        }
        if search {
            self.focus_search = true;
            actions.push(Action::SetSidebarTab(SidebarTab::Search));
        }
        if find {
            self.find = Some(self.find.take().unwrap_or_default());
        }
        if close_tab {
            actions.push(Action::CloseTab(self.active_tab));
        }
        if bookmark {
            actions.push(Action::ToggleBookmark);
        }
        if back {
            if self.history_pos > 1 {
                self.history_pos -= 1;
                if let Some((key, _)) = self.session_history.get(self.history_pos - 1).cloned() {
                    self.open_topic(key, None, Vec::new());
                }
            }
        }
        if forward && self.history_pos < self.session_history.len() {
            self.history_pos += 1;
            if let Some((key, _)) = self.session_history.get(self.history_pos - 1).cloned() {
                self.open_topic(key, None, Vec::new());
            }
        }
        if reindex {
            actions.push(Action::Reindex);
        }
        if zoom_in {
            actions.push(Action::SetZoom(self.current_scale() + 0.1));
        }
        if zoom_out {
            actions.push(Action::SetZoom(self.current_scale() - 0.1));
        }
        if zoom_reset {
            actions.push(Action::SetZoom(1.0));
        }
        if escape {
            if self.quick.is_some() {
                self.quick = None;
            } else if self.dialog.is_some() {
                actions.push(Action::CloseDialog);
            } else if self.find.is_some() {
                self.find = None;
                self.find_matches.clear();
            }
        }
    }

    /// Screenshot mode: wait for the content set, capture the window, write a PNG, exit.
    ///
    /// Used by the layout checks in `docs/DESIGN.md` §6 (and by anyone who wants a picture of
    /// a package without clicking through it).
    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let captured = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = captured {
            if let Some(plan) = self.screenshot.take() {
                match save_png(&plan.path, &image) {
                    Ok(()) => eprintln!(
                        "lucid: wrote {} ({}×{})",
                        plan.path.display(),
                        image.size[0],
                        image.size[1]
                    ),
                    Err(error) => eprintln!(
                        "lucid: could not write {}: {error}",
                        plan.path.display()
                    ),
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let Some(plan) = self.screenshot.as_mut() else {
            return;
        };
        if plan.requested {
            return;
        }
        plan.frames_drawn += 1;
        // Give the panels a couple of frames to settle after the content set arrives.
        let ready = (self.loaded.is_some() || self.load_error.is_some())
            && plan.frames_drawn >= plan.frames_to_wait;
        if ready {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            plan.requested = true;
        }
        ctx.request_repaint();
    }

    fn handle_dropped(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        for path in dropped {
            actions.push(Action::Load(path));
        }
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
        if hovering {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop-hint"),
            ));
            let rect = ctx.screen_rect();
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(120));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop a .mshc package, folder or .msha setup document",
                egui::FontId::proportional(20.0),
                Color32::WHITE,
            );
        }
    }
}

impl eframe::App for LucidApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Theme (re-applied when it changes, or when the font scale does).
        let theme_key = (self.state.theme, (self.state.font_scale * 100.0) as i32);
        if self.theme_applied != Some(theme_key) {
            theme::apply(ctx, self.state.theme, self.state.font_scale);
            self.theme_applied = Some(theme_key);
        }

        self.poll_loader(ctx);
        let mut actions: Vec<Action> = Vec::new();
        self.handle_shortcuts(ctx, &mut actions);
        self.handle_dropped(ctx, &mut actions);

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            self.toolbar(ui, &mut actions);
        });
        egui::TopBottomPanel::bottom("status")
            .exact_height(26.0)
            .show(ctx, |ui| {
                self.status_bar(ui);
            });
        // A persisted 520 px sidebar must not starve the document when the window is
        // 900 px wide; the range is capped by the window, not only by the hard maximum.
        let sidebar_max = (ctx.screen_rect().width() * 0.40).clamp(220.0, 520.0);
        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(self.state.sidebar_width.min(sidebar_max))
            .width_range(220.0..=sidebar_max)
            .show(ctx, |ui| {
                self.sidebar(ui, &mut actions);
            });
        if self.outline_visible(ctx) {
            egui::SidePanel::right("outline")
                .resizable(true)
                .default_width(self.state.outline_width)
                .width_range(150.0..=360.0)
                .show(ctx, |ui| {
                    self.outline_panel(ui, &mut actions);
                });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.central(ui, &mut actions);
        });

        self.quick_open_window(ctx, &mut actions);
        self.dialog_window(ctx, &mut actions);
        self.about_window(ctx);
        self.toast(ctx);
        self.apply(actions, ctx);
        self.handle_screenshot(ctx);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.state.save();
    }
}

// ---------------------------------------------------------------------- helpers

/// Human-readable byte size for the About window.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

fn first_key(node: &TocNode<TopicKey>) -> Option<TopicKey> {
    if let Some(key) = node.key {
        return Some(key);
    }
    node.children.iter().find_map(first_key)
}

fn all_titles(nodes: &[TocNode<TopicKey>]) -> Vec<String> {
    let mut out = Vec::new();
    for node in nodes {
        out.push(node.title.clone());
        out.extend(all_titles(&node.children));
    }
    out
}

/// Does this subtree contain a title matching the filter?
fn subtree_matches(node: &TocNode<TopicKey>, filter: &str) -> bool {
    if filter_matches(&node.title, filter) {
        return true;
    }
    node.children
        .iter()
        .any(|child| subtree_matches(child, filter))
}

fn filter_matches(title: &str, filter: &str) -> bool {
    filter.is_empty() || title.to_lowercase().contains(filter)
}

/// Shorten `text` until it is no wider than `room` (font size 14.5, widget text).
fn fit_label(ui: &Ui, text: &str, room: f32) -> String {
    if !room.is_finite() {
        return shorten(text, 28);
    }
    let font = egui::FontId::proportional(14.5);
    let measure = |text: &str| {
        ui.fonts(|fonts| {
            fonts
                .layout_no_wrap(text.to_owned(), font.clone(), ui.visuals().text_color())
                .rect
                .width()
        })
    };
    if measure(text) <= room {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}…", chars.iter().collect::<String>());
        if measure(&candidate) <= room {
            return candidate;
        }
    }
    "…".to_owned()
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn all_fields() -> u8 {
    (1 << lucid_search::NUM_FIELDS) - 1
}

/// Character ranges of `needle` inside `text`.
fn find_in_text(text: &str, needle: &str, case_sensitive: bool) -> Vec<Range<usize>> {
    let hay: Vec<char> = text.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let mut out = Vec::new();
    if pat.is_empty() || pat.len() > hay.len() {
        return out;
    }
    let mut index = 0usize;
    while index + pat.len() <= hay.len() {
        let matches = (0..pat.len()).all(|offset| {
            let a = hay[index + offset];
            let b = pat[offset];
            if case_sensitive {
                a == b
            } else {
                a.to_lowercase().eq(b.to_lowercase())
            }
        });
        if matches {
            out.push(index..index + pat.len());
            index += 1;
        } else {
            index += 1;
        }
    }
    out
}

fn copy_to_clipboard(ctx: &egui::Context, text: &str) {
    ctx.copy_text(text.to_owned());
}

/// Open a URL with the platform's handler (no dependency on a browser crate).
fn open_external(url: &str) -> std::io::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Entry point used by `cli::cmd_gui`.
pub fn run(
    path: Option<PathBuf>,
    topic: Option<String>,
    search: Option<String>,
    f1: Option<String>,
    no_index: bool,
    view: ViewOptions,
    startup: Startup,
) -> Result<ExitCode, String> {
    let state = AppState::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Lucid")
        .with_min_inner_size([900.0, 600.0])
        .with_maximized(view.maximized);
    viewport = match view.size {
        Some([width, height]) => viewport.with_inner_size([width, height]),
        // A size to fall back to when the window manager ignores the maximized request.
        None => viewport.with_inner_size([1440.0, 900.0]),
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = eframe::run_native(
        "Lucid",
        options,
        Box::new(move |cc| {
            Ok(Box::new(LucidApp::new(
                cc, state, no_index, path, topic, f1, search, view, startup,
            )))
        }),
    );
    result.map(|_| ExitCode::SUCCESS).map_err(|error| error.to_string())
}

/// Write an egui screenshot as a PNG.
fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let width = image.size[0] as u32;
    let height = image.size[1] as u32;
    let mut bytes = Vec::with_capacity((width * height * 4) as usize);
    for pixel in &image.pixels {
        bytes.extend_from_slice(&pixel.to_array());
    }
    let buffer = image::RgbaImage::from_raw(width, height, bytes).ok_or_else(|| {
        std::io::Error::other("screenshot buffer does not match its dimensions")
    })?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    buffer
        .save(path)
        .map_err(|error| std::io::Error::other(error.to_string()))
}
