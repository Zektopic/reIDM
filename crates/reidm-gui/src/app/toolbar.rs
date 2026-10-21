//! The toolbar of icon-over-label buttons, and the search box.

use super::*;

impl App {
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::top("toolbar")
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(8, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let sep = |ui: &mut egui::Ui| {
                        ui.add_space(3.0);
                        let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 44.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 0.0, p.border);
                        ui.add_space(3.0);
                    };

                    if tool(ui, p, "Add URL", Icon::Plus, true).clicked() {
                        self.open_add();
                    }
                    sep(ui);
                    if tool(ui, p, "Resume", Icon::Play, false).clicked() {
                        self.start_selected();
                    }
                    if tool(ui, p, "Pause", Icon::Pause, false).clicked() {
                        self.stop_selected();
                    }
                    if tool(ui, p, "Pause All", Icon::PauseAll, false).clicked() {
                        self.stop_all();
                    }
                    sep(ui);
                    if tool(ui, p, "Remove", Icon::Cross, false).clicked() {
                        self.ask_remove();
                    }
                    if tool(ui, p, "Remove\nCompleted", Icon::CrossAll, false).clicked() {
                        self.delete_completed();
                    }
                    sep(ui);
                    if tool(ui, p, "Options", Icon::Gear, false).clicked() {
                        self.open_options();
                    }
                    if tool(ui, p, "Scheduler", Icon::Clock, false).clicked() {
                        self.open_scheduler();
                    }
                    if tool(ui, p, "Start\nQueue", Icon::QueueStart, false).clicked() {
                        self.queue_now(true);
                    }
                    if tool(ui, p, "Stop\nQueue", Icon::QueueStop, false).clicked() {
                        self.queue_now(false);
                    }
                    sep(ui);
                    if tool(ui, p, "Site\nGrabber", Icon::Grabber, false).clicked() {
                        self.show_grabber = true;
                    }
                    if tool(ui, p, "Speed\nLimiter", Icon::Gauge, false).clicked() {
                        self.open_limiter();
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut s = std::mem::take(&mut self.search);
                        ui.add_sized(Vec2::new(160.0, 24.0), egui::TextEdit::singleline(&mut s).hint_text("Search…"));
                        self.search = s;
                        let (sr, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                        icons::search(ui.painter(), sr, p.text_dim);
                    });
                });
            });
    }
}

// ── toolbar button ───────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Plus,
    Play,
    Pause,
    PauseAll,
    Cross,
    CrossAll,
    Gear,
    Clock,
    QueueStart,
    QueueStop,
    Grabber,
    Gauge,
}

/// An IDM-style toolbar button: a large icon with a small caption underneath. Two-line
/// captions (e.g. "Delete\nCompleted") are supported, which is why the height is fixed
/// rather than derived from the text.
pub(super) fn tool(ui: &mut egui::Ui, p: &Palette, label: &str, icon: Icon, primary: bool) -> egui::Response {
    let lines: Vec<&str> = label.split('\n').collect();
    let widest = lines
        .iter()
        .map(|l| ui.painter().layout_no_wrap((*l).to_string(), egui::FontId::proportional(12.0), p.text).size().x)
        .fold(0.0f32, f32::max);
    let w = (widest + 12.0).max(52.0);
    // Room for two lines of 12-point caption under the icon.
    let h = 56.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::click());
    let fg = if primary { p.accent_text } else { p.text };

    if primary {
        let bg = if resp.hovered() { p.accent.gamma_multiply(0.85) } else { p.accent };
        ui.painter().rect_filled(rect, 5.0, bg);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, p.accent.gamma_multiply(0.16));
        ui.painter().rect_stroke(rect, 5.0, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    }

    let icon_sz = 20.0;
    let icon_rect = egui::Rect::from_center_size(egui::Pos2::new(rect.center().x, rect.top() + 6.0 + icon_sz / 2.0), Vec2::splat(icon_sz));
    let painter = ui.painter();
    match icon {
        Icon::Plus => icons::plus(painter, icon_rect, fg),
        Icon::Play => icons::play(painter, icon_rect, fg),
        Icon::Pause => icons::pause(painter, icon_rect, fg),
        Icon::PauseAll => icons::pause_all(painter, icon_rect, fg),
        Icon::Cross => icons::cross(painter, icon_rect, fg),
        Icon::CrossAll => icons::cross_all(painter, icon_rect, fg),
        Icon::Gear => icons::gear(painter, icon_rect, fg),
        Icon::Clock => icons::clock(painter, icon_rect, fg),
        Icon::QueueStart => icons::queue_start(painter, icon_rect, fg),
        Icon::QueueStop => icons::queue_stop(painter, icon_rect, fg),
        Icon::Grabber => icons::grabber(painter, icon_rect, fg),
        Icon::Gauge => icons::gauge(painter, icon_rect, fg),
    }
    for (i, line) in lines.iter().enumerate() {
        painter.text(egui::Pos2::new(rect.center().x, rect.top() + 28.0 + i as f32 * 13.0), egui::Align2::CENTER_TOP, *line, egui::FontId::proportional(12.0), fg);
    }
    accessible(ui, &resp, p, egui::WidgetType::Button, label, None);
    resp.on_hover_text(label.replace('\n', " "))
}
