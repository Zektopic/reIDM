//! The bars across the top of the window: storage warnings and a new release.

use super::*;

impl App {
    /// What the service says is wrong with the download list's storage (a damaged list restored
    /// from its backup, saves failing). Stays until the service stops reporting it.
    pub(super) fn warnings_banner(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let warnings = self.snap.service.warnings.clone();
        egui::Panel::top("warnings")
            .frame(egui::Frame::NONE.fill(p.warn.gamma_multiply(0.18)).inner_margin(egui::Margin::symmetric(10, 6)))
            .show(ui, |ui| {
                for w in &warnings {
                    ui.horizontal(|ui| {
                        icons::dot(ui.painter(), egui::Rect::from_center_size(ui.cursor().min + Vec2::new(5.0, 8.0), Vec2::splat(10.0)), p.warn);
                        ui.add_space(14.0);
                        ui.label(RichText::new(w).color(p.text));
                    });
                }
            });
    }

    pub(super) fn update_banner(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let Some(u) = &self.update else { return };
        let (version, url, asset) = (u.version.clone(), u.url.clone(), u.asset.clone());
        let mut open = false;
        let mut dismiss = false;

        egui::Panel::top("update")
            .frame(egui::Frame::NONE.fill(p.accent.gamma_multiply(0.16)).inner_margin(egui::Margin::symmetric(10, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    icons::dot(ui.painter(), egui::Rect::from_center_size(ui.cursor().min + Vec2::new(5.0, 8.0), Vec2::splat(10.0)), p.accent);
                    ui.add_space(14.0);
                    ui.label(RichText::new(format!("reIDM {version} is available — you have {}", env!("CARGO_PKG_VERSION"))).color(p.text));
                    if let Some((name, _)) = &asset {
                        ui.label(RichText::new(format!("({name})")).color(p.text_dim).size(12.0));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            dismiss = true;
                        }
                        if ui.small_button("Release notes").clicked() {
                            open = true;
                        }
                    });
                });
            });

        if dismiss {
            if let Some(u) = &mut self.update {
                u.dismissed = true;
            }
        }
        if open {
            self.about = false;
            open_external(&url);
        }
    }
}
