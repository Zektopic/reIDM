//! The window's own preferences. Kept apart from the daemon's settings: they describe this
//! user's window, not the download service, and the service runs fine without them.

use crate::notify::OnFinish;
use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub dark: bool,
    /// 0.7's "Show a window when a download completes", read once into `on_finish`.
    #[serde(default = "yes", skip_serializing)]
    notify_complete: bool,
    /// Fill the Add URL dialog with the links on the clipboard.
    #[serde(default = "yes")]
    pub clipboard: bool,
    /// The user has been told, on closing the window once, that downloads carry on in the
    /// background.
    #[serde(default)]
    pub close_explained: bool,
    /// Options shows its *Advanced* tab.
    #[serde(default)]
    pub advanced_settings: bool,
    /// Show an icon in the notification area.
    #[serde(default = "yes")]
    pub tray: bool,
    /// Closing the window hides it in the notification area.
    #[serde(default = "yes")]
    pub close_to_tray: bool,
    /// Minimising the window hides it in the notification area too.
    #[serde(default)]
    pub minimize_to_tray: bool,
    /// The user has been told, once, that reIDM is still running in the notification area.
    #[serde(default)]
    pub tray_notified: bool,
    /// What a finished download does; `None` until chosen, which means a notification where
    /// there is a tray and the *Download complete* window where there isn't.
    #[serde(default)]
    pub on_finish: Option<OnFinish>,
    /// Say so when a download fails, the same way.
    #[serde(default = "yes")]
    pub notify_failures: bool,
    /// Offer links copied to the clipboard as downloads.
    #[serde(default)]
    pub watch_clipboard: bool,
    /// The list shows each download's place in the queue.
    #[serde(default)]
    pub queue_column: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            dark: false,
            notify_complete: true,
            clipboard: true,
            close_explained: false,
            advanced_settings: false,
            tray: true,
            close_to_tray: true,
            minimize_to_tray: false,
            tray_notified: false,
            on_finish: None,
            notify_failures: true,
            watch_clipboard: false,
            queue_column: false,
        }
    }
}

fn path() -> std::path::PathBuf {
    reidm_core::default_data_dir().join("gui.json")
}

impl Prefs {
    pub fn load() -> Self {
        let p: Prefs = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        p.migrated()
    }

    /// A 0.7 file's "Don't show again" on the *Download complete* window means nothing at all
    /// when a download finishes.
    fn migrated(mut self) -> Self {
        if self.on_finish.is_none() && !self.notify_complete {
            self.on_finish = Some(OnFinish::Nothing);
        }
        self
    }

    /// What a finished download does.
    pub fn on_finish(&self, tray: bool) -> OnFinish {
        self.on_finish.unwrap_or(if tray { OnFinish::Notification } else { OnFinish::Dialog })
    }

    pub fn save(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let p = path();
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_07_file_keeps_its_meaning() {
        let p: Prefs = serde_json::from_str(r#"{"dark":true,"notify_complete":false,"clipboard":true,"close_explained":true}"#).unwrap();
        let p = p.migrated();
        assert_eq!(p.on_finish(true), OnFinish::Nothing, "\"Don't show again\" stays off");
        assert!(p.tray && p.close_to_tray && !p.minimize_to_tray && p.notify_failures, "new choices start at their defaults");
        let p: Prefs = serde_json::from_str(r#"{"notify_complete":true}"#).unwrap();
        let p = p.migrated();
        assert_eq!((p.on_finish(true), p.on_finish(false)), (OnFinish::Notification, OnFinish::Dialog), "the default follows the tray");
        assert!(!serde_json::to_string(&p).unwrap().contains("notify_complete"), "not written back");
    }
}
