//! The window's own preferences. Kept apart from the daemon's settings: they describe this
//! user's window, not the download service, and the service runs fine without them.

use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub dark: bool,
    /// Show IDM's "Download complete" window when a download finishes.
    #[serde(default = "yes")]
    pub notify_complete: bool,
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
}

impl Default for Prefs {
    fn default() -> Self {
        Self { dark: false, notify_complete: true, clipboard: true, close_explained: false, advanced_settings: false }
    }
}

fn path() -> std::path::PathBuf {
    reidm_core::default_data_dir().join("gui.json")
}

impl Prefs {
    pub fn load() -> Self {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
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
