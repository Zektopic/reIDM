//! The first-run welcome: where downloads go, and how to connect the browser. Two things, one
//! window, skippable; Help › Welcome… shows it again.

use super::*;

impl App {
    pub(super) fn welcome_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if self.prefs.welcomed {
            return;
        }
        let mut done = false;
        let mut open = true;
        let mut change = false;
        let mut setup = false;
        window("Welcome to reIDM").open(&mut open).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.label("reIDM downloads in the background, with many connections at once, and carries on where it stopped.");
            ui.add_space(10.0);

            ui.label(RichText::new("1. Downloads go to").strong());
            ui.horizontal(|ui| {
                match self.snap.service.settings.as_ref() {
                    Some(s) => ui.add(egui::Label::new(RichText::new(&s.default_dir).monospace().color(p.text)).truncate()),
                    None => ui.label(RichText::new("…waiting for the download service").color(p.text_dim)),
                };
                if ui.add_enabled(self.snap.service.settings.is_some() && !self.picker.busy(), egui::Button::new("Change…")).clicked() {
                    change = true;
                }
            });
            label(ui, p, "Videos, music, archives and programs each get a folder inside it. Options › Folders changes that.");
            ui.add_space(10.0);

            ui.label(RichText::new("2. Connect your browser").strong());
            match self.snap.service.extension.as_ref().filter(|e| e.seconds_ago < 180) {
                Some(e) => {
                    ui.label(RichText::new(format!("Connected: {} hands its downloads to reIDM.", if e.browser.is_empty() { "your browser" } else { &e.browser })).color(p.ok));
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Not connected yet.").color(p.warn));
                        if ui.button("Set up the extension…").clicked() {
                            setup = true;
                        }
                    });
                    label(ui, p, "Install reIDM's extension once in each browser you use (Chrome, Edge, Brave, Vivaldi, Opera). This turns green when it connects.");
                }
            }
            if self.tray.is_some() {
                ui.add_space(10.0);
                label(ui, p, "Closing this window keeps reIDM in the notification area, so downloads carry on.");
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let b = ui.add(egui::Button::new(RichText::new("Get started").strong()).min_size(Vec2::new(120.0, 30.0)));
                if b.clicked() {
                    done = true;
                }
                label(ui, p, "Help › Welcome… shows this again.");
            });
        });
        if change {
            let start = self.snap.service.settings.as_ref().map(|s| s.default_dir.clone()).unwrap_or_default();
            self.picker.open(ctx, PickFor::WelcomeDir, Ask::Folder { start });
        }
        if setup {
            self.show_integration = true;
        }
        if done || !open {
            self.prefs.welcomed = true;
            self.prefs.save();
        }
    }
}
