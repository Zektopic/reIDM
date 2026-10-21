//! *Tasks › Add torrent…*: a magnet link, a `.torrent` address or a `.torrent` file, and the
//! files in it to download.

use crate::api::Client;
use crate::model::size_human;
use crate::theme::Palette;
use egui::{RichText, Vec2};
use reidm_core::TorrentInfo;
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub struct TorrentWindow {
    pub open: bool,
    /// A magnet link or a `.torrent` address.
    url: String,
    /// A `.torrent` file chosen from disk: its name, and its bytes base64.
    file: Option<(String, String)>,
    inspecting: Option<Receiver<Result<TorrentInfo, String>>>,
    info: Option<TorrentInfo>,
    picked: Vec<bool>,
    error: Option<String>,
    sending: Option<Receiver<Result<(), String>>>,
    picker: crate::picker::Picker<()>,
}

impl TorrentWindow {
    /// Waiting on the service, so the window must keep drawing to notice the answer.
    pub fn busy(&self) -> bool {
        self.inspecting.is_some() || self.sending.is_some() || self.picker.busy()
    }
}

impl TorrentWindow {
    pub fn open_with(&mut self, url: Option<String>) {
        *self = TorrentWindow { open: true, url: url.unwrap_or_default(), ..Default::default() };
    }

    /// Draw the window. Returns a message for the status bar once the torrent is added.
    pub fn show(&mut self, ctx: &egui::Context, api: &Client, p: &Palette) -> Option<String> {
        if !self.open {
            return None;
        }
        if let Some(rx) = &self.inspecting {
            if let Ok(r) = rx.try_recv() {
                self.inspecting = None;
                match r {
                    Ok(i) => {
                        self.picked = vec![true; i.files.len()];
                        self.info = Some(i);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        if let Some(((), path)) = self.picker.poll() {
            self.use_file(&path, api);
        }
        let mut done = None;
        if let Some(rx) = &self.sending {
            if let Ok(r) = rx.try_recv() {
                self.sending = None;
                match r {
                    Ok(()) => {
                        self.open = false;
                        done = Some(format!("{} added to reIDM", self.info.as_ref().map(|i| i.name.clone()).unwrap_or_else(|| "The torrent".into())));
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }

        let mut open = true;
        let mut send: Option<bool> = None;
        egui::Window::new("Add torrent").open(&mut open).collapsible(false).resizable(true).default_size(Vec2::new(720.0, 480.0)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let hint = "magnet:?xt=urn:btih:…  or the address of a .torrent file";
                match &self.file {
                    Some((name, _)) => {
                        ui.label(RichText::new(format!("File: {name}")).strong());
                        if ui.button("Use a link instead").clicked() {
                            self.file = None;
                            self.info = None;
                        }
                    }
                    None => {
                        ui.add(egui::TextEdit::singleline(&mut self.url).hint_text(hint).desired_width(480.0));
                        if ui.button("Browse .torrent…").clicked() {
                            self.picker.open(ctx, (), crate::picker::Ask::Open { title: "Add a torrent", filter: ("Torrent", &["torrent"]) });
                        }
                    }
                }
                let can = self.inspecting.is_none() && (self.file.is_some() || !self.url.trim().is_empty());
                if ui.add_enabled(can, egui::Button::new("Show files")).clicked() {
                    self.start_inspect(api);
                }
            });
            if self.inspecting.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if self.file.is_none() && self.url.trim_start().starts_with("magnet:") {
                        "Asking peers for the torrent's contents… (for a magnet link this can take a minute)"
                    } else {
                        "Reading the torrent…"
                    });
                });
            }
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).color(p.err));
            }
            let Some(info) = &self.info else {
                ui.add_space(6.0);
                ui.label(RichText::new("Or add it straight away — reIDM downloads every file.").color(p.text_dim).size(12.0));
                ui.horizontal(|ui| {
                    let can = self.file.is_some() || !self.url.trim().is_empty();
                    if ui.add_enabled(can && self.sending.is_none(), egui::Button::new("Download all")).clicked() {
                        send = Some(false);
                    }
                });
                return;
            };
            ui.add_space(4.0);
            let chosen: u64 = info.files.iter().zip(&self.picked).filter(|(_, on)| **on).map(|(f, _)| f.size).sum();
            ui.horizontal(|ui| {
                ui.label(RichText::new(&info.name).strong().size(15.0));
                ui.label(RichText::new(format!("{} file{} · {} of {} selected", info.files.len(), if info.files.len() == 1 { "" } else { "s" }, size_human(chosen), size_human(info.size))).color(p.text_dim));
            });
            ui.horizontal(|ui| {
                if ui.button("Select all").clicked() {
                    self.picked.iter_mut().for_each(|b| *b = true);
                }
                if ui.button("Select none").clicked() {
                    self.picked.iter_mut().for_each(|b| *b = false);
                }
            });
            ui.separator();
            // Only the files in view are laid out: a torrent can hold tens of thousands.
            egui::ScrollArea::vertical().max_height((ui.available_height() - 50.0).max(120.0)).auto_shrink([false, false]).show_rows(ui, 22.0, info.files.len(), |ui, range| {
                egui::Grid::new("torrent-pick").num_columns(2).striped(true).spacing(Vec2::new(10.0, 4.0)).min_row_height(18.0).show(ui, |ui| {
                    for (f, on) in info.files[range.clone()].iter().zip(self.picked[range].iter_mut()) {
                        ui.checkbox(on, &f.path);
                        ui.label(RichText::new(size_human(f.size)).color(p.text_dim).size(12.0));
                        ui.end_row();
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                let any = self.picked.iter().any(|b| *b);
                if ui.add_enabled(any && self.sending.is_none(), egui::Button::new(RichText::new("Download").strong())).clicked() {
                    send = Some(false);
                }
                if ui.add_enabled(any && self.sending.is_none(), egui::Button::new("Download later")).clicked() {
                    send = Some(true);
                }
                if self.sending.is_some() {
                    ui.spinner();
                }
            });
        });
        if let Some(later) = send {
            self.error = None;
            let files = self.info.as_ref().and_then(|_| (!self.picked.iter().all(|b| *b)).then(|| self.picked.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i).collect::<Vec<_>>()));
            let data = self.file.as_ref().map(|(_, d)| d.clone());
            self.sending = Some(api.add_torrent_async(self.url.trim(), data, files, later));
        }
        if !open {
            self.open = false;
        }
        done
    }

    /// Open on a `.torrent` file (dropped on the window), and list what it holds.
    pub fn open_with_file(&mut self, path: &std::path::Path, api: &Client) {
        *self = TorrentWindow { open: true, ..Default::default() };
        self.use_file(path, api);
    }

    fn use_file(&mut self, path: &std::path::Path, api: &Client) {
        match std::fs::read(path) {
            Ok(b) => {
                use base64::Engine;
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.file = Some((name, base64::engine::general_purpose::STANDARD.encode(b)));
                self.info = None;
                self.start_inspect(api);
            }
            Err(e) => self.error = Some(format!("cannot read {}: {e}", path.display())),
        }
    }

    fn start_inspect(&mut self, api: &Client) {
        self.error = None;
        let data = self.file.as_ref().map(|(_, d)| d.clone());
        let url = (!self.url.trim().is_empty() && data.is_none()).then(|| self.url.trim().to_string());
        self.inspecting = Some(api.torrent_inspect_async(url, data));
    }
}
