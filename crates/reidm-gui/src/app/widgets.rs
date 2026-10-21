//! Small pieces the dialogs share.

use super::*;

pub(super) fn window(title: &str) -> egui::Window<'_> {
    egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
}

pub(super) fn label(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).color(p.text_dim).size(12.0));
}

/// A segment map: the whole file as a bar, each range's downloaded part filled, the ranges a
/// connection is working on marked at their write position.
pub(super) fn segment_map(ui: &mut egui::Ui, p: &Palette, it: &Item, segments: &[reidm_core::Segment]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, p.border.gamma_multiply(0.5));
    let total = it.size.unwrap_or(0);
    if it.status == Status::Done {
        painter.rect_filled(rect, 3.0, p.ok);
        return;
    }
    if total == 0 || segments.is_empty() {
        if let Some(f) = it.fraction() {
            painter.rect_filled(egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * f, rect.height())), 3.0, p.accent);
        }
        return;
    }
    let x = |b: u64| rect.left() + rect.width() * (b.min(total) as f32 / total as f32);
    for s in segments {
        let end = if s.end == u64::MAX { total } else { s.end + 1 };
        let filled = if s.finished { end } else { (s.start + s.done).min(end) };
        if filled > s.start {
            painter.rect_filled(egui::Rect::from_x_y_ranges(x(s.start)..=x(filled), rect.y_range()), 0.0, if s.active { p.accent } else { p.accent.gamma_multiply(0.75) });
        }
        // Range boundaries, so the split is visible.
        painter.line_segment([egui::Pos2::new(x(s.start), rect.top()), egui::Pos2::new(x(s.start), rect.bottom())], egui::Stroke::new(1.0, p.panel_alt));
        if s.active {
            painter.line_segment([egui::Pos2::new(x(filled), rect.top() - 2.0), egui::Pos2::new(x(filled), rect.bottom() + 2.0)], egui::Stroke::new(2.0, p.warn));
        }
    }
    painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
}

/// A visible keyboard focus for widgets the window paints itself (egui draws one only for its
/// own), and a name for screen readers, which can't read painted text.
pub(super) fn accessible(ui: &egui::Ui, resp: &egui::Response, p: &Palette, typ: egui::WidgetType, name: &str, selected: Option<bool>) {
    let name = name.replace('\n', " ");
    resp.widget_info(|| match selected {
        Some(s) => egui::WidgetInfo::selected(typ, true, s, &name),
        None => egui::WidgetInfo::labeled(typ, true, &name),
    });
    if resp.has_focus() {
        ui.painter().rect_stroke(resp.rect.shrink(1.0), 4.0, egui::Stroke::new(2.0, p.accent_fg), egui::StrokeKind::Inside);
    }
}
