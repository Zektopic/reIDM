//! The notification-area icon: reIDM keeps running there when the window is closed.
//!
//! Its menu: Show reIDM, Add URL…, Pause all, Resume all, the speed limiter, *When done ›*, and
//! Quit. The tooltip says what is downloading. A click on the icon shows the window (on macOS a
//! click opens the menu, as everywhere there).
//!
//! On Linux the icon is a StatusNotifierItem on D-Bus. Desktops without a host for those (GNOME
//! without the AppIndicator extension) show nothing at all, and a window that hid itself there
//! could not be found again, so reIDM looks for a host first and, without one, goes on as if it
//! had no tray: closing the window closes it.
//!
//! The menu's events arrive on another thread; they are passed to the window over a channel,
//! with a request to draw, so the window wakes up for them — even while hidden, when eframe still
//! runs `App::logic`. Hovering over the icon doesn't wake it.

use crate::when_done::Action;
use std::sync::mpsc::{Receiver, Sender};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Show,
    AddUrl,
    PauseAll,
    ResumeAll,
    ToggleLimiter,
    WhenDone(Action),
    Quit,
}

impl Command {
    fn id(self) -> String {
        match self {
            Command::Show => "show".into(),
            Command::AddUrl => "add".into(),
            Command::PauseAll => "pause-all".into(),
            Command::ResumeAll => "resume-all".into(),
            Command::ToggleLimiter => "limiter".into(),
            Command::WhenDone(a) => format!("when-done-{}", a.wire()),
            Command::Quit => "quit".into(),
        }
    }

    fn from_id(id: &str) -> Option<Command> {
        Some(match id {
            "show" => Command::Show,
            "add" => Command::AddUrl,
            "pause-all" => Command::PauseAll,
            "resume-all" => Command::ResumeAll,
            "limiter" => Command::ToggleLimiter,
            "quit" => Command::Quit,
            _ => Command::WhenDone(Action::ALL.into_iter().find(|a| id.strip_prefix("when-done-") == Some(a.wire()))?),
        })
    }
}

/// What the icon's tooltip and menu show. The tray is only touched when this changes: on Linux
/// each change is a D-Bus message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub tooltip: String,
    /// The speed limiter: on, and the rate; `None` until the settings are known.
    pub limiter: Option<(bool, String)>,
    /// The *When done* choice; `None` when the service can't do it.
    pub when_done: Option<Action>,
    pub can_pause: bool,
    pub can_resume: bool,
}

/// The tooltip: what reIDM is doing, at a glance.
pub fn tooltip(running: usize, rate: &str) -> String {
    match (running, rate.is_empty()) {
        (0, _) => "reIDM".into(),
        (1, true) => "reIDM — 1 download".into(),
        (n, true) => format!("reIDM — {n} downloads"),
        (1, false) => format!("reIDM — 1 download, {rate}"),
        (n, false) => format!("reIDM — {n} downloads, {rate}"),
    }
}

/// Whether this desktop can show a tray icon at all, found on a thread of its own (on Linux it
/// is a D-Bus round trip). The answer comes over the channel, with a request to draw.
pub fn probe(ctx: &egui::Context) -> Receiver<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("reidm-tray-probe".into())
        .spawn(move || {
            let _ = tx.send(host_present());
            ctx.request_repaint();
        })
        .ok();
    rx
}

#[cfg(target_os = "linux")]
fn host_present() -> bool {
    let ask = || -> zbus::Result<bool> {
        let conn = zbus::blocking::Connection::session()?;
        let watcher = zbus::blocking::Proxy::new(&conn, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher")?;
        watcher.get_property::<bool>("IsStatusNotifierHostRegistered")
    };
    ask().unwrap_or(false)
}

#[cfg(any(windows, target_os = "macos"))]
fn host_present() -> bool {
    true
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn host_present() -> bool {
    false
}

pub struct Tray {
    _icon: TrayIcon,
    limiter: CheckMenuItem,
    when_menu: Submenu,
    when: Vec<(Action, CheckMenuItem)>,
    pause: MenuItem,
    resume: MenuItem,
    commands: Receiver<Command>,
    shown: Option<State>,
}

/// The icon for the notification area: small, as it is drawn small.
fn icon() -> Result<tray_icon::Icon, String> {
    #[cfg(windows)]
    const PNG: &[u8] = include_bytes!("../../../packaging/icon/reidm-32.png");
    #[cfg(not(windows))]
    const PNG: &[u8] = include_bytes!("../../../packaging/icon/reidm-64.png");
    let (rgba, w, h) = crate::decode_png(PNG).ok_or("the tray icon image is damaged")?;
    tray_icon::Icon::from_rgba(rgba, w, h).map_err(|e| e.to_string())
}

impl Tray {
    /// Put the icon in the notification area. Call on the window's thread (macOS requires it).
    pub fn new(ctx: &egui::Context) -> Result<Tray, String> {
        let item = |c: Command, text: &str| MenuItem::with_id(c.id(), text, true, None);
        let show = item(Command::Show, "Show reIDM");
        let add = item(Command::AddUrl, "Add URL…");
        let pause = item(Command::PauseAll, "Pause all");
        let resume = item(Command::ResumeAll, "Resume all");
        let limiter = CheckMenuItem::with_id(Command::ToggleLimiter.id(), "Speed limiter", true, false, None);
        let when: Vec<(Action, CheckMenuItem)> = Action::ALL.into_iter().map(|a| (a, CheckMenuItem::with_id(Command::WhenDone(a).id(), a.label(), true, a == Action::None, None))).collect();
        let when_menu = Submenu::new("When done", false);
        for (_, c) in &when {
            when_menu.append(c).map_err(|e| e.to_string())?;
        }
        let quit = item(Command::Quit, "Quit reIDM");
        let menu = Menu::new();
        menu.append_items(&[&show, &add, &PredefinedMenuItem::separator(), &pause, &resume, &limiter, &when_menu, &PredefinedMenuItem::separator(), &quit]).map_err(|e| e.to_string())?;

        let (tx, commands) = std::sync::mpsc::channel();
        install_handlers(tx, ctx.clone());
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon()?)
            .with_tooltip("reIDM")
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Tray { _icon: icon, limiter, when_menu, when, pause, resume, commands, shown: None })
    }

    /// What was chosen in the menu since the last look.
    pub fn commands(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }

    /// Bring the tooltip and menu up to date; nothing happens when nothing changed.
    pub fn show_state(&mut self, s: &State) {
        if self.shown.as_ref() == Some(s) {
            return;
        }
        let _ = self._icon.set_tooltip(Some(&s.tooltip));
        match &s.limiter {
            Some((on, rate)) => {
                self.limiter.set_enabled(true);
                self.limiter.set_checked(*on);
                self.limiter.set_text(if rate.is_empty() { "Speed limiter".to_string() } else { format!("Speed limiter ({rate})") });
            }
            None => self.limiter.set_enabled(false),
        }
        self.when_menu.set_enabled(s.when_done.is_some());
        for (a, item) in &self.when {
            item.set_checked(s.when_done == Some(*a));
        }
        self.pause.set_enabled(s.can_pause);
        self.resume.set_enabled(s.can_resume);
        self.shown = Some(s.clone());
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // The handlers hold the window's context; they go with the icon.
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    }
}

fn install_handlers(tx: Sender<Command>, ctx: egui::Context) {
    let (tx2, ctx2) = (tx.clone(), ctx.clone());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if let Some(c) = Command::from_id(&e.id.0) {
            let _ = tx.send(c);
            ctx.request_repaint();
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        let show = match e {
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => !cfg!(target_os = "macos"),
            TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => true,
            // Hovering: nothing to do, so no reason to wake the window.
            _ => false,
        };
        if show {
            let _ = tx2.send(Command::Show);
            ctx2.request_repaint();
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_survives_its_menu_id() {
        let mut all = vec![Command::Show, Command::AddUrl, Command::PauseAll, Command::ResumeAll, Command::ToggleLimiter, Command::Quit];
        all.extend(Action::ALL.map(Command::WhenDone));
        for c in all {
            assert_eq!(Command::from_id(&c.id()), Some(c), "{}", c.id());
        }
        assert_eq!(Command::from_id("when-done-reboot"), None);
        assert_eq!(Command::from_id("something else"), None);
    }

    #[test]
    fn the_tooltip_says_what_is_going_on() {
        assert_eq!(tooltip(0, ""), "reIDM");
        assert_eq!(tooltip(1, ""), "reIDM — 1 download");
        assert_eq!(tooltip(3, "2.4 MB/s"), "reIDM — 3 downloads, 2.4 MB/s");
    }
}
