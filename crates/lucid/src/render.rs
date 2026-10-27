//! Rendering a topic: [`lucid_mshc::Document`] (the parsed block model) → egui widgets.
//!
//! Design notes
//! ------------
//! * Paragraphs are drawn from one `LayoutJob`, so text wraps as a paragraph and can be
//!   selected as a whole, while links inside it stay clickable: a click is mapped back to a
//!   character index with `Galley::cursor_from_pos`.
//! * Images are decoded from package assets on first use and cached as textures.
//! * Highlighting (from the search pane) and find-in-topic both split the text runs of a
//!   paragraph into highlighted and normal sections, using the character ranges the search
//!   engine produced.
//! * Microsoft Help behaviours map onto native widgets: `CollapsibleArea` becomes a
//!   collapsing header, `CodeSnippet` a code block with copy/language tabs,
//!   `LanguageSpecificText` follows the reader's preferred code language.
//! * Nothing here knows about HTML — the parsing happened in `lucid-mshc`.

use std::collections::HashSet;
use std::ops::Range;

use egui::text::{LayoutJob, LayoutSection, TextFormat};
use egui::{Color32, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2};

use lucid_mshc::{Block, CodeBlock, DocSet, Document, Inline, Table, TopicKey};

use crate::state::CodeLanguage;
use crate::theme;

/// Something the renderer wants the application to do.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderAction {
    /// A link was clicked; the application resolves the raw `href` in the content set.
    Link(String),
    /// Copy text to the clipboard.
    Copy(String),
}

/// Where a block was drawn, so the view can scroll to it.
#[derive(Debug, Clone, Copy)]
pub struct BlockAnchor {
    /// Character offset of the block inside `Document::text`.
    pub text_offset: usize,
    /// Top edge of the block in screen space.
    pub top: f32,
}

/// Find-in-topic pattern, already lowercased when matching case-insensitively.
#[derive(Debug, Clone)]
pub struct FindPattern {
    pub needle: String,
    pub case_sensitive: bool,
}

/// Per-topic view state, kept across frames while a tab is open.
#[derive(Debug, Clone, Default)]
pub struct ViewState {
    /// Anchor name → y position, recorded while drawing (for `#anchor` links).
    pub anchors: std::collections::HashMap<String, f32>,
    /// Block positions, for scrolling to an anchor or a find-in-topic match.
    pub blocks: Vec<BlockAnchor>,
    /// Selected language per snippet group.
    pub snippet_choice: std::collections::HashMap<String, usize>,
    /// Zoom factor for this tab.
    pub scale: f32,
    /// Terms from the search pane that produced this tab (highlighted in the text).
    pub highlight_terms: Vec<String>,
    /// Set by the application: scroll to this anchor on the next frame.
    pub scroll_to_anchor: Option<String>,
    /// Set by the application: pending scroll offset (find-in-topic).
    pub scroll_to_offset: Option<usize>,
}

impl ViewState {
    pub fn new() -> Self {
        ViewState {
            scale: 1.0,
            ..Default::default()
        }
    }
}

/// Find-in-topic state (Ctrl+F).
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub needle: String,
    pub case_sensitive: bool,
    /// Index of the current match (into the list of matches of the open tab).
    pub current: usize,
}

/// Decoded image cache (shared by all tabs of a content set).
#[derive(Default)]
pub struct AssetCache {
    textures: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    failed: HashSet<String>,
}

impl std::fmt::Debug for AssetCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetCache")
            .field("textures", &self.textures.len())
            .finish()
    }
}

impl AssetCache {
    /// Drop cached textures (used when the content set changes).
    pub fn clear(&mut self) {
        self.textures.clear();
        self.failed.clear();
    }

    fn texture(
        &mut self,
        ctx: &egui::Context,
        set: &DocSet,
        key: TopicKey,
        src: &str,
    ) -> Option<egui::TextureHandle> {
        if self.failed.contains(src) {
            return None;
        }
        if let Some(cached) = self.textures.get(src) {
            return cached.clone();
        }
        let texture = match set.asset(key, src) {
            Ok(Some(asset)) => match image::load_from_memory(&asset.bytes) {
                Ok(image) => {
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let pixels = rgba.into_raw();
                    let colour_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                    Some(ctx.load_texture(
                        format!("asset:{}", asset.norm),
                        colour_image,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                Err(_) => {
                    self.failed.insert(src.to_owned());
                    None
                }
            },
            _ => {
                self.failed.insert(src.to_owned());
                None
            }
        };
        self.textures.insert(src.to_owned(), texture.clone());
        texture
    }
}

/// A set of `CodeSnippet`s sharing a group id (rendered as language tabs).
#[derive(Debug, Clone)]
struct SnippetGroup {
    id: String,
    codes: Vec<CodeBlock>,
}

/// Base text kinds.
#[derive(Debug, Clone, Copy)]
enum TextKind {
    Body,
    Heading { size: f32 },
    Small,
}

impl TextKind {
    fn format(self, ui: &Ui, scale: f32) -> TextFormat {
        match self {
            TextKind::Body => TextFormat {
                font_id: FontId::proportional(14.5 * scale),
                color: ui.visuals().text_color(),
                ..Default::default()
            },
            TextKind::Heading { size } => TextFormat {
                font_id: FontId::proportional(size * scale),
                color: ui.visuals().strong_text_color(),
                ..Default::default()
            },
            TextKind::Small => TextFormat {
                font_id: FontId::proportional(12.0 * scale),
                color: theme::muted(ui.ctx()),
                ..Default::default()
            },
        }
    }
}

/// The renderer for one topic.
pub struct Renderer<'a> {
    set: &'a DocSet,
    key: TopicKey,
    assets: &'a mut AssetCache,
    view: &'a mut ViewState,
    find: Option<FindPattern>,
    action_sink: &'a mut Vec<RenderAction>,
    code_language: CodeLanguage,
    scale: f32,
    highlight: Vec<String>,
    text_offset: usize,
    snippet_groups: Vec<SnippetGroup>,
    drawn_groups: HashSet<String>,
    anchors: std::collections::HashMap<String, f32>,
    blocks: Vec<BlockAnchor>,
    /// Monotonic table counter: `Grid` ids must be unique within a topic, and two member
    /// tables often have the same row count.
    table_index: usize,
    /// Table cells whose longest token cannot fit the column. Only those are allowed to
    /// break mid-token; ordinary prose keeps wrapping at word boundaries.
    break_anywhere_depth: usize,
    /// Nesting depth of the table cell being drawn (0 outside tables).
    cell_depth: usize,
    /// Width a table cell's text must wrap at. Inside `Grid` the measured
    /// `available_width()` can be zero (the sizing pass, and every block after the first
    /// in a cell), so the cell width is carried explicitly instead.
    cell_wrap: Option<f32>,
}

impl<'a> Renderer<'a> {
    pub fn new(
        set: &'a DocSet,
        key: TopicKey,
        assets: &'a mut AssetCache,
        view: &'a mut ViewState,
        find: Option<FindPattern>,
        code_language: CodeLanguage,
        action_sink: &'a mut Vec<RenderAction>,
    ) -> Self {
        let scale = if view.scale > 0.0 { view.scale } else { 1.0 };
        let highlight = view.highlight_terms.clone();
        Renderer {
            set,
            key,
            assets,
            view,
            find,
            action_sink,
            code_language,
            scale,
            highlight,
            text_offset: 0,
            snippet_groups: Vec::new(),
            drawn_groups: HashSet::new(),
            anchors: std::collections::HashMap::new(),
            blocks: Vec::new(),
            table_index: 0,
            break_anywhere_depth: 0,
            cell_depth: 0,
            cell_wrap: None,
        }
    }

    /// Draw a whole document and persist the recorded positions into the view state.
    pub fn draw(&mut self, ui: &mut Ui, document: &Document) {
        self.snippet_groups = collect_snippet_groups(&document.blocks);
        let blocks = document.blocks.clone();
        for block in &blocks {
            self.block(ui, block, 0);
        }
        self.view.anchors = std::mem::take(&mut self.anchors);
        self.view.blocks = std::mem::take(&mut self.blocks);
    }

    // ------------------------------------------------------------------ blocks

    fn block(&mut self, ui: &mut Ui, block: &Block, depth: usize) {
        let start_offset = self.text_offset;
        let top = ui.cursor().top();
        match block {
            Block::Heading {
                level,
                inlines,
                anchor,
            } => {
                if let Some(anchor) = anchor {
                    self.anchors.insert(anchor.clone(), top);
                }
                ui.add_space(if *level <= 1 { 4.0 } else { 14.0 });
                let size = match level {
                    1 => 22.0,
                    2 => 18.0,
                    3 => 16.0,
                    _ => 14.5,
                };
                self.label(ui, inlines, TextKind::Heading { size });
                if *level <= 2 {
                    ui.add_space(2.0);
                    ui.separator();
                }
                ui.add_space(4.0);
            }
            Block::Paragraph { inlines } => {
                self.label(ui, inlines, TextKind::Body);
                ui.add_space(4.0);
            }
            Block::List { ordered, items } => {
                for (index, item) in items.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        let marker = if *ordered {
                            format!("{}.", index + 1)
                        } else {
                            "•".to_owned()
                        };
                        ui.label(RichText::new(marker).color(theme::muted(ui.ctx())));
                        ui.vertical(|ui| {
                            for block in item {
                                self.block(ui, block, depth + 1);
                            }
                        });
                    });
                }
                ui.add_space(4.0);
            }
            Block::Table(table) => {
                self.table(ui, table);
                ui.add_space(8.0);
            }
            Block::Code(code) => {
                self.code(ui, code);
                ui.add_space(8.0);
            }
            Block::Note { label, blocks } => {
                let colour = theme::WARN;
                theme::note_frame(ui.ctx(), colour).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        // A pin rather than "ⓘ": U+24D8 is not in the bundled fonts and was
                        // rendering as a box in every callout (see docs/DESIGN.md §6).
                        ui.label(RichText::new("📌").color(colour).strong());
                        ui.vertical(|ui| {
                            if let Some(label) = label {
                                ui.label(RichText::new(label).strong().color(colour));
                            }
                            for block in blocks {
                                self.block(ui, block, depth + 1);
                            }
                        });
                    });
                });
                ui.add_space(8.0);
            }
            Block::Collapsible {
                title,
                expanded,
                blocks,
            } => {
                // Section titles are outline entries too; the model registered the matching
                // anchor, so clicking the outline must scroll here.
                self.anchors
                    .insert(lucid_mshc::model::slugify(title), top);
                egui::CollapsingHeader::new(RichText::new(title).strong())
                    .id_salt(format!("collapsible:{}:{}", self.key, title))
                    .default_open(*expanded)
                    .show(ui, |ui| {
                        for block in blocks {
                            self.block(ui, block, depth + 1);
                        }
                    });
                ui.add_space(4.0);
            }
            Block::Quote { blocks } => {
                let left = ui.cursor().left();
                let top = ui.cursor().top();
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 0,
                        top: 2,
                        bottom: 2,
                    })
                    .show(ui, |ui| {
                        for block in blocks {
                            self.block(ui, block, depth + 1);
                        }
                    });
                let bottom = ui.min_rect().bottom();
                ui.painter().line_segment(
                    [egui::pos2(left + 3.0, top), egui::pos2(left + 3.0, bottom)],
                    Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.6)),
                );
                ui.add_space(6.0);
            }
            Block::Rule => {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }
            Block::Image { src, alt } => {
                self.image(ui, src, alt);
            }
            Block::Anchor(name) => {
                self.anchors.insert(name.clone(), top);
            }
        }
        self.blocks.push(BlockAnchor {
            text_offset: start_offset,
            top,
        });
        self.text_offset = start_offset + block_text_length(block);
    }

    fn table(&mut self, ui: &mut Ui, table: &Table) {
        if let Some(caption) = &table.caption {
            ui.add_space(4.0);
            ui.label(
                RichText::new(caption)
                    .italics()
                    .color(theme::muted(ui.ctx())),
            );
        }
        let columns = table
            .rows
            .iter()
            .map(|row| row.cells.iter().map(|cell| cell.colspan.max(1) as usize).sum())
            .max()
            .unwrap_or(1)
            .max(1);
        let widths = self.column_widths(ui, table, columns);
        let spacing = 12.0;
        // A `Grid` only ever grows its remembered column widths: after the window is
        // resized narrower (or after the first, maximized frame) the table keeps its old,
        // wider layout and pushes the document card — and with it every following block —
        // past the panel edge. Draw the grid in a child clamped to the width the columns
        // were measured for, and allocate exactly that much width in the parent. Anything
        // the grid still wants beyond it is clipped instead of widening the page.
        let total = (widths.iter().sum::<f32>() + spacing * columns.saturating_sub(1) as f32)
            .min(ui.available_width())
            .max(24.0);
        // `table.rows.len()` alone is not unique: reference pages routinely have two
        // member tables with the same row count, which made egui log duplicate `Grid` ids
        // and drew one table on top of the other. The width is part of the id as well, so
        // a table laid out for a wider window starts from fresh column widths.
        let id = format!(
            "table:{}:{}:{}:{}",
            self.key,
            self.table_index,
            table.rows.len(),
            total.round() as i64
        );
        self.table_index += 1;
        let origin = ui.cursor().min;
        let clip = egui::Rect::from_min_max(
            egui::pos2(origin.x, ui.clip_rect().top()),
            egui::pos2(origin.x + total, ui.clip_rect().bottom()),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(
                    origin,
                    egui::vec2(total, ui.available_height().max(1.0)),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(clip);
        egui::Grid::new(id)
            .striped(true)
            .num_columns(columns)
            .spacing(Vec2::new(spacing, 6.0))
            .show(&mut child, |ui| {
                for (row_index, row) in table.rows.iter().enumerate() {
                    let is_header = row.is_header || row_index < table.header_rows;
                    let mut column = 0usize;
                    for cell in &row.cells {
                        let span = (cell.colspan.max(1) as usize).min(columns - column.max(1) + 1);
                        let cell_width = widths[column..(column + span).min(columns)]
                            .iter()
                            .sum::<f32>()
                            + spacing * span.saturating_sub(1) as f32;
                        column += span;
                        // A fixed width per cell is how a `Grid` gets sane column widths:
                        // without it every column collapses to its narrowest word.
                        //
                        // The cell is drawn in a *vertical* child. `Grid` adds its contents
                        // inside a horizontal ui, so a cell with several blocks (a summary
                        // with two paragraphs, a name plus a signature) used to lay them
                        // out side by side: everything after the first block had zero width
                        // left and wrapped one word per line.
                        //
                        // The child's measured height is then allocated as a single grid
                        // widget. `allocate_ui_with_layout` would tell the grid only the
                        // *requested* height (0), so a row never grew to fit its text and
                        // the following row's stripe was painted over the last line.
                        let cell_width = cell_width.max(24.0);
                        let cell_origin = ui.cursor().min;
                        let mut cell_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(egui::Rect::from_min_size(
                                    cell_origin,
                                    egui::vec2(cell_width, ui.available_height().max(1.0)),
                                ))
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        cell_ui.set_width(cell_width);
                        if is_header {
                            let strong = cell_ui.visuals().strong_text_color();
                            cell_ui.style_mut().visuals.override_text_color = Some(strong);
                        }
                        let text = cell_text(&cell.blocks);
                        let forced = widest_word(&cell_ui, &text, self.scale) > cell_width;
                        if forced {
                            self.break_anywhere_depth += 1;
                        }
                        self.cell_depth += 1;
                        let outer_wrap = self.cell_wrap.replace(cell_width);
                        for block in &cell.blocks {
                            self.block(&mut cell_ui, block, 1);
                        }
                        self.cell_wrap = outer_wrap;
                        self.cell_depth -= 1;
                        if forced {
                            self.break_anywhere_depth -= 1;
                        }
                        let used = cell_ui.min_rect();
                        ui.allocate_rect(used, egui::Sense::hover());
                        // `rowspan` is not modelled; padding keeps the columns aligned.
                        for _ in 1..span {
                            ui.label(" ");
                        }
                    }
                    ui.end_row();
                }
            });
        let height = child.min_rect().height();
        ui.allocate_rect(
            egui::Rect::from_min_size(origin, egui::vec2(total, height)),
            egui::Sense::hover(),
        );
    }

    /// Column widths for a table.
    ///
    /// `pref` is the width of the widest line (what the column would like) and `min` the
    /// width of the widest unbreakable token (what it needs so an identifier is not cut
    /// mid-word). When the table does not fit, prose columns give way down to their `min`
    /// before any column is broken mid-token — `PossibleParameters` used to wrap as
    /// `PossibleParamet` / `ers` on a 1280px window.
    fn column_widths(&self, ui: &Ui, table: &Table, columns: usize) -> Vec<f32> {
        const MIN_COLUMN: f32 = 40.0;
        const MAX_FRACTION: f32 = 0.6;
        let font = FontId::proportional(14.5 * self.scale);
        let measure = |text: &str| {
            ui.fonts(|fonts| {
                fonts
                    .layout_no_wrap(text.to_owned(), font.clone(), ui.visuals().text_color())
                    .rect
                    .width()
            })
        };
        let mut pref = vec![MIN_COLUMN; columns];
        let mut min = vec![MIN_COLUMN; columns];

        for row in &table.rows {
            let mut column = 0usize;
            for cell in &row.cells {
                let span = (cell.colspan.max(1) as usize).min(columns - column.max(1) + 1);
                if span == 0 {
                    continue;
                }
                let text = cell_text(&cell.blocks);
                let longest_line = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .max_by_key(|line| line.chars().count())
                    .unwrap_or("");
                let longest_token = text
                    .split_whitespace()
                    .max_by(|a, b| measure(a).total_cmp(&measure(b)))
                    .unwrap_or("");
                if !longest_line.is_empty() {
                    let measured = measure(longest_line).max(MIN_COLUMN) / span as f32;
                    for offset in 0..span {
                        if column + offset < columns {
                            pref[column + offset] = pref[column + offset].max(measured);
                        }
                    }
                }
                if !longest_token.is_empty() {
                    let measured = measure(longest_token).max(24.0) / span as f32;
                    for offset in 0..span {
                        if column + offset < columns {
                            min[column + offset] = min[column + offset].max(measured);
                        }
                    }
                }
                column += span;
            }
        }

        let spacing = 12.0 * columns.saturating_sub(1) as f32;
        let available = (ui.available_width() - spacing).max(columns as f32 * 32.0);
        // No single column may starve the others.
        for (wanted, needed) in pref.iter_mut().zip(&min) {
            *wanted = wanted.min(available * MAX_FRACTION).max(*needed);
        }
        let preferred: f32 = pref.iter().sum();
        if preferred <= available {
            return pref;
        }
        // Give every column what it needs, then share the slack in proportion to how much
        // more each column would *like* to have.
        let needed: f32 = min.iter().sum();
        if needed >= available {
            let factor = available / needed;
            return min.iter().map(|width| (width * factor).max(16.0)).collect();
        }
        let slack = available - needed;
        let wanted: f32 = pref
            .iter()
            .zip(&min)
            .map(|(wanted, needed)| (wanted - needed).max(0.0))
            .sum();
        if wanted <= 0.0 {
            return min;
        }
        min.iter()
            .zip(&pref)
            .map(|(needed, wanted)| needed + slack * (wanted - needed).max(0.0) / wanted)
            .collect()
    }

    fn code(&mut self, ui: &mut Ui, code: &CodeBlock) {
        if let Some(group_id) = code.group.as_deref() {
            if let Some(group) = self
                .snippet_groups
                .iter()
                .find(|candidate| candidate.id == group_id)
                .cloned()
            {
                if !self.drawn_groups.insert(group_id.to_owned()) {
                    return; // already rendered with its tabs
                }
                self.snippet_group(ui, &group);
                return;
            }
        }
        let text = code.lines.join("\n");
        self.code_block(
            ui,
            &text,
            code.display_language.as_deref(),
            code.copyable,
        );
    }

    /// A snippet group: language tabs (as in Microsoft Help Viewer) plus copy.
    fn snippet_group(&mut self, ui: &mut Ui, group: &SnippetGroup) {
        if group.codes.is_empty() {
            return;
        }
        let default_index = self
            .code_language
            .matching_index(&group.codes)
            .unwrap_or(0);
        let selected = self
            .view
            .snippet_choice
            .get(&group.id)
            .copied()
            .filter(|index| *index < group.codes.len())
            .unwrap_or(default_index);

        let mut newly_selected = None;
        ui.horizontal(|ui| {
            for (index, code) in group.codes.iter().enumerate() {
                let label = code
                    .display_language
                    .clone()
                    .or_else(|| code.language.clone())
                    .unwrap_or_else(|| format!("Language {}", index + 1));
                if ui
                    .selectable_label(index == selected, RichText::new(label).small())
                    .clicked()
                {
                    newly_selected = Some(index);
                }
            }
        });
        if let Some(index) = newly_selected {
            self.view.snippet_choice.insert(group.id.clone(), index);
        }
        let chosen = &group.codes[selected];
        let text = chosen.lines.join("\n");
        // The tab row already names the language; printing it again inside the frame
        // duplicated every `C#`/`VB` label (seen on real `iapi.mshc` pages).
        self.code_block(ui, &text, None, chosen.copyable);
    }

    fn code_block(&mut self, ui: &mut Ui, text: &str, language: Option<&str>, copyable: bool) {
        let mut copy = false;
        theme::code_frame(ui.ctx()).show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(language) = language {
                    ui.label(RichText::new(language).small().color(theme::ACCENT_ALT));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if copyable && ui.small_button("Copy").clicked() {
                        copy = true;
                    }
                });
            });
            let mut job = LayoutJob::default();
            job.wrap.max_width = ui.available_width();
            job.append(
                text,
                0.0,
                TextFormat {
                    font_id: theme::code_font(self.scale),
                    color: ui.visuals().text_color(),
                    ..Default::default()
                },
            );
            ui.add(egui::Label::new(job).selectable(true));
        });
        if copy {
            self.action_sink.push(RenderAction::Copy(text.to_owned()));
        }
    }

    fn image(&mut self, ui: &mut Ui, src: &str, alt: &str) {
        match self.assets.texture(ui.ctx(), self.set, self.key, src) {
            Some(texture) => {
                let size = texture.size_vec2();
                let max_width = ui.available_width().max(64.0);
                let factor = (max_width / size.x).min(1.0);
                let shown = Vec2::new(size.x * factor, size.y * factor);
                ui.add(egui::Image::new(&texture).fit_to_exact_size(shown));
            }
            None => {
                theme::note_frame(ui.ctx(), theme::ERROR).show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("⚠ image not available: {src}"))
                            .small()
                            .color(theme::muted(ui.ctx())),
                    );
                });
            }
        }
        if !alt.trim().is_empty() && self.cell_depth == 0 {
            // Caption styled like the rest of the small print (alt text). Member icons in
            // reference tables carry their access level in `alt`; captioning those turned
            // the icon column into a wrapped `Public method` block.
            let mut caption =
                JobBuilder::new(ui, TextKind::Small, self.scale, false, self.cell_wrap);
            caption.text(alt, TextKind::Small.format(ui, self.scale));
            caption.finish(ui, self.action_sink);
        }
        ui.add_space(6.0);
    }

    // ------------------------------------------------------------------ inline text

    fn label(&mut self, ui: &mut Ui, inlines: &[Inline], kind: TextKind) {
        let mut builder = JobBuilder::new(
            ui,
            kind,
            self.scale,
            self.break_anywhere_depth > 0,
            self.cell_wrap,
        );
        builder.highlight = self.highlight.clone();
        builder.find = self.find.clone();
        for inline in inlines {
            builder.inline(inline, kind.format(ui, self.scale), &self.code_language);
        }
        builder.finish(ui, self.action_sink);
    }
}

/// Builds the `LayoutJob` of one paragraph, tracking links and highlight ranges.
struct JobBuilder {
    job: LayoutJob,
    /// `(character range in the job text, raw href)`.
    links: Vec<(Range<usize>, String)>,
    /// Character count appended so far.
    column: usize,
    highlight: Vec<String>,
    find: Option<FindPattern>,
}

impl JobBuilder {
    fn new(
        ui: &Ui,
        kind: TextKind,
        scale: f32,
        break_anywhere: bool,
        wrap: Option<f32>,
    ) -> Self {
        let base = kind.format(ui, scale);
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap.unwrap_or_else(|| ui.available_width()).max(32.0);
        job.wrap.break_anywhere = break_anywhere;
        job.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: 0..0,
            format: base,
        });
        JobBuilder {
            job,
            links: Vec::new(),
            column: 0,
            highlight: Vec::new(),
            find: None,
        }
    }

    /// Local (0-based) occurrence ranges of a needle inside `text`.
    fn occurrences(text: &str, needle: &str, case_sensitive: bool) -> Vec<Range<usize>> {
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
                index += pat.len();
            } else {
                index += 1;
            }
        }
        out
    }

    /// All highlight/find ranges of `text`, in global (job) character coordinates.
    fn ranges_for(&self, text: &str) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for term in &self.highlight {
            if term.chars().count() < 2 {
                continue;
            }
            ranges.extend(
                Self::occurrences(text, term, false)
                    .into_iter()
                    .map(|range| (range.start + self.column)..(range.end + self.column)),
            );
        }
        if let Some(find) = &self.find {
            ranges.extend(
                Self::occurrences(text, &find.needle, find.case_sensitive)
                    .into_iter()
                    .map(|range| (range.start + self.column)..(range.end + self.column)),
            );
        }
        ranges.sort_by_key(|range| (range.start, range.end));
        ranges
    }

    /// Append text, splitting it into plain and highlighted sections.
    fn text(&mut self, text: &str, format: TextFormat) {
        if text.is_empty() {
            return;
        }
        let length = text.chars().count();
        let ranges = self.ranges_for(text);
        if ranges.is_empty() {
            self.job.append(text, 0.0, format);
            self.column += length;
            return;
        }
        let chars: Vec<char> = text.chars().collect();
        let mut cursor = 0usize;
        for range in ranges {
            let start = range.start.saturating_sub(self.column).max(cursor);
            let end = (range.end.saturating_sub(self.column)).min(chars.len());
            if start >= end || start >= chars.len() {
                continue;
            }
            if start > cursor {
                let chunk: String = chars[cursor..start].iter().collect();
                self.job.append(&chunk, 0.0, format.clone());
            }
            let chunk: String = chars[start..end].iter().collect();
            let mut highlighted = format.clone();
            highlighted.background = theme::ACCENT.gamma_multiply(0.40);
            self.job.append(&chunk, 0.0, highlighted);
            cursor = end;
        }
        if cursor < chars.len() {
            let chunk: String = chars[cursor..].iter().collect();
            self.job.append(&chunk, 0.0, format);
        }
        self.column += length;
    }

    fn inline(&mut self, inline: &Inline, format: TextFormat, language: &CodeLanguage) {
        match inline {
            Inline::Text(text) => self.text(text, format),
            Inline::Bold(inner) => {
                // egui has no bold face in its default fonts: "strong" means brighter + same
                // size, which is exactly what Microsoft's own topics rely on.
                let mut strong = format.clone();
                strong.color = theme::mix(format.color, Color32::WHITE, 0.35);
                for inline in inner {
                    self.inline(inline, strong.clone(), language);
                }
            }
            Inline::Italic(inner) => {
                let mut italic = format.clone();
                italic.italics = true;
                for inline in inner {
                    self.inline(inline, italic.clone(), language);
                }
            }
            Inline::Underline(inner) => {
                let mut underlined = format.clone();
                underlined.underline = Stroke::new(1.0, format.color);
                for inline in inner {
                    self.inline(inline, underlined.clone(), language);
                }
            }
            Inline::Strike(inner) => {
                let mut struck = format.clone();
                struck.strikethrough = Stroke::new(1.0, format.color);
                for inline in inner {
                    self.inline(inline, struck.clone(), language);
                }
            }
            Inline::Sub(inner) | Inline::Sup(inner) => {
                let mut small = format.clone();
                small.font_id = FontId::proportional(format.font_id.size * 0.75);
                for inline in inner {
                    self.inline(inline, small.clone(), language);
                }
            }
            Inline::Code(text) => {
                let mut code = format.clone();
                code.font_id = theme::code_font(1.0);
                code.background = theme::mix(format.background, theme::ACCENT, 0.10);
                self.text(text, code);
            }
            Inline::Link { target, inlines } => {
                let start = self.column;
                let mut link = format.clone();
                link.color = theme::ACCENT;
                link.underline = Stroke::new(1.0, theme::ACCENT);
                for inline in inlines {
                    self.inline(inline, link.clone(), language);
                }
                self.links.push((start..self.column, target.clone()));
            }
            Inline::Image { src, alt } => {
                // Inline images degrade to their alt text: block images are rendered properly.
                let label = if alt.trim().is_empty() {
                    format!("[image: {src}]")
                } else {
                    format!("[{alt}]")
                };
                self.text(&label, format);
            }
            // A real row break, not a space: block-level `<br>` separates signature lines
            // (`Type: ICustomScan` / `object containing…`) and collapsing it glued them.
            Inline::LineBreak => self.text("\n", format),
            Inline::LanguageSpecific { cs, vb, cpp, nu } => {
                let chosen = match language {
                    CodeLanguage::Cs => cs,
                    CodeLanguage::Vb => vb,
                    CodeLanguage::Cpp => cpp,
                    CodeLanguage::Other(_) => nu,
                };
                let text = if chosen.is_empty() { nu } else { chosen };
                let mut code = format.clone();
                code.font_id = theme::code_font(1.0);
                self.text(text, code);
            }
            Inline::Label(inner) => {
                let mut emphasised = format.clone();
                emphasised.color = theme::mix(format.color, theme::ACCENT_ALT, 0.5);
                for inline in inner {
                    self.inline(inline, emphasised.clone(), language);
                }
            }
        }
    }

    /// Draw the paragraph and turn clicks on link sections into actions.
    fn finish(self, ui: &mut Ui, actions: &mut Vec<RenderAction>) {
        let JobBuilder { job, links, .. } = self;
        if job.text.is_empty() {
            return;
        }
        // Lay the job out with the width chosen above and hand egui the finished galley.
        // `Label::wrap` would otherwise replace `max_width` with `ui.available_width()`,
        // which is zero in a `Grid` cell after its first widget and during the grid's
        // sizing pass — text then wrapped at 32 px and rows grew to thousands of pixels.
        let galley = ui.fonts(|fonts| fonts.layout_job(job.clone()));
        let response = ui.add(
            egui::Label::new(galley)
                .sense(Sense::click())
                .selectable(true),
        );

        if links.is_empty() {
            return;
        }
        let hit = |pos: egui::Pos2, rect: Rect| -> Option<usize> {
            if !rect.contains(pos) {
                return None;
            }
            let galley = ui.fonts(|fonts| fonts.layout_job(job.clone()));
            Some(galley.cursor_from_pos(pos - rect.min).index)
        };
        if response.hovered() {
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                if let Some(index) = hit(pos, response.rect) {
                    if links.iter().any(|(range, _)| range.contains(&index)) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
            }
        }
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(index) = hit(pos, response.rect) {
                    if let Some((_, target)) = links.iter().find(|(range, _)| range.contains(&index))
                    {
                        actions.push(RenderAction::Link(target.clone()));
                    }
                }
            }
        }
    }
}

impl CodeLanguage {
    /// Index of the snippet whose display language matches this preference.
    pub fn matching_index(&self, codes: &[CodeBlock]) -> Option<usize> {
        let wanted: Vec<String> = match self {
            CodeLanguage::Cs => vec!["c#".into(), "csharp".into(), "cs".into()],
            CodeLanguage::Vb => vec!["visual basic".into(), "vb".into(), "vbnet".into()],
            CodeLanguage::Cpp => vec!["c++".into(), "cpp".into(), "visualc++".into()],
            CodeLanguage::Other(name) => vec![name.to_lowercase()],
        };
        codes.iter().position(|code| {
            let label = code
                .display_language
                .as_deref()
                .or(code.language.as_deref())
                .unwrap_or("")
                .to_lowercase();
            wanted.iter().any(|needle| label.contains(needle.as_str()))
        })
    }
}

/// Length of a block's contribution to `Document::text` (mirrors `lucid-mshc`'s builder, so
/// the search highlight offsets line up with the block list).
fn block_text_length(block: &Block) -> usize {
    match block {
        Block::Heading { inlines, .. } | Block::Paragraph { inlines } => {
            lucid_mshc::model::inline_text(inlines).chars().count() + 1
        }
        Block::List { items, .. } => items.iter().map(|item| blocks_text_length(item)).sum(),
        Block::Table(table) => table
            .rows
            .iter()
            .flat_map(|row| row.cells.iter())
            .map(|cell| blocks_text_length(&cell.blocks))
            .sum(),
        Block::Code(code) => code
            .lines
            .iter()
            .map(|line| line.chars().count() + 1)
            .sum(),
        Block::Note { blocks, .. } | Block::Quote { blocks } => blocks_text_length(blocks),
        Block::Collapsible { title, blocks, .. } => {
            title.chars().count() + 1 + blocks_text_length(blocks)
        }
        Block::Image { alt, .. } => alt.chars().count() + 1,
        Block::Rule | Block::Anchor(_) => 0,
    }
}

fn blocks_text_length(blocks: &[Block]) -> usize {
    blocks.iter().map(block_text_length).sum()
}

/// Text a table cell actually shows.
///
/// Block images inside cells are drawn without their `alt` caption (they are member
/// icons), so the caption must not influence the column width either.
fn cell_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    push_cell_text(blocks, &mut out);
    out
}

fn push_cell_text(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Heading { inlines, .. } | Block::Paragraph { inlines } => {
                out.push_str(&lucid_mshc::model::inline_text(inlines));
                out.push('\n');
            }
            Block::List { items, .. } => {
                for item in items {
                    push_cell_text(item, out);
                }
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        push_cell_text(&cell.blocks, out);
                    }
                }
            }
            Block::Code(code) => {
                for line in &code.lines {
                    out.push_str(line);
                    out.push('\n');
                }
            }
            Block::Note { blocks, .. } | Block::Quote { blocks } => push_cell_text(blocks, out),
            Block::Collapsible { title, blocks, .. } => {
                out.push_str(title);
                out.push('\n');
                push_cell_text(blocks, out);
            }
            Block::Rule | Block::Image { .. } | Block::Anchor(_) => {}
        }
    }
}

/// Width of the widest whitespace-delimited token in `text`.
fn widest_word(ui: &Ui, text: &str, scale: f32) -> f32 {
    let font = FontId::proportional(14.5 * scale);
    text.split_whitespace()
        .map(|word| {
            ui.fonts(|fonts| {
                fonts
                    .layout_no_wrap(word.to_owned(), font.clone(), ui.visuals().text_color())
                    .rect
                    .width()
            })
        })
        .fold(0.0f32, f32::max)
}

/// Collect the snippet groups of a document (used for the language tabs).
fn collect_snippet_groups(blocks: &[Block]) -> Vec<SnippetGroup> {
    fn walk(blocks: &[Block], groups: &mut Vec<SnippetGroup>) {
        for block in blocks {
            match block {
                Block::Code(code) => {
                    if let Some(group) = code.group.as_deref() {
                        match groups.iter_mut().find(|candidate| candidate.id == group) {
                            Some(existing) => existing.codes.push(code.clone()),
                            None => groups.push(SnippetGroup {
                                id: group.to_owned(),
                                codes: vec![code.clone()],
                            }),
                        }
                    }
                }
                Block::List { items, .. } => {
                    for item in items {
                        walk(item, groups);
                    }
                }
                Block::Table(table) => {
                    for row in &table.rows {
                        for cell in &row.cells {
                            walk(&cell.blocks, groups);
                        }
                    }
                }
                Block::Note { blocks, .. } | Block::Quote { blocks } => walk(blocks, groups),
                Block::Collapsible { blocks, .. } => walk(blocks, groups),
                _ => {}
            }
        }
    }
    let mut groups = Vec::new();
    walk(blocks, &mut groups);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occurrences_are_unicode_aware_and_non_overlapping() {
        let ranges = JobBuilder::occurrences("café CafÉ café", "café", false);
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0], 0..4);
        assert_eq!(ranges[1], 5..9);
        let ranges = JobBuilder::occurrences("aaaa", "aa", false);
        assert_eq!(ranges, vec![0..2, 2..4]);
    }

    #[test]
    fn occurrences_respect_case_sensitivity() {
        assert_eq!(JobBuilder::occurrences("Phrase phrase", "phrase", true).len(), 1);
        assert_eq!(JobBuilder::occurrences("Phrase phrase", "phrase", false).len(), 2);
    }

    #[test]
    fn block_text_length_matches_the_model_offsets() {
        let document = lucid_mshc::model::parse_document(
            "<p>Hello world</p><h2>Section</h2><ul><li>one</li></ul>",
        );
        let total: usize = document.blocks.iter().map(block_text_length).sum();
        // "Hello world\n" + "Section\n" + "one\n"
        assert_eq!(total, 12 + 8 + 4);
    }

    #[test]
    fn snippet_groups_collect_members_in_order() {
        let document = lucid_mshc::model::parse_document(
            r#"<div id="snippetGroup">
<CodeSnippet Language="CSharp" DisplayLanguage="C#">a</CodeSnippet>
<CodeSnippet Language="VisualBasic" DisplayLanguage="VB">b</CodeSnippet>
</div>"#,
        );
        let groups = collect_snippet_groups(&document.blocks);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].codes.len(), 2);
        assert_eq!(groups[0].codes[0].display_language.as_deref(), Some("C#"));
    }

    #[test]
    fn code_language_preference_picks_the_right_snippet() {
        let codes = vec![
            CodeBlock {
                display_language: Some("Visual Basic".to_owned()),
                ..Default::default()
            },
            CodeBlock {
                display_language: Some("C#".to_owned()),
                ..Default::default()
            },
        ];
        assert_eq!(CodeLanguage::Cs.matching_index(&codes), Some(1));
        assert_eq!(CodeLanguage::Vb.matching_index(&codes), Some(0));
        assert_eq!(
            CodeLanguage::Other("F#".to_owned()).matching_index(&codes),
            None
        );
    }
}
