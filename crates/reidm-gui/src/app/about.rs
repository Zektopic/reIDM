//! About reIDM.

use super::*;

impl App {
    pub(super) fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.about {
            return;
        }
        let mut open = true;
        let mut do_check = false;
        window("About reIDM").open(&mut open).show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.label(RichText::new(format!("reIDM {}", env!("CARGO_PKG_VERSION"))).strong().size(16.0));
            ui.add_space(6.0);
            ui.label("A cross-platform download manager that takes over your browser's downloads and speaks the IDM browser-extension protocol.");
            ui.add_space(8.0);
            if let Some(u) = &self.update {
                if u.checking {
                    ui.label(RichText::new("Checking for updates…").color(egui::Color32::from_gray(140)).size(11.0));
                } else if !u.version.is_empty() {
                    ui.label(RichText::new(format!("Version {} is available.", u.version)).color(egui::Color32::from_rgb(0x2A, 0x84, 0xE0)).size(12.0));
                } else if let Some(e) = &u.error {
                    ui.label(RichText::new(format!("Update check failed: {e}")).color(egui::Color32::from_rgb(0xB0, 0x6A, 0x00)).size(11.0));
                }
            }
            if !self.snap.service.version.is_empty() && self.snap.service.version != env!("CARGO_PKG_VERSION") {
                ui.label(RichText::new(format!("The download service running is version {}; restart it to use this one.", self.snap.service.version)).color(egui::Color32::from_rgb(0xB0, 0x6A, 0x00)).size(11.0));
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Not affiliated with, endorsed by, or derived from Tonec Inc.").color(egui::Color32::from_gray(140)).size(11.0));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Check for updates").clicked() {
                    do_check = true;
                }
                if ui.button("Close").clicked() {
                    self.about = false;
                }
            });
        });
        if do_check {
            self.check_for_updates();
        }
        if !open {
            self.about = false;
        }
    }
}
