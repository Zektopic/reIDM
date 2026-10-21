//! The reIDM desktop window.
//!
//! Laid out like the classic IDM main window: a menu bar, a toolbar of icon-over-label
//! buttons, a category tree down the left, the download table in the middle, and a status
//! bar. Every IDM feature the window shows is implemented; anything reIDM does not have is left
//! out rather than shown as a dead button.

mod about;
mod add;
mod banners;
mod commands;
mod complete;
mod confirm;
mod grabber;
mod intake;
mod integration;
mod lifecycle;
mod menu;
mod options_dialog;
mod props;
mod refresh;
mod scheduler;
mod sidebar;
mod state;
mod status_bar;
mod table;
mod toolbar;
mod when_done;
mod widgets;

use crate::api::{open_external, BatchResult, Client, Pace, Poller, Snapshot};
use crate::icons;
use crate::model::{eta_human, now_secs, rate_human, since_human, size_human, Category, GrabOptions, Item, Schedule, Status, View, CATEGORIES};
use crate::options::OptionsForm;
use crate::prefs::Prefs;
use crate::theme::{self, Palette};
use crate::view::{SortKey, ViewModel};
use crate::worker::{Reply, Worker};
use egui::{Align, Layout, RichText, Vec2};
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use crate::picker::{Ask, Picker};
use state::*;
use widgets::{accessible, label, segment_map, window};

/// A reply from the command thread: what to do in the window with a command's result.
fn reply(f: impl FnOnce(&mut App) + Send + 'static) -> Option<Reply<App>> {
    Some(Box::new(f))
}

/// A confirmation the window is waiting for before doing something that can't be undone.
enum Confirm {
    /// Remove these from the list, and maybe delete their files.
    Remove(Vec<u64>),
    /// Start these again from the first byte.
    Restart(Vec<u64>),
    /// Remove every finished download from the list.
    ClearCompleted(Vec<u64>),
    /// The window is closing for the first time: say that downloads carry on, in the tray or
    /// without the window.
    Close { tray: bool },
    /// Quit everything while this many downloads are running.
    Quit(usize),
}

struct Toast {
    text: String,
    until: Instant,
}

pub struct App {
    poller: Poller,
    /// Quick commands, one at a time, off the window's thread.
    cmd: Worker<App>,
    /// For calls that take long enough to need a thread of their own.
    client: Client,
    snap: Snapshot,
    /// The visible rows and the counts, rebuilt only when the list or the view changes.
    vm: ViewModel,
    view: View,
    selected: HashSet<u64>,
    /// The row last clicked: what Properties, Open and Enter act on.
    focus: Option<u64>,
    /// A row the list should scroll to, once (after moving with the keyboard).
    scroll_to: Option<u64>,
    anchor: Option<u64>,
    search: String,
    sort: SortKey,
    desc: bool,
    prefs: Prefs,
    applied_theme: Option<bool>,
    toast: Option<Toast>,

    // dialogs
    add: AddState,
    batch: BatchState,
    options: options_dialog::OptionsDialog,
    confirm: Option<Confirm>,
    about: bool,
    refresh: Option<RefreshState>,
    sched: SchedEdit,
    limiter: LimiterEdit,
    props: Option<PropsState>,
    show_integration: bool,

    ctx: egui::Context,
    // "Download complete" windows, oldest first, and what each download was last seen doing.
    completed: Vec<u64>,
    watch: crate::notify::Watch,
    /// Notifications on their way, with what to show instead if they can't be.
    notifying: Vec<(crate::notify::Message, Receiver<Result<(), String>>)>,
    /// The snapshot `note_completions` last looked at.
    noted_rev: u64,

    // the tray
    tray: Option<crate::tray::Tray>,
    /// Looking for somewhere to show a tray icon; then whether there is.
    tray_probe: Option<Receiver<bool>>,
    tray_available: Option<bool>,
    /// The icon couldn't be made: don't try every frame.
    tray_failed: bool,
    /// Hidden in the tray.
    hidden: bool,
    /// Closing for real, not into the tray.
    quitting: bool,
    /// The *When done* countdown the user cancelled, and the one last brought to the front
    /// (by when each ends).
    countdown_cancelled: Option<u64>,
    countdown_seen: Option<u64>,

    /// The file or folder picker open, if any.
    picker: Picker<PickFor>,

    // watching the clipboard
    clip_watcher: Option<intake::ClipboardWatcher>,
    clip_on: bool,
    clip_priming: bool,
    clip_seen: crate::dropped::ClipboardSeen,
    /// Copied links waiting for *Download this link?*, oldest first.
    clip_offers: std::collections::VecDeque<String>,

    // updates
    update: Option<UpdateOffer>,
    update_rx: Option<Receiver<Result<Option<reidm_core::update::Release>, String>>>,
    /// IDM's "Categories > All Downloads" node is collapsible; start open.
    cats_open: bool,
    downloads_open: bool,
    /// An update check is kicked off once, a moment after startup.
    checked_on_start: bool,

    // site grabber
    show_grabber: bool,
    playlist: crate::playlist::PlaylistWindow,
    torrent: crate::torrent::TorrentWindow,
    grab: GrabState,
}

/// The text on the clipboard, if any. Read only when the user opens a dialog that uses it.
fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, force_dark: bool) -> Self {
        let mut prefs = Prefs::load();
        if force_dark {
            prefs.dark = true;
        }
        theme::apply(&cc.egui_ctx, prefs.dark);
        let poller = Poller::start(Some(cc.egui_ctx.clone()));
        let cmd = {
            let (ctx, poke) = (cc.egui_ctx.clone(), poller.poke.clone());
            Worker::start(poller.client.clone(), move || {
                // Show the command's effect now rather than at the next poll.
                let _ = poke.send(());
                ctx.request_repaint();
            })
        };
        Self {
            client: poller.client.clone(),
            poller,
            cmd,
            snap: Snapshot::default(),
            vm: ViewModel::default(),
            view: View::Category(Category::All),
            selected: HashSet::new(),
            focus: None,
            scroll_to: None,
            anchor: None,
            search: String::new(),
            sort: SortKey::Name,
            desc: false,
            prefs,
            applied_theme: None,
            toast: None,
            add: AddState::default(),
            batch: BatchState::default(),
            options: Default::default(),
            confirm: None,
            about: false,
            refresh: None,
            sched: SchedEdit::default(),
            limiter: LimiterEdit::default(),
            props: None,
            show_integration: false,
            ctx: cc.egui_ctx.clone(),
            completed: Vec::new(),
            watch: Default::default(),
            notifying: Vec::new(),
            noted_rev: 0,
            tray: None,
            tray_probe: Some(crate::tray::probe(&cc.egui_ctx)),
            tray_available: None,
            tray_failed: false,
            hidden: false,
            quitting: false,
            countdown_cancelled: None,
            countdown_seen: None,
            picker: Picker::default(),
            clip_watcher: None,
            clip_on: false,
            clip_priming: false,
            clip_seen: Default::default(),
            clip_offers: Default::default(),
            update: None,
            update_rx: None,
            cats_open: true,
            downloads_open: true,
            checked_on_start: false,
            show_grabber: false,
            playlist: Default::default(),
            torrent: Default::default(),
            grab: GrabState::default(),
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(5) });
    }

    /// Waiting for an answer that arrives on a channel the window has to look at.
    fn waiting(&self) -> bool {
        self.update_rx.is_some()
            || self.grab.rx.is_some()
            || self.refresh.as_ref().is_some_and(|r| r.busy.is_some() || r.capturing)
            || self.props.is_some()
            || self.playlist.busy()
            || self.torrent.busy()
    }

    fn item(&self, id: u64) -> Option<&Item> {
        self.snap.item(id)
    }

    // ── selection ────────────────────────────────────────────────────────

    fn select_only(&mut self, id: u64) {
        self.selected.clear();
        self.selected.insert(id);
        self.anchor = Some(id);
        self.focus = Some(id);
    }

    fn toggle_select(&mut self, id: u64) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
            self.anchor = Some(id);
            self.focus = Some(id);
        }
    }

    fn select_range(&mut self, id: u64) {
        let Some(a) = self.anchor else { return self.select_only(id) };
        let ordered: Vec<u64> = self.vm.ids(&self.snap).collect();
        let (Some(i), Some(j)) = (ordered.iter().position(|x| *x == a), ordered.iter().position(|x| *x == id)) else {
            return self.select_only(id);
        };
        let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
        self.selected = ordered[lo..=hi].iter().copied().collect();
        self.focus = Some(id);
    }

    /// The selected downloads, in list order.
    fn selected_items(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.selected.iter().filter_map(|id| self.item(*id)).collect();
        v.sort_by_key(|i| i.id);
        v
    }

    fn selected_ids(&self) -> Vec<u64> {
        self.selected_items().iter().map(|i| i.id).collect()
    }

    /// Esc closes the dialog on top. The confirmation and the countdown handle their own.
    fn escape(&mut self) {
        if self.confirm.is_some() || self.snap.service.when_done.is_some_and(|w| w.fires_at.is_some_and(|at| self.countdown_cancelled != Some(at))) {
            return;
        }
        if self.batch.open {
            self.batch.open = false;
        } else if self.sched.open {
            self.sched.open = false;
        } else if self.limiter.open {
            self.limiter.open = false;
        } else if self.options.open {
            self.cancel_options();
        } else if self.add.open {
            self.add.open = false;
        } else if self.props.is_some() {
            self.props = None;
        } else if self.show_integration {
            self.show_integration = false;
        } else if self.about {
            self.about = false;
        } else if self.show_grabber {
            self.show_grabber = false;
        } else {
            self.clip_offers.pop_front();
        }
    }

    /// Move the focus through the list, selecting as it goes (Shift extends the selection).
    fn navigate(&mut self, nav: crate::view::Nav, extend: bool) {
        let ids: Vec<u64> = self.vm.ids(&self.snap).collect();
        let from = self.focus.and_then(|f| ids.iter().position(|x| *x == f));
        let Some(to) = crate::view::step(ids.len(), from, nav) else { return };
        let id = ids[to];
        if extend {
            self.select_range(id);
        } else {
            self.select_only(id);
        }
        self.scroll_to = Some(id);
    }

    /// Keyboard shortcuts for the list. Ignored while a text field has the keyboard.
    ///
    /// Each key is read with the modifiers it was pressed with, not those held when the frame
    /// is drawn: a quick Ctrl+↑ can be over by then.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use crate::view::Nav;
        use egui::Key;
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.escape();
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (keys, paste) = ctx.input(|i| {
            let keys: Vec<(Key, egui::Modifiers)> = i.events.iter().filter_map(|e| if let egui::Event::Key { key, pressed: true, modifiers, .. } = e { Some((*key, *modifiers)) } else { None }).collect();
            (keys, i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))))
        });
        for (key, m) in keys {
            let plain = !m.command && !m.alt;
            let nav = match key {
                Key::ArrowUp => Some(Nav::Up),
                Key::ArrowDown => Some(Nav::Down),
                Key::PageUp => Some(Nav::PageUp),
                Key::PageDown => Some(Nav::PageDown),
                Key::Home => Some(Nav::Home),
                Key::End => Some(Nav::End),
                _ => None,
            };
            match (key, nav) {
                (Key::ArrowUp | Key::ArrowDown, _) if m.command => {
                    if self.vm.counts.ordered {
                        self.move_selected(if key == Key::ArrowUp { crate::queue::To::Up } else { crate::queue::To::Down });
                    }
                }
                (_, Some(n)) if plain => {
                    // The arrows mean the list here, not egui's moving the focus between widgets.
                    ctx.memory_mut(|mem| {
                        if let Some(f) = mem.focused() {
                            mem.surrender_focus(f);
                        }
                    });
                    self.navigate(n, m.shift);
                }
                (Key::Space, _) if plain && ctx.memory(|mem| mem.focused().is_none()) => {
                    if let Some(id) = self.focus {
                        self.toggle_select(id);
                    }
                }
                (Key::F2, _) => self.props_of_focus(),
                (Key::Enter, _) if m.alt => self.props_of_focus(),
                (Key::Enter, _) => {
                    if let Some(id) = self.focus.filter(|id| self.selected.contains(id)) {
                        if self.item(id).is_some_and(|i| i.status == Status::Done) {
                            self.open_id(id, false);
                        } else {
                            self.open_props(id);
                        }
                    }
                }
                (Key::Delete, _) if !self.selected.is_empty() => self.confirm = Some(Confirm::Remove(self.selected_ids())),
                (Key::A, _) if m.command => self.selected = self.vm.ids(&self.snap).collect(),
                (Key::N, _) if m.command => self.open_add(),
                (Key::Insert, _) => self.open_add(),
                _ => {}
            }
        }
        if paste && !self.add.open {
            self.open_add();
        }
    }

    fn props_of_focus(&mut self) {
        if let Some(id) = self.focus.filter(|id| self.selected.contains(id)) {
            self.open_props(id);
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.run_logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.refresh_props();
        if self.applied_theme != Some(self.prefs.dark) {
            theme::apply(&ctx, self.prefs.dark);
            if self.applied_theme.is_some() {
                self.prefs.save();
            }
            self.applied_theme = Some(self.prefs.dark);
        }
        let p = theme::palette_for(self.prefs.dark);
        self.shortcuts(&ctx);

        // Panel order matters: outermost first, central panel last.
        if !self.snap.service.warnings.is_empty() {
            self.warnings_banner(ui, &p);
        }
        if self.update.as_ref().is_some_and(|u| !u.version.is_empty() && !u.dismissed) {
            self.update_banner(ui, &p);
        }
        self.menu_bar(ui, &p);
        self.toolbar(ui, &p);
        self.status_bar(ui, &p);
        self.sidebar(ui, &p);
        self.table(ui, &p);

        self.add_dialog(&ctx, &p);
        self.batch_dialog(&ctx, &p);
        self.options_dialog(&ctx, &p);
        if let Some(msg) = self.playlist.show(&ctx, &self.client, &p) {
            self.toast(msg);
        }
        if let Some(msg) = self.torrent.show(&ctx, &self.client, &p) {
            self.toast(msg);
        }
        self.about_dialog(&ctx, &p);
        self.confirm_dialog(&ctx, &p);
        self.grabber_window(&ctx, &p);
        self.props_window(&ctx, &p);
        self.refresh_window(&ctx, &p);
        self.scheduler_window(&ctx, &p);
        self.limiter_window(&ctx, &p);
        self.integration_window(&ctx, &p);
        self.complete_window(&ctx, &p);
        self.countdown_window(&ctx, &p);
        self.clipboard_prompt(&ctx, &p);
        self.drops(&ctx, &p);

        // Draw again only when something will have changed. A new snapshot and a command's
        // answer each ask for a frame themselves; what is left is time passing.
        let mut next = Duration::from_secs(30); // "Last Try" says "2 min ago"
        if let Some(t) = &self.toast {
            let left = t.until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.toast = None;
            } else {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -48.0))
                    .order(egui::Order::Tooltip)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_max_width(640.0);
                            ui.label(RichText::new(&t.text).color(p.text));
                        });
                    });
                next = next.min(left);
            }
        }
        if self.waiting() {
            // An answer arriving on a channel of our own doesn't wake the window.
            next = next.min(Duration::from_millis(250));
        }
        ctx.request_repaint_after(next);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::palette_for(self.prefs.dark).bg.to_normalized_gamma_f32()
    }
}
