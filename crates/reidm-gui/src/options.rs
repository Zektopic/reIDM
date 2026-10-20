//! The Options dialog's service settings, as edited, and what *Save* sends.
//!
//! Save sends only what the user changed, compared with the settings the dialog was opened
//! with. Sending every field instead meant that a dialog opened while the settings could not be
//! loaded — its fields at their defaults — wrote those defaults over the user's folders, torrent
//! settings and limits on the first Save. Without loaded settings there is nothing to compare
//! with, so the window offers no Save at all.

use crate::model::Settings;
use serde_json::{Map, Value};

/// The ranges the service accepts (`reidm_core::Manager::update_settings` clamps to them). A
/// slider narrower than this would quietly lower a value set elsewhere on the next Save.
pub const CONNECTIONS: std::ops::RangeInclusive<usize> = 1..=32;
pub const AT_ONCE: std::ops::RangeInclusive<usize> = 1..=64;

#[derive(Clone, Debug, PartialEq)]
pub struct OptionsForm {
    pub dir: String,
    pub conn: usize,
    pub max: usize,
    pub sort: bool,
    /// Category name and its folder, as edited; empty is its default.
    pub folders: Vec<(&'static str, String)>,
    pub temp: String,
    /// Seed until this ratio; 0 not at all, below 0 forever.
    pub seed: f64,
    /// Upload limit in KB/s; 0 none.
    pub up_kb: u64,
    pub port: u16,
    pub dht: bool,
}

impl OptionsForm {
    pub fn from_settings(s: &Settings) -> OptionsForm {
        OptionsForm {
            dir: s.default_dir.clone(),
            conn: s.connections.clamp(*CONNECTIONS.start(), *CONNECTIONS.end()),
            max: s.max_concurrent.clamp(*AT_ONCE.start(), *AT_ONCE.end()),
            sort: s.folders.sort,
            // Only the folders the user chose are shown filled in; the rest follow the
            // download folder, and show it greyed as their hint.
            folders: reidm_core::category::ALL
                .iter()
                .map(|c| (c.name(), if s.folders.custom.iter().any(|n| n == c.name()) { s.folders.dirs.get(c.name()).cloned().unwrap_or_default() } else { String::new() }))
                .collect(),
            temp: if s.folders.temp_custom { s.folders.temp_dir.clone() } else { String::new() },
            seed: s.seed_ratio,
            up_kb: s.upload_limit / 1024,
            port: s.torrent_port,
            dht: s.torrent_dht,
        }
    }

    /// The settings patch for `POST /api/settings`: the fields that differ from `base`, the
    /// settings as loaded. Empty when nothing changed.
    pub fn changes(&self, base: &OptionsForm) -> Map<String, Value> {
        let mut m = Map::new();
        let mut put = |k: &str, changed: bool, v: Value| {
            if changed {
                m.insert(k.into(), v);
            }
        };
        put("default_dir", self.dir.trim() != base.dir.trim(), self.dir.trim().into());
        put("connections", self.conn != base.conn, self.conn.into());
        put("max_concurrent", self.max != base.max, self.max.into());
        put("sort_by_category", self.sort != base.sort, self.sort.into());
        // The service takes the folders as a whole ("replaces every choice"), so a change to any
        // one sends all of them.
        let trimmed = |f: &[(&str, String)]| f.iter().map(|(n, d)| (n.to_string(), d.trim().to_string())).collect::<Vec<_>>();
        let dirs: Map<String, Value> = trimmed(&self.folders).into_iter().filter(|(_, d)| !d.is_empty()).map(|(n, d)| (n, d.into())).collect();
        put("category_dirs", trimmed(&self.folders) != trimmed(&base.folders), dirs.into());
        put("temp_dir", self.temp.trim() != base.temp.trim(), self.temp.trim().into());
        put("seed_ratio", self.seed != base.seed, self.seed.into());
        put("upload_limit", self.up_kb != base.up_kb, (self.up_kb * 1024).into());
        put("torrent_port", self.port != base.port, self.port.into());
        put("torrent_dht", self.dht != base.dht, self.dht.into());
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded() -> Settings {
        serde_json::from_value(serde_json::json!({
            "default_dir": "/home/me/Downloads", "connections": 8, "max_concurrent": 32,
            "speed_limit": 0, "limit_enabled": false,
            "folders": { "sort": true, "dirs": { "Video": "/media/films", "Music": "/home/me/Downloads/Music" }, "custom": ["Video"], "temp_dir": "/fast/tmp", "temp_custom": true },
            "seed_ratio": 2.0, "upload_limit": 102400, "torrent_port": 51413, "torrent_dht": false,
        }))
        .unwrap()
    }

    #[test]
    fn saving_unchanged_options_sends_nothing() {
        let base = OptionsForm::from_settings(&loaded());
        assert!(base.clone().changes(&base).is_empty());
        assert_eq!(base.max, 32, "a value above the old slider's 16 survives loading");
    }

    #[test]
    fn only_what_changed_is_sent() {
        let base = OptionsForm::from_settings(&loaded());
        let mut f = base.clone();
        f.max = 6;
        let c = f.changes(&base);
        assert_eq!(c.len(), 1);
        assert_eq!(c["max_concurrent"], 6);

        // One folder changed: every chosen folder goes, since the service replaces them all.
        let mut f = base.clone();
        f.folders.iter_mut().find(|(n, _)| *n == "Music").unwrap().1 = "/music".into();
        let c = f.changes(&base);
        assert_eq!(c.len(), 1);
        assert_eq!(c["category_dirs"], serde_json::json!({ "Video": "/media/films", "Music": "/music" }));
    }

    #[test]
    fn ranges_match_the_service() {
        // `reidm_core::Manager::update_settings` clamps connections to 1..=32 and downloads at
        // once to 1..=64.
        assert_eq!(CONNECTIONS, 1..=32);
        assert_eq!(AT_ONCE, 1..=64);
    }
}
