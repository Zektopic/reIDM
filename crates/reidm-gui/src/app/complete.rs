//! IDM's *Download complete* window.

use super::*;

impl App {
    /// IDM's "Download complete" window, one finished download at a time.
    pub(super) fn complete_window(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(&id) = self.completed.first() else { return };
        let Some(it) = self.item(id).cloned() else {
            self.completed.remove(0);
            return;
        };
        let mut close = false;
        let mut action: Option<&'static str> = None;
        let mut never = false;
        egui::Window::new("Download complete")
            .id(egui::Id::new(("complete", id)))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -40.0))
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
                    icons::file_card(ui.painter(), r, it.category(), self.prefs.dark);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(it.file_name()).strong());
                        let folder = it.torrent.as_ref().and_then(|t| t.folder.clone()).or_else(|| it.path.parent().map(|d| d.to_path_buf()));
                        ui.label(RichText::new(format!("{} · {}", it.size.map(size_human).unwrap_or_default(), folder.map(|d| d.display().to_string()).unwrap_or_default())).color(p.text_dim).size(11.5));
                    });
                });
                if self.completed.len() > 1 {
                    ui.label(RichText::new(format!("and {} more finished", self.completed.len() - 1)).color(p.text_dim).size(11.0));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Open").strong()).clicked() {
                        action = Some("open");
                    }
                    if ui.button("Open folder").clicked() {
                        action = Some("folder");
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Don't show again").clicked() {
                            never = true;
                        }
                    });
                });
            });
        if let Some(a) = action {
            self.action(id, a);
            close = true;
        }
        if never {
            self.prefs.on_finish = Some(crate::notify::OnFinish::Nothing);
            self.prefs.save();
            self.completed.clear();
            self.toast("Turn it back on in Options › General");
        } else if close {
            self.completed.retain(|x| *x != id);
        }
    }
}
