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
    // `selection.stroke.color` is also the text colour of a *selected* selectable label
    // (tabs, chips, list rows), so it must contrast with `selection.bg_fill` — an accent
    // colour here produced blue-on-blue labels at ~3:1.
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0xF2, 0xF4, 0xF9));
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
    // Chrome lines: visible without shouting (the previous #D7DCE4 was ~1.5:1).
    let line = Color32::from_rgb(0xC5, 0xCD, 0xD9);
    let text = Color32::from_rgb(0x1B, 0x1F, 0x27);

    visuals.panel_fill = surface;
    visuals.window_fill = bg;
    visuals.extreme_bg_color = bg;
    visuals.faint_bg_color = elevated;
    visuals.override_text_color = Some(text);
    visuals.hyperlink_color = Color32::from_rgb(0x1F, 0x5F, 0xD1);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.25);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0x12, 0x16, 0x1C));
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
///
/// Fixed values per theme rather than "blend the text towards the panel": blending towards a
/// light background produces unreadable low-contrast text (which is exactly what the light
/// theme did before this was pinned down).
pub fn muted(ctx: &Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(0x9A, 0xA3, 0xB2)
    } else {
        Color32::from_rgb(0x5A, 0x62, 0x70)
    }
}

/// Colour for secondary/strong text that must stay readable on both themes.
pub fn strong_text(ctx: &Context) -> Color32 {
    if ctx.style().visuals.dark_mode {
        Color32::from_rgb(0xF2, 0xF4, 0xF9)
    } else {
        Color32::from_rgb(0x12, 0x16, 0x1C)
    }
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

    /// Composite a (premultiplied) colour over an opaque background.
    fn over(fill: Color32, background: Color32) -> Color32 {
        let alpha = fill.a() as f32 / 255.0;
        let blend = |front: u8, back: u8| {
            (front as f32 + back as f32 * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8
        };
        Color32::from_rgb(
            blend(fill.r(), background.r()),
            blend(fill.g(), background.g()),
            blend(fill.b(), background.b()),
        )
    }

    #[test]
    fn selected_labels_are_readable() {
        // egui paints a selected `selectable_label` with `selection.stroke.color` on top of
        // `selection.bg_fill` (which is semi-transparent, so it must be composited first).
        for visuals in [dark_visuals(), light_visuals()] {
            let pill = over(visuals.selection.bg_fill, visuals.panel_fill);
            let ratio = contrast_ratio(visuals.selection.stroke.color, pill);
            assert!(
                ratio >= 4.5,
                "selected label contrast {ratio:.2} ({:?} on {pill:?})",
                visuals.selection.stroke.color
            );
        }
    }

    #[test]
    fn accent_on_panel_is_usable_for_links() {
        for visuals in [dark_visuals(), light_visuals()] {
            let ratio = contrast_ratio(visuals.hyperlink_color, visuals.extreme_bg_color);
            assert!(ratio >= 4.5, "link contrast {ratio:.2}");
        }
    }

    #[test]
    fn palettes_differ_between_themes() {
        let dark = dark_visuals();
        let light = light_visuals();
        assert!(dark.dark_mode);
        assert!(!light.dark_mode);
        assert_ne!(dark.panel_fill, light.panel_fill);
    }

    /// WCAG relative luminance.
    fn relative_luminance(colour: Color32) -> f32 {
        fn channel(value: u8) -> f32 {
            let value = value as f32 / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(colour.r()) + 0.7152 * channel(colour.g()) + 0.0722 * channel(colour.b())
    }

    /// WCAG contrast ratio between two colours (1.0 = identical, 21.0 = black on white).
    fn contrast_ratio(a: Color32, b: Color32) -> f32 {
        let (first, second) = (relative_luminance(a), relative_luminance(b));
        let (lighter, darker) = if first > second {
            (first, second)
        } else {
            (second, first)
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn muted_text_keeps_contrast_on_both_themes() {
        // Small secondary text must stay at WCAG AA (4.5:1) against its panel.
        let dark_muted = Color32::from_rgb(0x9A, 0xA3, 0xB2);
        let dark_bg = dark_visuals().panel_fill;
        assert!(
            contrast_ratio(dark_muted, dark_bg) >= 4.5,
            "dark muted contrast {}",
            contrast_ratio(dark_muted, dark_bg)
        );

        let light_muted = Color32::from_rgb(0x5A, 0x62, 0x70);
        let light_bg = light_visuals().panel_fill;
        assert!(
            contrast_ratio(light_muted, light_bg) >= 4.5,
            "light muted contrast {}",
            contrast_ratio(light_muted, light_bg)
        );

        // The previous implementation blended the text towards the panel, which fails AA on
        // the light theme — this is the regression the assertion above is protecting.
        let old_light = mix(Color32::from_rgb(0x1B, 0x1F, 0x27), light_bg, 0.45);
        assert!(
            contrast_ratio(old_light, light_bg) < 4.5,
            "the old blend was expected to fail AA, got {}",
            contrast_ratio(old_light, light_bg)
        );

        // Strong text is meant to be the opposite: high contrast on both themes.
        for (text, bg) in [
            (Color32::from_rgb(0xF2, 0xF4, 0xF9), dark_bg),
            (Color32::from_rgb(0x12, 0x16, 0x1C), light_bg),
        ] {
            assert!(contrast_ratio(text, bg) >= 12.0);
        }
    }
}
