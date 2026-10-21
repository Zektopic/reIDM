//! The Options dialog: five plain tabs, each option with a line saying what it does, and an
//! *Advanced settings* switch at the bottom left that adds a sixth tab — a grid of everything
//! else, each with its unit and a way back to the default (qBittorrent's split, from the
//! October competitor review).
//!
//! What the service doesn't report is left out (an older service has no proxy, presets or
//! timeouts), and Save sends only what changed (`crate::options`).

use super::*;
use crate::model::{Performance, ProxyKind, ProxyMode};
use crate::notify::OnFinish;
use crate::options::{AdvValue, Kind, ADVANCED};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum OptTab {
    General,
    Folders,
    Downloads,
    Network,
    Torrents,
    Advanced,
}

pub(super) struct OptionsDialog {
    pub(super) open: bool,
    tab: OptTab,
    /// The service's settings as edited.
    pub(super) form: OptionsForm,
    /// The settings Options was opened with, to send only what changed; `None` until they have
    /// loaded, and Save is not offered until then.
    pub(super) base: Option<OptionsForm>,
    /// The window's own preferences when the dialog opened, for Cancel to put back (the theme
    /// changes the moment it is ticked).
    prefs_before: Option<Prefs>,
    /// Starting at sign-in, as edited and as found; `None` where it can't be told.
    autostart: Option<bool>,
    autostart_before: Option<bool>,
}

impl Default for OptionsDialog {
    fn default() -> Self {
        OptionsDialog {
            open: false,
            tab: OptTab::General,
            form: OptionsForm::from_settings(&Default::default()),
            base: None,
            prefs_before: None,
            autostart: None,
            autostart_before: None,
        }
    }
}

/// What a tab asks for that the dialog does after drawing.
#[derive(Default)]
struct Asked {
    integration: bool,
    scheduler: bool,
    check_updates: bool,
}

/// The line under an option that says what it does.
fn caption(ui: &mut egui::Ui, p: &Palette, text: &str) {
    // In line with the text of the checkbox or radio button above it.
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, ..Default::default() }).show(ui, |ui| label(ui, p, text));
    ui.add_space(6.0);
}

fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).strong());
    ui.add_space(2.0);
}

impl App {
    /// Options opens on the settings last polled, and fetches them again: Save sends only what
    /// changed against those, and is offered only once they are known.
    pub(super) fn open_options(&mut self) {
        let base = self.known_settings().map(|s| OptionsForm::from_settings(&s));
        let o = &mut self.options;
        if let Some(b) = &base {
            o.form = b.clone();
        }
        o.base = base;
        o.prefs_before = Some(self.prefs.clone());
        o.autostart = crate::autostart::enabled();
        o.autostart_before = o.autostart;
        o.tab = OptTab::General;
        o.open = true;
        self.cmd.run(|c| {
            let s = c.settings();
            reply(move |app: &mut App| {
                let Ok(s) = s else { return };
                let fresh = OptionsForm::from_settings(&s);
                let o = &mut app.options;
                // Unless the user has started editing, show the very latest.
                if o.base.as_ref().is_none_or(|b| *b == o.form) {
                    o.form = fresh.clone();
                    o.base = Some(fresh);
                }
            })
        });
    }

    /// Where a category goes when the user hasn't chosen: under the download folder.
    pub(super) fn default_folder(&self, name: &str) -> String {
        let base = std::path::Path::new(&self.options.form.dir);
        if name == reidm_core::Category::General.name() { base.to_string_lossy().into_owned() } else { base.join(name).to_string_lossy().into_owned() }
    }

    pub(super) fn options_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.options.open {
            return;
        }
        // A tab can go away under the user: Advanced unticked, or a service without a proxy.
        let tabs: Vec<(OptTab, &str)> = [
            (OptTab::General, "General"),
            (OptTab::Folders, "Folders"),
            (OptTab::Downloads, "Downloads"),
            (OptTab::Network, "Network"),
            (OptTab::Torrents, "Torrents"),
            (OptTab::Advanced, "Advanced"),
        ]
        .into_iter()
        .filter(|(t, _)| match t {
            OptTab::Network => self.options.form.proxy.is_some(),
            OptTab::Advanced => self.prefs.advanced_settings,
            _ => true,
        })
        .collect();
        if !tabs.iter().any(|(t, _)| *t == self.options.tab) {
            self.options.tab = OptTab::General;
        }
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        let mut asked = Asked::default();
        window("Options").open(&mut open).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.horizontal(|ui| {
                for (t, name) in &tabs {
                    ui.selectable_value(&mut self.options.tab, *t, *name);
                }
            });
            ui.separator();
            // The same height on every tab, so switching doesn't make the window jump.
            egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, false]).show(ui, |ui| match self.options.tab {
                OptTab::General => self.general_tab(ui, p, &mut asked),
                OptTab::Folders => self.folders_tab(ui, p),
                OptTab::Downloads => self.downloads_tab(ui, p, &mut asked),
                OptTab::Network => self.network_tab(ui, p),
                OptTab::Torrents => self.torrents_tab(ui, p),
                OptTab::Advanced => self.advanced_tab(ui, p),
            });
            ui.separator();
            let loaded = self.options.base.is_some();
            if !loaded {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading the service's settings… they can be saved once they have loaded.").color(p.warn));
                });
            }
            let problem = self.options.form.problem();
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.prefs.advanced_settings, "Advanced settings").on_hover_text("Adds a tab with every setting, for fine-tuning");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_sized(Vec2::new(90.0, 28.0), egui::Button::new("Cancel")).clicked() {
                        cancel = true;
                    }
                    // Without the settings as they are, Save would have nothing to compare with
                    // and could only write the defaults shown over the user's own.
                    if ui.add_enabled(loaded && problem.is_none(), egui::Button::new(RichText::new("Save").strong()).min_size(Vec2::new(90.0, 28.0))).clicked() {
                        save = true;
                    }
                    if let Some(why) = problem {
                        ui.label(RichText::new(why).color(p.err));
                    }
                });
            });
        });
        if asked.integration {
            self.show_integration = true;
        }
        if asked.scheduler {
            self.open_scheduler();
        }
        if asked.check_updates {
            self.check_for_updates();
        }
        if save {
            self.save_options();
        } else if cancel || !open {
            self.cancel_options();
        }
    }

    /// Close Options without saving: the window's own preferences go back too, except the
    /// Advanced switch, which is a choice of view rather than a setting.
    pub(super) fn cancel_options(&mut self) {
        let advanced = self.prefs.advanced_settings;
        if let Some(before) = self.options.prefs_before.take() {
            self.prefs = before;
        }
        if self.prefs.advanced_settings != advanced {
            self.prefs.advanced_settings = advanced;
            self.prefs.save();
        }
        self.options.open = false;
    }

    fn save_options(&mut self) {
        // The window's own preferences save at once; the service's, only what changed.
        self.prefs.save();
        let o = &mut self.options;
        o.open = false;
        o.prefs_before = None;
        if let (Some(on), true) = (o.autostart, o.autostart != o.autostart_before) {
            if let Err(e) = crate::autostart::set(on) {
                self.toast(format!("Could not change starting at sign-in: {e}"));
            }
        }
        let patch = self.options.base.as_ref().map(|b| self.options.form.changes(b)).unwrap_or_default();
        if patch.is_empty() {
            self.toast("Options saved");
            return;
        }
        self.save_settings(serde_json::Value::Object(patch), |app, r| match r {
            Ok(()) => app.toast("Options saved"),
            Err(e) => {
                // Back to the dialog, as edited, to put it right.
                app.options.open = true;
                app.toast(format!("Could not save: {e}"));
            }
        });
    }

    /// Options › General: how reIDM starts and behaves, and the browser.
    fn general_tab(&mut self, ui: &mut egui::Ui, p: &Palette, asked: &mut Asked) {
        let o = &mut self.options;
        if let Some(on) = &mut o.autostart {
            ui.checkbox(on, "Start reIDM when you sign in");
            caption(ui, p, "Starts the download service in the background, so downloads from the browser are caught before you open this window.");
        }
        if let Some(on) = &mut o.form.keep_awake {
            ui.checkbox(on, "Keep the computer awake while downloading");
            caption(ui, p, "The computer doesn't go to sleep until downloads finish. It can still be put to sleep by hand.");
        }
        if self.tray_available == Some(true) {
            ui.checkbox(&mut self.prefs.tray, "Show reIDM's icon in the notification area");
            caption(ui, p, "Its menu adds links, pauses or resumes everything, and says what to do when the downloads are done.");
            ui.add_enabled_ui(self.prefs.tray, |ui| {
                ui.checkbox(&mut self.prefs.close_to_tray, "Closing the window keeps reIDM in the notification area");
                caption(ui, p, "Downloads carry on either way; the icon brings the window back.");
            });
        }

        section(ui, "When a download finishes");
        let tray = self.prefs.tray && self.tray_available == Some(true);
        let current = self.prefs.on_finish(tray);
        for (choice, text) in [
            (OnFinish::Notification, "Show a notification"),
            (OnFinish::Dialog, "Show the Download complete window"),
            (OnFinish::Nothing, "Nothing"),
        ] {
            if ui.radio(current == choice, text).clicked() {
                self.prefs.on_finish = Some(choice);
            }
        }
        caption(ui, p, match current {
            OnFinish::Notification => "A notification from the desktop. Where they are switched off, the Download complete window shows instead.",
            OnFinish::Dialog => "IDM's small window, with buttons to open the file or its folder.",
            OnFinish::Nothing => "The list still shows it as complete.",
        });

        section(ui, "Look");
        ui.checkbox(&mut self.prefs.dark, "Dark theme");
        caption(ui, p, "Light text on a dark background.");

        section(ui, "Links you copy");
        ui.checkbox(&mut self.prefs.clipboard, "Fill Add URL with links from the clipboard");
        caption(ui, p, "Links you copied are already in Add URL when it opens.");
        ui.checkbox(&mut self.prefs.watch_clipboard, "Watch the clipboard for download links");
        caption(ui, p, "When you copy a link to a file or a magnet link, reIDM offers to download it, once. It looks about once a second while this is on.");

        section(ui, "Browser");
        ui.horizontal(|ui| {
            match self.snap.service.extension.clone().filter(|e| e.seconds_ago < 180) {
                Some(e) => ui.label(RichText::new(format!("Connected: reIDM extension {} in {}", e.version, e.browser)).color(p.ok)),
                None => ui.label(RichText::new("reIDM's extension is not connected").color(p.warn)),
            };
            if ui.button("Set up…").clicked() {
                asked.integration = true;
            }
        });
        caption(ui, p, "The extension hands the browser's downloads to reIDM.");

        section(ui, "Updates");
        ui.horizontal(|ui| {
            if ui.button("Check now").clicked() {
                asked.check_updates = true;
            }
        });
        caption(ui, p, "Asks GitHub for the newest release; nothing is downloaded or installed by itself.");

        section(ui, "Language");
        label(ui, p, "English. reIDM isn't translated yet.");
    }

    /// Options › Folders: where downloads go, and where each type of file goes.
    fn folders_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
        section(ui, "Download folder");
        ui.horizontal(|ui| {
            ui.add_sized(Vec2::new(440.0, 24.0), egui::TextEdit::singleline(&mut self.options.form.dir));
            if ui.button("Browse…").clicked() {
                self.picker.open(&self.ctx, PickFor::DownloadDir, Ask::Folder { start: self.options.form.dir.clone() });
            }
        });
        caption(ui, p, "Where finished downloads go, unless one is given a folder of its own.");

        ui.checkbox(&mut self.options.form.sort, "Save each download in the folder for its type");
        caption(ui, p, "As IDM does: videos in Video, archives in Compressed, installers in Programs… A folder left empty is the one under the download folder shown in grey.");
        let defaults: Vec<String> = self.options.form.folders.iter().map(|(n, _)| self.default_folder(n)).collect();
        let sort = self.options.form.sort;
        let mut pick: Option<(&'static str, String)> = None;
        egui::Grid::new("folders").num_columns(4).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
            for ((name, dir), default) in self.options.form.folders.iter_mut().zip(&defaults) {
                let name: &'static str = name;
                let on = sort || name == "General";
                ui.add_enabled_ui(on, |ui| ui.label(if name == "General" { "Everything else" } else { name }));
                ui.add_enabled_ui(on, |ui| ui.add_sized(Vec2::new(330.0, 22.0), egui::TextEdit::singleline(dir).hint_text(default.as_str())));
                if ui.add_enabled(on, egui::Button::new("Browse…")).clicked() {
                    let start = if dir.is_empty() { default.clone() } else { dir.clone() };
                    pick = Some((name, start));
                }
                if ui.add_enabled(!dir.is_empty(), egui::Button::new("Default")).on_hover_text("Back to the folder under the download folder").clicked() {
                    dir.clear();
                }
                ui.end_row();
            }
        });
        if let Some((name, start)) = pick {
            self.picker.open(&self.ctx, PickFor::CategoryDir(name), Ask::Folder { start });
        }
    }

    /// Options › Downloads: how hard reIDM works, and how fast.
    fn downloads_tab(&mut self, ui: &mut egui::Ui, p: &Palette, asked: &mut Asked) {
        let f = &mut self.options.form;
        if let Some(cur) = f.performance() {
            section(ui, "Performance");
            ui.horizontal(|ui| {
                for preset in Performance::PRESETS {
                    if ui.radio(cur == preset, preset.label()).clicked() {
                        f.set_performance(preset);
                    }
                }
                if cur == Performance::Custom {
                    ui.add_enabled(false, egui::RadioButton::new(true, Performance::Custom.label()));
                }
            });
            caption(ui, p, cur.caption());
        }

        section(ui, "Downloads at the same time");
        ui.add(egui::Slider::new(&mut f.max, crate::options::AT_ONCE));
        caption(ui, p, "The rest wait their turn. Torrents that are only sharing don't count.");

        section(ui, "Speed");
        ui.horizontal(|ui| {
            ui.checkbox(&mut f.limit_on, "Limit the total download speed to");
            ui.add_enabled(f.limit_on, egui::DragValue::new(&mut f.limit_kb).range(1..=10_000_000).suffix(" KB/s"));
        });
        caption(ui, p, "Shared by every download, and applied at once. One download can have a lower limit of its own in its Properties.");

        section(ui, "Schedule");
        if ui.button("Scheduler…").clicked() {
            asked.scheduler = true;
        }
        caption(ui, p, "Starts and stops the scheduled queue (Download later) at set times, such as overnight.");

        section(ui, "Asking before each download");
        label(ui, p, "Whether the browser asks before handing a download to reIDM (IDM's Download File Info window) is set in the reIDM extension's options.");
    }

    /// Options › Network: the proxy.
    fn network_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let Some(px) = &mut self.options.form.proxy else { return };
        section(ui, "Proxy");
        ui.radio_value(&mut px.mode, ProxyMode::System, "Use the computer's proxy settings");
        caption(ui, p, "Whatever the system is set to; on Linux, the http_proxy, https_proxy and no_proxy variables.");
        ui.radio_value(&mut px.mode, ProxyMode::None, "No proxy");
        caption(ui, p, "Connect to every server directly.");
        ui.radio_value(&mut px.mode, ProxyMode::Manual, "Use this proxy:");
        ui.add_enabled_ui(px.mode == ProxyMode::Manual, |ui| {
            ui.indent("proxy", |ui| {
                egui::Grid::new("proxy-grid").num_columns(2).spacing(Vec2::new(10.0, 6.0)).show(ui, |ui| {
                    ui.label("Type");
                    let name = |k: ProxyKind| match k {
                        ProxyKind::Http => "HTTP",
                        ProxyKind::Https => "HTTPS",
                        ProxyKind::Socks5 => "SOCKS5",
                    };
                    egui::ComboBox::from_id_salt("proxy-kind").selected_text(name(px.kind)).show_ui(ui, |ui| {
                        for k in [ProxyKind::Http, ProxyKind::Https, ProxyKind::Socks5] {
                            ui.selectable_value(&mut px.kind, k, name(k));
                        }
                    });
                    ui.end_row();
                    ui.label("Address");
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut px.host).hint_text("proxy.example.com").desired_width(300.0));
                        if r.lost_focus() {
                            px.tidy_host();
                        }
                        ui.label("Port");
                        ui.add(egui::DragValue::new(&mut px.port).range(0..=65535));
                    });
                    ui.end_row();
                    ui.label("User name");
                    ui.add(egui::TextEdit::singleline(&mut px.username).hint_text("if the proxy asks for one").desired_width(300.0));
                    ui.end_row();
                    ui.label("Password");
                    ui.horizontal(|ui| {
                        let hint = if px.password_set { "saved; type to replace it" } else { "if the proxy asks for one" };
                        ui.add(egui::TextEdit::singleline(&mut px.password).password(true).hint_text(hint).desired_width(300.0));
                        if px.password_set && ui.button("Forget it").clicked() {
                            px.password_set = false;
                            px.password.clear();
                        }
                    });
                    ui.end_row();
                    ui.label("Not for");
                    ui.add(egui::TextEdit::multiline(&mut px.bypass).hint_text("*.lan\n192.168.0.0/16").desired_rows(2).desired_width(300.0));
                    ui.end_row();
                });
                label(ui, p, "Servers to reach directly, one per line. The password is kept by the service and never shown again.");
                if px.kind != ProxyKind::Socks5 {
                    label(ui, p, "Torrents can only use a SOCKS5 proxy; with this one they connect directly.");
                }
            });
        });
    }

    /// Options › Torrents: how long to share, and how fast.
    fn torrents_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let f = &mut self.options.form;
        section(ui, "When a torrent is complete");
        let seed_label = |r: f64| if r == 0.0 { "Stop: don't share it".to_string() } else if r < 0.0 { "Share it for as long as reIDM runs".into() } else { format!("Share it until it has sent {r}× its size") };
        egui::ComboBox::from_id_salt("seed").width(330.0).selected_text(seed_label(f.seed)).show_ui(ui, |ui| {
            for r in [0.0, 0.5, 1.0, 2.0, 5.0, -1.0] {
                ui.selectable_value(&mut f.seed, r, seed_label(r));
            }
        });
        caption(ui, p, "Sharing (seeding) what you downloaded with others is how torrents work. Stop or restart it for one torrent from its right-click menu.");
        section(ui, "Upload limit");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut f.up_kb).range(0..=10_000_000).suffix(" KB/s"));
            label(ui, p, if f.up_kb == 0 { "(none)" } else { "" });
        });
        caption(ui, p, "How fast torrents may send. Their download speed follows the speed limit in Downloads.");
        if !self.prefs.advanced_settings {
            label(ui, p, "The listening port and DHT are under Advanced settings.");
        }
    }

    /// Options › Advanced: every other setting, as name | value | unit | default.
    fn advanced_tab(&mut self, ui: &mut egui::Ui, p: &Palette) {
        label(ui, p, "For fine-tuning. The defaults suit most people, and Default puts one back. Settings that say “requires restart” apply the next time reIDM starts.");
        ui.add_space(6.0);
        let f = &mut self.options.form;
        egui::Grid::new("advanced").num_columns(4).striped(true).spacing(Vec2::new(10.0, 4.0)).show(ui, |ui| {
            for a in ADVANCED {
                let Some(v) = a.get(f) else { continue };
                let on = a.applies(f);
                // A fixed width, so the grid is the same on every screen; the rest of a long
                // name is in the tooltip with the explanation.
                ui.allocate_ui_with_layout(Vec2::new(330.0, 20.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_width(330.0);
                    ui.add_enabled(on, egui::Label::new(a.label).truncate()).on_hover_text(a.hint);
                });
                let changed = match (v, a.kind) {
                    (AdvValue::Num(mut n), Kind::Num { min, max, .. }) => {
                        let r = ui.add_enabled(on, egui::DragValue::new(&mut n).range(min..=max));
                        r.changed().then_some(AdvValue::Num(n))
                    }
                    (AdvValue::Flag(mut b), _) => {
                        let r = ui.add_enabled(on, egui::Checkbox::without_text(&mut b));
                        r.changed().then_some(AdvValue::Flag(b))
                    }
                    (AdvValue::Num(_), Kind::Flag { .. }) => None,
                };
                if let Some(v) = changed {
                    a.set(f, v);
                }
                ui.label(RichText::new(a.unit).color(p.text_dim));
                let default = match a.default() {
                    AdvValue::Num(n) => n.to_string(),
                    AdvValue::Flag(b) => if b { "on" } else { "off" }.to_string(),
                };
                // Ids by setting rather than by position: rows come and go with the service.
                let reset = ui.push_id(a.key, |ui| ui.add_enabled(on && !a.is_default(f), egui::Button::new("Default").small()).on_hover_text(format!("Back to {default}")));
                if reset.inner.clicked() {
                    a.set(f, a.default());
                }
                ui.end_row();
            }
        });
        section(ui, "This window");
        if self.tray_available == Some(true) {
            ui.add_enabled_ui(self.prefs.tray, |ui| {
                ui.checkbox(&mut self.prefs.minimize_to_tray, "Minimising the window hides it in the notification area too");
            });
        }
        ui.checkbox(&mut self.prefs.notify_failures, "Say so when a download fails, as when one finishes");
        section(ui, "Temporary folder");
        let mut pick_temp = None;
        let temp_default = reidm_core::default_data_dir().join("parts").to_string_lossy().into_owned();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut f.temp).hint_text(temp_default.as_str()).desired_width(400.0));
            if ui.button("Browse…").clicked() {
                let start = if f.temp.is_empty() { temp_default.clone() } else { f.temp.clone() };
                pick_temp = Some(start);
            }
            if ui.add_enabled(!f.temp.is_empty(), egui::Button::new("Default").small()).on_hover_text("Back to the folder in reIDM's data folder").clicked() {
                f.temp.clear();
            }
        });
        label(ui, p, "Unfinished downloads wait here and move to their folder when complete, so the download folder only ever holds finished files. On another drive, the move is a copy.");
        if let Some(start) = pick_temp {
            self.picker.open(&self.ctx, PickFor::TempDir, Ask::Folder { start });
        }
    }
}
