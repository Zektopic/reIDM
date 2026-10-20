//! The status bar.

use super::*;

impl App {
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui, p: &Palette) {
        egui::Panel::bottom("status")
            .frame(egui::Frame::NONE.fill(p.panel).inner_margin(egui::Margin::symmetric(10, 5)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let c = &self.vm.counts;
                    let (active, done, failed, queued) = (c.running, c.done, c.failed, c.queued);

                    let mut parts = vec![format!("{} total", c.total)];
                    if active > 0 {
                        parts.push(format!("{active} downloading"));
                    }
                    if done > 0 {
                        parts.push(format!("{done} complete"));
                    }
                    if failed > 0 {
                        parts.push(format!("{failed} failed"));
                    }
                    if queued > 0 {
                        parts.push(format!("{queued} in queue"));
                    }
                    ui.label(RichText::new(parts.join("  ·  ")).color(p.text_dim).size(12.0));

                    // Browser capture: the one thing users most need to know is working.
                    ui.separator();
                    let ext = self.snap.service.extension.clone().filter(|e| e.seconds_ago < 180);
                    let (txt, col, tip) = match &ext {
                        Some(e) => (
                            format!("Browser: {}", if e.browser.is_empty() { "connected".to_string() } else { e.browser.clone() }),
                            p.ok,
                            format!("reIDM's extension {} checked in {}s ago. Downloads started in the browser come here.", e.version, e.seconds_ago),
                        ),
                        None => (
                            "Browser: extension not connected".to_string(),
                            p.warn,
                            "reIDM's browser extension has not been seen. Click to set it up.".to_string(),
                        ),
                    };
                    if ui.add(egui::Label::new(RichText::new(txt).color(col).size(12.0)).sense(egui::Sense::click())).on_hover_text(tip).clicked() {
                        self.show_integration = true;
                    }

                    if self.snap.connected && self.snap.service.platform == "windows" && !self.snap.service.idm_port {
                        ui.separator();
                        ui.label(RichText::new("Port 1001 busy").color(p.warn).size(12.0)).on_hover_text(
                            "Another program (usually an installed IDM) holds port 1001, so the official IDM extension \
                             cannot reach reIDM directly. reIDM's own extension is unaffected.",
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.snap.total_rate > 0.0 {
                            ui.label(RichText::new(rate_human(self.snap.total_rate)).color(p.accent).size(12.0).strong());
                            ui.label(RichText::new("Total rate:").color(p.text_dim).size(12.0));
                            ui.separator();
                        }
                        if let Some(s) = &self.snap.service.settings {
                            let txt = if s.limit_enabled && s.speed_limit > 0 { format!("Speed limit: {}", rate_human(s.speed_limit as f64)) } else { "Speed limit: off".into() };
                            let col = if s.limit_enabled { p.warn } else { p.text_dim };
                            if ui.add(egui::Label::new(RichText::new(txt).color(col).size(12.0)).sense(egui::Sense::click())).on_hover_text("Click to change").clicked() {
                                self.open_limiter();
                            }
                            ui.separator();
                        }
                        if let Some(u) = &self.update {
                            if u.checking {
                                ui.label(RichText::new("Checking for updates…").color(p.text_dim).size(11.0));
                            } else if let Some(e) = &u.error {
                                ui.label(RichText::new("Update check failed").color(p.warn).size(11.0)).on_hover_text(e);
                            }
                        }
                    });
                });
            });
    }
}
