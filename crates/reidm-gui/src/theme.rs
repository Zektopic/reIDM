//! Visual style: a modernised version of the classic IDM window — light chrome, blue selection,
//! a dense bordered table. Colours are defined once here so the whole app stays consistent.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub row_alt: Color32,
}

impl Palette {
    pub fn light() -> Self {
        Self {
            bg: Color32::from_rgb(0xF0, 0xF0, 0xF0),
            panel: Color32::from_rgb(0xFA, 0xFA, 0xFA),
            panel_alt: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            border: Color32::from_rgb(0xC8, 0xC8, 0xC8),
            text: Color32::from_rgb(0x1A, 0x1A, 0x1A),
            text_dim: Color32::from_rgb(0x6C, 0x6C, 0x6C),
            accent: Color32::from_rgb(0x0A, 0x60, 0xC8),
            accent_text: Color32::WHITE,
            ok: Color32::from_rgb(0x18, 0x80, 0x38),
            warn: Color32::from_rgb(0xB0, 0x6A, 0x00),
            err: Color32::from_rgb(0xC0, 0x28, 0x28),
            row_alt: Color32::from_rgb(0xFA, 0xFC, 0xFF),
        }
    }

    pub fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(0x1E, 0x1E, 0x1E),
            panel: Color32::from_rgb(0x25, 0x25, 0x26),
            panel_alt: Color32::from_rgb(0x2D, 0x2D, 0x30),
            border: Color32::from_rgb(0x3C, 0x3C, 0x3C),
            text: Color32::from_rgb(0xE6, 0xE6, 0xE6),
            text_dim: Color32::from_rgb(0x9A, 0x9A, 0x9A),
            accent: Color32::from_rgb(0x2A, 0x84, 0xE0),
            accent_text: Color32::WHITE,
            ok: Color32::from_rgb(0x4C, 0xC0, 0x6A),
            warn: Color32::from_rgb(0xE0, 0xA0, 0x40),
            err: Color32::from_rgb(0xF0, 0x6A, 0x6A),
            row_alt: Color32::from_rgb(0x2A, 0x2A, 0x2E),
        }
    }

    pub fn status_color(&self, s: &crate::model::Status) -> Color32 {
        use crate::model::Status::*;
        match s {
            Running => self.accent,
            Done => self.ok,
            Paused => self.warn,
            Failed(_) => self.err,
            Queued => self.text_dim,
        }
    }
}

/// The palette for an explicit light/dark choice.
///
/// The app tracks its own theme flag rather than reading `Context::theme()`: on a platform whose
/// system-theme report can change underneath us, egui may resolve back to light and leave the
/// window half-restyled.
pub fn palette_for(dark: bool) -> Palette {
    if dark {
        Palette::dark()
    } else {
        Palette::light()
    }
}

fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.dark_mode = dark;
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.panel_alt;
    v.faint_bg_color = p.row_alt;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.widgets.noninteractive.bg_fill = p.panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.bg_fill = p.panel_alt;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.bg_fill = p.accent.gamma_multiply(0.18);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.active.bg_fill = p.accent.gamma_multiply(0.30);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    // Not `accent_text`: egui resolves `RichText::strong()` to `widgets.active.fg_stroke`, so a
    // white value here silently paints every strong label white-on-white.
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    v.window_corner_radius = CornerRadius::same(6);
    v
}

/// Apply the app-wide egui style. Called once at startup, and again whenever the theme toggles.
///
/// The same visuals are installed under *both* theme slots, so whichever one egui selects the
/// window keeps the look the user asked for.
pub fn apply(ctx: &egui::Context, dark: bool) {
    ctx.set_theme(if dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
    let v = visuals(&palette_for(dark), dark);
    ctx.all_styles_mut(|style| {
        style.visuals = v.clone();
        style.spacing.item_spacing = egui::vec2(6.0, 5.0);
        style.spacing.button_padding = egui::vec2(9.0, 4.0);
        style.spacing.menu_margin = egui::Margin::same(6);
    });
}
