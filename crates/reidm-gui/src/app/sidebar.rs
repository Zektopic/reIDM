//! The category tree down the left.

use super::*;

impl App {
    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::left("cats")
            .default_size(204.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(6, 8)))
            .show(ui, |ui| {
                ui.label(RichText::new("CATEGORIES").size(10.0).color(p.text_dim).strong());
                ui.add_space(2.0);

                // IDM nests its categories under "Categories > All Downloads"; mirror that with
                // real expanders rather than a flat list.
                let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), egui::Sense::click());
                icons::chevron(ui.painter(), egui::Rect::from_min_size(r.min + Vec2::new(2.0, 4.0), Vec2::splat(12.0)), p.text_dim, self.cats_open);
                ui.painter().text(r.min + Vec2::new(18.0, 10.0), egui::Align2::LEFT_CENTER, "Categories", egui::FontId::proportional(11.5), p.text_dim);
                if resp.clicked() {
                    self.cats_open = !self.cats_open;
                }

                if self.cats_open {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
                    icons::chevron(ui.painter(), egui::Rect::from_min_size(r.min + Vec2::new(16.0, 5.0), Vec2::splat(12.0)), p.text, self.downloads_open);
                    icons::folder(ui.painter(), egui::Rect::from_min_size(r.min + Vec2::new(30.0, 4.0), Vec2::splat(14.0)), p.text_dim);
                    let all = View::Category(Category::All);
                    let on_all = self.view == all;
                    ui.painter().text(r.min + Vec2::new(50.0, 11.0), egui::Align2::LEFT_CENTER, "All Downloads", egui::FontId::proportional(12.5), if on_all { p.accent } else { p.text });
                    ui.painter().text(r.right_center() - Vec2::new(8.0, 0.0), egui::Align2::RIGHT_CENTER, self.vm.counts.total.to_string(), egui::FontId::proportional(11.5), p.text_dim);
                    if resp.clicked() {
                        // The chevron toggles; the label selects.
                        if ui.input(|i| i.pointer.interact_pos()).is_some_and(|pos| pos.x < r.min.x + 30.0) {
                            self.downloads_open = !self.downloads_open;
                        } else {
                            self.view = all;
                        }
                    }

                    if self.downloads_open {
                        for c in CATEGORIES.into_iter().filter(|c| *c != Category::All) {
                            let v = View::Category(c);
                            let dark = self.prefs.dark;
                            if tree_row(ui, p, 34.0, c.label(), Some(self.vm.counts.of(v)), self.view == v, |pt, r| icons::file_card(pt, r, c, dark)) {
                                self.view = v;
                            }
                        }
                    }
                    for (v, done) in [(View::Unfinished, false), (View::Finished, true)] {
                        let col = if done { p.ok } else { p.accent };
                        if tree_row(ui, p, 16.0, v.label(), Some(self.vm.counts.of(v)), self.view == v, |pt, r| {
                            if done {
                                icons::dot(pt, r, col)
                            } else {
                                icons::play(pt, r, col)
                            }
                        }) {
                            self.view = v;
                        }
                    }
                }

                ui.add_space(8.0);
                ui.label(RichText::new("QUEUES").size(10.0).color(p.text_dim).strong());
                let q = View::Queue;
                let qcol = p.text_dim;
                if tree_row(ui, p, 4.0, q.label(), Some(self.vm.counts.of(q)), self.view == q, |pt, r| icons::clock(pt, r, qcol)) {
                    self.view = q;
                }
                ui.add_space(8.0);
                ui.label(RichText::new("GRABBER").size(10.0).color(p.text_dim).strong());
                let gcol = p.text_dim;
                if tree_row(ui, p, 4.0, "Site grabber…", None, false, |pt, r| icons::grabber(pt, r, gcol)) {
                    self.show_grabber = true;
                }
            });
    }
}

/// One row of the category tree. Returns whether it was clicked.
pub(super) fn tree_row(ui: &mut egui::Ui, p: &Palette, indent: f32, label: &str, n: Option<usize>, on: bool, icon: impl FnOnce(&egui::Painter, egui::Rect)) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 23.0), egui::Sense::click());
    if on {
        ui.painter().rect_filled(rect, 3.0, p.accent.gamma_multiply(0.22));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, p.accent.gamma_multiply(0.09));
    }
    icon(ui.painter(), egui::Rect::from_min_size(rect.min + Vec2::new(indent, 4.0), Vec2::splat(15.0)));
    ui.painter().text(rect.min + Vec2::new(indent + 22.0, 11.5), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), if on { p.accent } else { p.text });
    if let Some(n) = n {
        ui.painter().text(rect.right_center() - Vec2::new(8.0, 0.0), egui::Align2::RIGHT_CENTER, n.to_string(), egui::FontId::proportional(11.5), p.text_dim);
    }
    resp.clicked()
}
