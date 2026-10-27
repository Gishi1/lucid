//! The look: a neutral slate palette, one accent colour, an 8-point grid.
//!
//! Deliberately centralised: every colour and spacing value the UI uses lives here, so the
//! application has no hard-coded styling scattered around.

use egui::{Color32, Context, FontFamily, FontId, Margin, Stroke, TextStyle};

use crate::state::ThemeChoice;

/// Accent colour used for selection, links and highlights.
pub const ACCENT: Color32 = Color32::from_rgb(0x4F, 0x8C, 0xFF);
/// Secondary accent (F1 keywords, code snippet language tabs).
pub const ACCENT_ALT: Color32 = Color32::from_rgb(0x34, 0xD3, 0x99);
/// Warning colour (notes, warnings in the status bar).
pub const WARN: Color32 = Color32::from_rgb(0xE8, 0xB3, 0x39);
/// Error colour.
pub const ERROR: Color32 = Color32::from_rgb(0xF0, 0x6A, 0x6A);

/// Apply the theme and font scale to a context.
pub fn apply(ctx: &Context, theme: ThemeChoice, font_scale: f32) {
    let scale = font_scale.clamp(0.8, 1.8);
    let mut visuals = match theme {
        ThemeChoice::Dark => dark_visuals(),
        ThemeChoice::Light => light_visuals(),
    };
    visuals.override_text_color = None;
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        // 8-point grid.
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.menu_margin = Margin::same(8);
        style.spacing.indent = 16.0;
        style.spacing.scroll.bar_width = 10.0;
        style.spacing.interact_size.y = 24.0;
        // Text selection and copy inside labels (the document pane needs it).
        style.interaction.selectable_labels = true;
        style.interaction.show_tooltips_only_when_still = true;

        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5 * scale)),
            (TextStyle::Body, FontId::proportional(14.5 * scale)),
            (TextStyle::Button, FontId::proportional(14.0 * scale)),
            (TextStyle::Heading, FontId::proportional(19.0 * scale)),
            (TextStyle::Monospace, FontId::monospace(13.0 * scale)),
        ]
        .into();

        style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
        style.visuals.window_corner_radius = egui::CornerRadius::same(10);
    });
}

fn dark_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    let bg = Color32::from_rgb(0x11, 0x13, 0x18);
    let surface = Color32::from_rgb(0x18, 0x1B, 0x21);
    let elevated = Color32::from_rgb(0x21, 0x25, 0x2D);
    let line = Color32::from_rgb(0x2A, 0x2F, 0x3A);
    let text = Color32::from_rgb(0xE6, 0xE8, 0xEE);
    let muted = Color32::from_rgb(0x9A, 0xA3, 0xB2);

    visuals.panel_fill = surface;
    visuals.window_fill = elevated;
    visuals.extreme_bg_color = bg;
    visuals.faint_bg_color = elevated;
    visuals.override_text_color = Some(text);
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.window_stroke = Stroke::new(1.0, line);

    visuals.widgets.noninteractive.bg_fill = surface;
    visuals.widgets.noninteractive.weak_bg_fill = surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, line);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);

    visuals.widgets.inactive.bg_fill = elevated;
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, line);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text);

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x2C, 0x33, 0x40);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x24, 0x2A, 0x34);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.25);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.open.bg_fill = elevated;
    visuals.widgets.open.weak_bg_fill = elevated;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, line);

    let _ = muted;
    visuals
}

fn light_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    let bg = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    let surface = Color32::from_rgb(0xF7, 0xF8, 0xFA);
    let elevated = Color32::from_rgb(0xEF, 0xF1, 0xF5);
    let line = Color32::from_rgb(0xD7, 0xDC, 0xE4);
    let text = Color32::from_rgb(0x1B, 0x1F, 0x27);

    visuals.panel_fill = surface;
    visuals.window_fill = bg;
    visuals.extreme_bg_color = bg;
    visuals.faint_bg_color = elevated;
    visuals.override_text_color = Some(text);
    visuals.hyperlink_color = Color32::from_rgb(0x1F, 0x5F, 0xD1);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.25);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.window_stroke = Stroke::new(1.0, line);

    visuals.widgets.noninteractive.bg_fill = surface;
    visuals.widgets.noninteractive.weak_bg_fill = surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, line);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);

    visuals.widgets.inactive.bg_fill = elevated;
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, line);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text);

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0xE2, 0xE7, 0xF0);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xEA, 0xEE, 0xF5);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::BLACK);

    visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.2);
    visuals.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.15);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::BLACK);

    visuals.widgets.open.bg_fill = elevated;
    visuals.widgets.open.weak_bg_fill = elevated;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, line);

    visuals
}

/// Frame for the document surface (slightly different from the chrome, so topics stand out).
pub fn document_frame(ctx: &Context) -> egui::Frame {
    let visuals = ctx.style().visuals.clone();
    egui::Frame::new()
        .fill(visuals.extreme_bg_color)
        .inner_margin(Margin::symmetric(24, 20))
        .corner_radius(egui::CornerRadius::same(8))
}

/// Frame for note/callout blocks inside a topic.
pub fn note_frame(ctx: &Context, kind_colour: Color32) -> egui::Frame {
    let visuals = ctx.style().visuals.clone();
    let base = visuals.extreme_bg_color;
    egui::Frame::new()
        .fill(mix(base, kind_colour, 0.12))
        .inner_margin(Margin::symmetric(12, 10))
        .corner_radius(egui::CornerRadius::same(6))
        .stroke(Stroke::new(1.0, kind_colour.gamma_multiply(0.5)))
}

/// Frame for code blocks and snippets.
pub fn code_frame(ctx: &Context) -> egui::Frame {
    let visuals = ctx.style().visuals.clone();
    egui::Frame::new()
        .fill(mix(visuals.extreme_bg_color, visuals.panel_fill, 0.5))
        .inner_margin(Margin::symmetric(10, 8))
        .corner_radius(egui::CornerRadius::same(6))
        .stroke(Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color))
}

/// Frame for cards in the search results / keyword lists.
pub fn card_frame(ctx: &Context, selected: bool) -> egui::Frame {
    let visuals = ctx.style().visuals.clone();
    let fill = if selected {
        mix(visuals.extreme_bg_color, ACCENT, 0.18)
    } else {
        visuals.panel_fill
    };
    egui::Frame::new()
        .fill(fill)
        .inner_margin(Margin::symmetric(10, 8))
        .corner_radius(egui::CornerRadius::same(6))
        .stroke(Stroke::new(
            1.0,
            if selected {
                ACCENT
            } else {
                visuals.widgets.noninteractive.bg_stroke.color
            },
        ))
}

/// Muted text colour (hints, breadcrumbs, counters).
pub fn muted(ctx: &Context) -> Color32 {
    let visuals = ctx.style().visuals.clone();
    mix(visuals.override_text_color.unwrap_or(Color32::GRAY), visuals.panel_fill, 0.45)
}

/// Blend two colours (`t` = 0 → `a`, 1 → `b`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let blend = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        blend(a.r(), b.r()),
        blend(a.g(), b.g()),
        blend(a.b(), b.b()),
        blend(a.a(), b.a()),
    )
}

/// Background colour used behind highlighted search terms.
pub fn highlight_colour(ctx: &Context) -> Color32 {
    let theme_is_dark = ctx.style().visuals.dark_mode;
    if theme_is_dark {
        ACCENT.gamma_multiply(0.45)
    } else {
        ACCENT.gamma_multiply(0.3)
    }
}

/// Monospace font id for code blocks.
pub fn code_font(scale: f32) -> FontId {
    FontId::new(13.0 * scale.clamp(0.8, 1.8), FontFamily::Monospace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_interpolates_between_colours() {
        let a = Color32::from_rgb(0, 0, 0);
        let b = Color32::from_rgb(255, 255, 255);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5).r(), 128);
        // Out-of-range factors clamp instead of wrapping.
        assert_eq!(mix(a, b, -3.0), a);
        assert_eq!(mix(a, b, 7.0), b);
    }

    #[test]
    fn palettes_differ_between_themes() {
        let dark = dark_visuals();
        let light = light_visuals();
        assert!(dark.dark_mode);
        assert!(!light.dark_mode);
        assert_ne!(dark.panel_fill, light.panel_fill);
    }
}
