//! File and folder pickers that don't stop the window.
//!
//! `rfd::FileDialog` blocks the thread that opens it; on Linux the portal's picker is a separate
//! program, so the window froze — no repaint, no progress — until it was closed. Here the
//! picker is `rfd::AsyncFileDialog`, waited for on a thread of its own; the answer comes back
//! with a request to draw. One at a time: a second *Browse…* while one is open does nothing.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

/// What to ask for.
pub enum Ask {
    Folder { start: String },
    Open { title: &'static str, filter: (&'static str, &'static [&'static str]) },
    Save { title: &'static str, name: String, filter: (&'static str, &'static [&'static str]) },
}

/// A picker that may be open, and what its answer is for.
pub struct Picker<T> {
    pending: Option<(T, Receiver<Option<PathBuf>>)>,
}

impl<T> Default for Picker<T> {
    fn default() -> Self {
        Picker { pending: None }
    }
}

impl<T> Picker<T> {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Open a picker; `purpose` comes back with the path chosen.
    pub fn open(&mut self, ctx: &egui::Context, purpose: T, ask: Ask) {
        if self.busy() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("reidm-picker".into())
            .spawn(move || {
                let d = rfd::AsyncFileDialog::new();
                let chosen = match ask {
                    Ask::Folder { start } => pollster::block_on(if start.is_empty() { d } else { d.set_directory(start) }.pick_folder()),
                    Ask::Open { title, filter } => pollster::block_on(d.set_title(title).add_filter(filter.0, filter.1).pick_file()),
                    Ask::Save { title, name, filter } => pollster::block_on(d.set_title(title).add_filter(filter.0, filter.1).set_file_name(name).save_file()),
                };
                let _ = tx.send(chosen.map(|h| h.path().to_path_buf()));
                ctx.request_repaint();
            })
            .ok();
        self.pending = Some((purpose, rx));
    }

    /// The path chosen, once the picker closes; nothing if it was cancelled.
    pub fn poll(&mut self) -> Option<(T, PathBuf)> {
        let (_, rx) = self.pending.as_ref()?;
        match rx.try_recv() {
            Ok(chosen) => {
                let (purpose, _) = self.pending.take()?;
                Some((purpose, chosen?))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                None
            }
        }
    }
}
