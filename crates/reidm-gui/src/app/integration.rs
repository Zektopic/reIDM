//! How to connect a browser.

use super::*;

impl App {
    /// How to connect a browser, with what is known about the connection right now.
    pub(super) fn integration_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.show_integration {
            return;
        }
        let mut open = true;
        let svc = self.snap.service.clone();
        let mut copy: Option<String> = None;
        egui::Window::new("Browser integration")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(560.0);
                match svc.extension.clone().filter(|e| e.seconds_ago < 180) {
                    Some(e) => {
                        ui.label(RichText::new(format!("Connected — reIDM extension {} in {}", e.version, if e.browser.is_empty() { "your browser" } else { &e.browser })).color(p.ok).strong());
                        label(ui, p, "Downloads you start in that browser are taken over by reIDM. Hold Alt while clicking a link to let the browser have it.");
                    }
                    None => {
                        ui.label(RichText::new("The reIDM extension has not connected yet").color(p.warn).strong());
                        label(ui, p, "Install it once in each browser you use (Chrome, Edge, Brave, Vivaldi, Opera, Chromium):");
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new("Install").strong());
                for (n, step) in [
                    "Open the browser's extensions page: chrome://extensions, edge://extensions or brave://extensions.",
                    "Turn on Developer mode (top right, or bottom left in Edge).",
                    "Choose Load unpacked and select the folder below.",
                ]
                .iter()
                .enumerate()
                {
                    ui.label(format!("{}. {step}", n + 1));
                }
                ui.add_space(6.0);
                match &svc.extension_dir {
                    Some(dir) => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(RichText::new(dir).monospace().color(p.text)).truncate());
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Open the folder").clicked() {
                                open_external(dir);
                            }
                            if ui.button("Copy the path").clicked() {
                                copy = Some(dir.clone());
                            }
                        });
                        label(ui, p, "reIDM keeps this folder up to date, so the extension updates itself the next time the browser starts.");
                    }
                    None => {
                        ui.label(RichText::new("This installation does not include the extension folder. Run `reidm extension` from a terminal, or download reidm-extension.zip from the release page.").color(p.warn));
                    }
                }
                ui.add_space(8.0);
                ui.label(RichText::new("What it does").strong());
                label(ui, p, "• Takes over every browser download before the browser saves anything — the browser shows nothing and keeps nothing.\n\
                              • Checks reIDM can fetch the file (with your cookies) first; if not, the browser keeps it.\n\
                              • Starts reIDM if it is not running.\n\
                              • Finds video and audio, offers each quality, and grabs every resource on a page.");
                if svc.platform == "windows" && !svc.idm_port {
                    ui.add_space(8.0);
                    label(ui, p, "Note: an installed IDM holds port 1001, which only matters for Tonec's own extension, not reIDM's.");
                }
            });
        if let Some(c) = copy {
            self.copy_text(ctx, c);
            self.toast("Path copied");
        }
        if !open {
            self.show_integration = false;
        }
    }
}
