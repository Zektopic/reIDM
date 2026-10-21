//! Links that arrive other than through Add URL: dropped on the window, or copied while
//! *Watch the clipboard* is on.

use super::*;
use crate::dropped::{classify, Dropped};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// Reads the clipboard about once a second on a thread of its own, only while watching is on,
/// and tells the window only when the text changed — so an unchanged clipboard costs the window
/// nothing, and a switched-off watcher costs nothing at all.
pub(super) struct ClipboardWatcher {
    on: Arc<AtomicBool>,
    wake: Sender<()>,
    texts: Receiver<Option<String>>,
}

impl ClipboardWatcher {
    fn start(ctx: egui::Context) -> ClipboardWatcher {
        let on = Arc::new(AtomicBool::new(false));
        let (wake, woken) = std::sync::mpsc::channel::<()>();
        let (tx, texts) = std::sync::mpsc::channel();
        let flag = on.clone();
        std::thread::Builder::new()
            .name("reidm-clipboard".into())
            .spawn(move || {
                let mut clip: Option<arboard::Clipboard> = None;
                let mut last: Option<Option<String>> = None;
                loop {
                    if !flag.load(Ordering::Relaxed) {
                        // Off: sleep until switched on (or the window goes).
                        last = None;
                        if woken.recv().is_err() {
                            return;
                        }
                        continue;
                    }
                    if clip.is_none() {
                        clip = arboard::Clipboard::new().ok();
                    }
                    let text = clip.as_mut().and_then(|c| c.get_text().ok());
                    if last.as_ref() != Some(&text) {
                        last = Some(text.clone());
                        if tx.send(text).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                    if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) = woken.recv_timeout(Duration::from_secs(1)) {
                        return;
                    }
                }
            })
            .ok();
        ClipboardWatcher { on, wake, texts }
    }

    fn set(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
        let _ = self.wake.send(());
    }
}

impl App {
    /// Put text on the clipboard, and remember it was the window's own, so watching the
    /// clipboard doesn't offer it back.
    pub(super) fn copy_text(&mut self, ctx: &egui::Context, text: String) {
        self.clip_seen.copied_here(&text);
        ctx.copy_text(text);
    }

    /// Follow Options › General's *Watch the clipboard*, and take in what the watcher saw.
    pub(super) fn clipboard_logic(&mut self, ctx: &egui::Context) {
        let want = self.prefs.watch_clipboard;
        if want != self.clip_on {
            self.clip_on = want;
            let w = self.clip_watcher.get_or_insert_with(|| ClipboardWatcher::start(ctx.clone()));
            w.set(want);
            // The first text after switching on is what was there already: not news.
            self.clip_priming = want;
        }
        let Some(w) = &self.clip_watcher else { return };
        let texts: Vec<Option<String>> = w.texts.try_iter().collect();
        for text in texts {
            if self.clip_priming {
                self.clip_priming = false;
                self.clip_seen.prime(text);
                continue;
            }
            let Some(link) = text.as_deref().and_then(|t| self.clip_seen.offer(t)) else { continue };
            if self.hidden {
                // Shown, but not taking the keyboard from what the user is doing.
                self.hidden = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            }
            self.clip_offers.push_back(link);
        }
    }

    /// *Download this link?* for the oldest link copied, with Download, Later and Dismiss.
    pub(super) fn clipboard_prompt(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(link) = self.clip_offers.front().cloned() else { return };
        let mut answer: Option<&str> = None;
        egui::Area::new(egui::Id::new("clipboard-offer"))
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(RichText::new("Download this link?").strong());
                    ui.add(egui::Label::new(RichText::new(&link).color(p.text_dim)).truncate()).on_hover_text(&link);
                    if self.clip_offers.len() > 1 {
                        label(ui, p, &format!("and {} more copied", self.clip_offers.len() - 1));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Download").strong()).clicked() {
                            answer = Some("now");
                        }
                        if ui.button("Later").on_hover_text("Into the scheduled queue").clicked() {
                            answer = Some("later");
                        }
                        if ui.button("Dismiss").clicked() {
                            answer = Some("dismiss");
                        }
                    });
                });
            });
        let Some(answer) = answer else { return };
        self.clip_offers.pop_front();
        if answer == "dismiss" {
            return;
        }
        if link.to_ascii_lowercase().starts_with("magnet:") {
            // A torrent's files are chosen in Add torrent.
            self.torrent.open_with(Some(link));
            return;
        }
        let later = answer == "later";
        self.cmd.run(move |c| {
            let r = c.post("/api/downloads", serde_json::json!({ "url": link, "later": later }));
            reply(move |app: &mut App| match r {
                Ok(_) => app.toast(if later { "Added to the scheduled queue" } else { "Download started" }),
                Err(e) => app.toast(format!("Could not add it: {e}")),
            })
        });
    }

    /// Files dropped on the window, and a hint while they are dragged over it.
    pub(super) fn drops(&mut self, ctx: &egui::Context, p: &Palette) {
        let (hovering, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>()));
        if hovering {
            let rect = ctx.content_rect();
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop-hint"));
            let painter = ctx.layer_painter(layer);
            painter.rect_filled(rect, 0.0, p.bg.gamma_multiply(0.85));
            painter.rect_stroke(rect.shrink(12.0), 8.0, egui::Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Drop links, .torrent files or download lists to add them", egui::FontId::proportional(18.0), p.text);
        }
        if dropped.is_empty() {
            return;
        }
        let mut torrents = 0;
        let mut refused = Vec::new();
        for d in classify(&dropped, |path| {
            // Shortcut files are a few hundred bytes; anything big isn't one.
            let meta = std::fs::metadata(path).ok()?;
            (meta.len() < 64 * 1024).then(|| std::fs::read_to_string(path).ok()).flatten()
        }) {
            match d {
                Dropped::Urls(urls) => {
                    let mut text = self.add.urls.trim_end().to_string();
                    for u in urls {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&u);
                    }
                    self.add.urls = text;
                    self.open_add();
                }
                Dropped::Torrent(path) => {
                    torrents += 1;
                    if torrents == 1 {
                        self.torrent.open_with_file(&path, &self.client);
                    }
                }
                Dropped::Import(path) => self.import_path(&path),
                Dropped::Unsupported(name) => refused.push(name),
            }
        }
        if torrents > 1 {
            self.toast(format!("Opened the first of {torrents} torrents; add the others one at a time"));
        } else if !refused.is_empty() {
            self.toast(format!("reIDM can't add {}: drop links, .torrent files, or .txt and .ef2 lists", refused.join(", ")));
        }
    }
}
