//! The site grabber window.

use super::*;

impl App {
    /// The site-grabber window: scan a page, pick what you want, queue it.
    pub(super) fn grabber_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.show_grabber {
            return;
        }
        let mut open = true;
        let mut scan = false;
        let mut export = false;
        let mut import = false;
        let mut queue: Option<(Vec<String>, bool)> = None;
        let found = std::mem::take(&mut self.grab.found);
        let mut selected = std::mem::take(&mut self.grab.selected);

        egui::Window::new("Site Grabber")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(760.0, 520.0))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                label(ui, p, "Page address");
                ui.horizontal(|ui| {
                    let r = ui.add_sized(Vec2::new(430.0, 26.0), egui::TextEdit::singleline(&mut self.grab.url).hint_text("https://example.com/downloads/"));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.add_enabled(!self.grab.scanning, egui::Button::new("Scan")).clicked() || enter) && !self.grab.scanning {
                        scan = true;
                    }
                    if self.grab.scanning {
                        ui.spinner();
                    }
                });
                label(ui, p, "For pages that build their links with script, or need you signed in, use the browser extension's Resource grabber instead.");

                ui.add_space(6.0);
                egui::CollapsingHeader::new("Filters").default_open(false).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        label(ui, p, "Follow links");
                        ui.add(egui::DragValue::new(&mut self.grab.depth).range(0..=3));
                        label(ui, p, if self.grab.depth == 0 { "this page only" } else { "levels deep" });
                        ui.separator();
                        ui.checkbox(&mut self.grab.same_host, "Same host only");
                        ui.checkbox(&mut self.grab.sizes, "Ask for file sizes (slower)");
                    });
                    ui.horizontal(|ui| {
                        label(ui, p, "Types");
                        ui.add_sized(Vec2::new(300.0, 22.0), egui::TextEdit::singleline(&mut self.grab.types).hint_text("zip, iso, mp4 … (blank = all common file types)"));
                    });
                    ui.horizontal(|ui| {
                        label(ui, p, "Include");
                        ui.add_sized(Vec2::new(230.0, 22.0), egui::TextEdit::singleline(&mut self.grab.include).hint_text("/pub/"));
                        label(ui, p, "Exclude");
                        ui.add_sized(Vec2::new(230.0, 22.0), egui::TextEdit::singleline(&mut self.grab.exclude).hint_text("sample"));
                    });
                });

                if let Some(e) = &self.grab.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).color(p.err).size(12.0));
                }

                ui.add_space(8.0);
                ui.separator();

                if found.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(RichText::new("Scan a page to list the files it links to.\nNothing is downloaded until you say so.").color(p.text_dim));
                    });
                } else {
                    let all_on = selected.iter().all(|s| *s);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut all_on.clone(), format!("{} files found", found.len())).clicked() {
                            for s in selected.iter_mut() {
                                *s = !all_on;
                            }
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Export list…").clicked() {
                                export = true;
                            }
                            if ui.button("Import list…").clicked() {
                                import = true;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    // Only the rows in view are laid out: a deep scan can find thousands.
                    egui::ScrollArea::vertical().max_height(280.0).show_rows(ui, 22.0, found.len(), |ui, range| {
                        for (i, f) in found.iter().enumerate().skip(range.start).take(range.len()) {
                            ui.horizontal(|ui| {
                                if i < selected.len() {
                                    ui.checkbox(&mut selected[i], "");
                                }
                                let icon_rect = egui::Rect::from_min_size(ui.cursor().min, Vec2::splat(16.0));
                                let cat = Category::of(&f.ext.to_ascii_uppercase());
                                icons::file_card(ui.painter(), icon_rect, cat, self.prefs.dark);
                                ui.add_space(20.0);
                                match f.size {
                                    Some(sz) => ui.label(RichText::new(size_human(sz)).size(12.0).color(p.text_dim)),
                                    None => ui.label(RichText::new("          ").size(12.0)),
                                };
                                ui.label(RichText::new(&f.filename).size(12.0).color(p.text))
                                    .on_hover_text(if f.found_on.is_empty() { f.url.clone() } else { format!("{}\nfound on {}", f.url, f.found_on) });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(&f.ext).size(12.0).color(p.text_dim));
                                });
                            });
                        }
                    });
                }

                ui.add_space(8.0);
                ui.separator();
                ui.horizontal(|ui| {
                    // Counted each frame, collected only when used.
                    let n = selected.iter().filter(|s| **s).count();
                    let chosen = || found.iter().zip(selected.iter()).filter(|(_, s)| **s).map(|(f, _)| f.url.clone()).collect::<Vec<String>>();
                    if ui.add_enabled(n > 0, egui::Button::new(format!("Download {n} selected"))).clicked() {
                        queue = Some((chosen(), false));
                    }
                    if ui.add_enabled(n > 0, egui::Button::new("Download later")).clicked() {
                        queue = Some((chosen(), true));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            self.show_grabber = false;
                        }
                    });
                });
            });

        if export {
            let all_urls: Vec<String> = found.iter().map(|f| f.url.clone()).collect();
            self.export_found(all_urls);
        }
        self.grab.found = found;
        self.grab.selected = selected;
        if scan {
            self.start_grab();
        }
        if let Some((urls, later)) = queue {
            self.queue_urls(urls, later);
        }
        if import {
            self.import_from_file();
        }
        if !open {
            self.show_grabber = false;
        }
    }
}
