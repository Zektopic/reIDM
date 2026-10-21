//! The Options dialog's service settings, as edited, and what *Save* sends.
//!
//! Save sends only what the user changed, compared with the settings the dialog was opened
//! with. Sending every field instead meant that a dialog opened while the settings could not be
//! loaded — its fields at their defaults — wrote those defaults over the user's folders, torrent
//! settings and limits on the first Save. Without loaded settings there is nothing to compare
//! with, so the window offers no Save at all.
//!
//! A setting an older service doesn't report is `None` here: the dialog hides it, and Save never
//! sends it, so a 0.8 window works with a 0.7 service and shows only what that one can do.

use crate::model::{Performance, Proxy, ProxyKind, ProxyMode, Settings};
use serde_json::{json, Map, Value};

/// The ranges the service accepts (`reidm_core::Manager::update_settings` clamps to them). A
/// slider narrower than this would quietly lower a value set elsewhere on the next Save.
pub const CONNECTIONS: std::ops::RangeInclusive<usize> = 1..=32;
pub const AT_ONCE: std::ops::RangeInclusive<usize> = 1..=64;

/// The write buffer when the service doesn't say (its default).
const BUFFER_DEFAULT: u32 = 256;

/// The three knobs a performance preset sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knobs {
    pub connections: usize,
    pub at_once: usize,
    pub buffer_kib: u32,
}

impl Performance {
    pub const PRESETS: [Performance; 3] = [Performance::Light, Performance::Balanced, Performance::Maximum];

    pub fn label(self) -> &'static str {
        match self {
            Performance::Light => "Light",
            Performance::Balanced => "Balanced",
            Performance::Maximum => "Maximum",
            Performance::Custom => "Custom",
        }
    }

    pub fn caption(self) -> &'static str {
        match self {
            Performance::Light => "Few connections and two downloads at a time. For a slow computer, a shared connection, or a metered one.",
            Performance::Balanced => "Fast on most connections without crowding out the rest of what you do online.",
            Performance::Maximum => "Many connections and eight downloads at a time. For a fast line that one connection can't fill.",
            Performance::Custom => "Set by hand in Advanced settings. Pick a preset to go back to it.",
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            Performance::Light => "light",
            Performance::Balanced => "balanced",
            Performance::Maximum => "maximum",
            Performance::Custom => "custom",
        }
    }
}

/// The proxy as edited. The password is write-only: the service only says whether one is set,
/// so the field starts empty, and an empty field keeps the saved password.
#[derive(Clone, Debug, PartialEq)]
pub struct ProxyForm {
    pub mode: ProxyMode,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// A new password, typed here. Empty keeps the one saved.
    pub password: String,
    /// The service has a password saved, and the user hasn't asked to forget it.
    pub password_set: bool,
    /// One pattern per line (or comma-separated): `*.lan`, `10.0.0.0/8`.
    pub bypass: String,
    pub remote_dns: bool,
    pub torrents: bool,
    pub videos: bool,
}

impl ProxyForm {
    pub fn from_proxy(p: &Proxy) -> ProxyForm {
        ProxyForm {
            mode: p.mode,
            kind: p.kind.unwrap_or_default(),
            host: p.host.clone().unwrap_or_default(),
            port: p.port.unwrap_or(0),
            username: p.username.clone().unwrap_or_default(),
            password: String::new(),
            password_set: p.password_set,
            bypass: p.bypass.join("\n"),
            remote_dns: p.remote_dns.unwrap_or(true),
            torrents: p.torrents.unwrap_or(false),
            videos: p.videos.unwrap_or(true),
        }
    }

    /// Accept an address pasted whole: `socks5://proxy.lan:1080` sets the type, the host and
    /// the port at once.
    pub fn tidy_host(&mut self) {
        let mut h = self.host.trim().to_string();
        if let Some((scheme, rest)) = h.split_once("://") {
            match scheme.to_ascii_lowercase().as_str() {
                "http" => self.kind = ProxyKind::Http,
                "https" => self.kind = ProxyKind::Https,
                "socks5" | "socks5h" | "socks" => self.kind = ProxyKind::Socks5,
                _ => {}
            }
            h = rest.to_string();
        }
        let h = h.trim_end_matches('/').to_string();
        // `host:port`, but not a bare IPv6 address.
        let split = match h.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() && !host.contains(':') || host.ends_with(']') => port.parse::<u16>().ok().map(|p| (host.to_string(), p)),
            _ => None,
        };
        match split {
            Some((host, port)) => {
                self.host = host;
                self.port = port;
            }
            None => self.host = h,
        }
    }

    /// The bypass list as the service takes it.
    pub fn bypass_list(&self) -> Vec<String> {
        self.bypass.split(['\n', ',']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
    }

    /// What stops this proxy from being saved, in words for the user. The address is judged as
    /// Save will send it, after `tidy_host`, so a pasted `socks5://host:1080` isn't an error.
    pub fn problem(&self) -> Option<&'static str> {
        if self.mode != ProxyMode::Manual {
            return None;
        }
        let mut tidy = self.clone();
        tidy.tidy_host();
        let host = tidy.host.trim();
        if host.is_empty() {
            Some("Enter the proxy's address, or choose another proxy setting.")
        } else if !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:[]".contains(c)) {
            Some("The proxy's address is a name or an IP address only, like proxy.example.com, without a path or spaces.")
        } else if tidy.port == 0 {
            Some("Enter the proxy's port.")
        } else {
            None
        }
    }

    /// The `proxy` object for `POST /api/settings`. The whole object goes, as the service
    /// replaces it whole; the password only when one was typed, or as empty to forget it.
    fn patch(&self, base: &ProxyForm) -> Value {
        let username = self.username.trim();
        let mut v = json!({
            "mode": self.mode,
            "kind": self.kind,
            "host": self.host.trim(),
            "port": self.port,
            "username": (!username.is_empty()).then_some(username),
            "bypass": self.bypass_list(),
            "remote_dns": self.remote_dns,
            "torrents": self.torrents,
            "videos": self.videos,
        });
        if !self.password.is_empty() {
            v["password"] = self.password.clone().into();
        } else if base.password_set && !self.password_set {
            v["password"] = "".into();
        }
        v
    }
}

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
    pub limit_on: bool,
    /// The total speed limit in KB/s, kept while the limiter is off.
    pub limit_kb: u64,
    /// The performance preset, when the service has them: as loaded, then as chosen here, or
    /// worked out again when a knob it sets is changed by hand.
    pub performance: Option<Performance>,
    /// What each preset sets, from the service.
    pub profiles: Vec<(Performance, Knobs)>,
    pub buffer_kib: Option<u32>,
    pub retries: Option<u32>,
    pub retry_wait: Option<u32>,
    pub idle_timeout: Option<u32>,
    pub connect_timeout: Option<u32>,
    pub min_free_mib: Option<u64>,
    pub keep_awake: Option<bool>,
    pub keep_awake_seeding: Option<bool>,
    pub proxy: Option<ProxyForm>,
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
            limit_on: s.limit_enabled,
            // With no rate chosen yet, offer 1 MB/s rather than a limit of nothing.
            limit_kb: if s.speed_limit >= 1024 { s.speed_limit / 1024 } else { 1024 },
            profiles: Performance::PRESETS
                .into_iter()
                .filter_map(|p| {
                    let pr = s.profiles.get(p.wire())?;
                    Some((p, Knobs { connections: pr.connections, at_once: pr.max_concurrent, buffer_kib: pr.write_buffer_kib }))
                })
                .collect(),
            // Presets need the service's table too: without it there is nothing to choose from.
            performance: s.performance.filter(|_| Performance::PRESETS.iter().all(|p| s.profiles.contains_key(p.wire()))),
            buffer_kib: s.write_buffer_kib,
            retries: s.retries,
            retry_wait: s.retry_wait_secs,
            idle_timeout: s.idle_timeout_secs,
            connect_timeout: s.connect_timeout_secs,
            min_free_mib: s.min_free_space_mib,
            keep_awake: s.keep_awake,
            keep_awake_seeding: s.keep_awake_seeding,
            proxy: s.proxy.as_ref().map(ProxyForm::from_proxy),
        }
    }

    fn knobs(&self) -> Knobs {
        Knobs { connections: self.conn, at_once: self.max, buffer_kib: self.buffer_kib.unwrap_or(BUFFER_DEFAULT) }
    }

    /// The preset, when the service has them.
    pub fn performance(&self) -> Option<Performance> {
        self.performance
    }

    /// Choose a preset: its knobs too, so Advanced shows what it means.
    pub fn set_performance(&mut self, p: Performance) {
        self.performance = self.performance.map(|_| p);
        if let Some((_, k)) = self.profiles.iter().find(|(q, _)| *q == p) {
            self.conn = k.connections;
            self.max = k.at_once;
            if self.buffer_kib.is_some() {
                self.buffer_kib = Some(k.buffer_kib);
            }
        }
    }

    /// A knob a preset sets was changed by hand: the preset is whichever one the knobs now
    /// match, or `Custom`.
    pub fn knob_changed(&mut self) {
        if self.performance.is_some() {
            let k = self.knobs();
            self.performance = Some(self.profiles.iter().find(|(_, pk)| *pk == k).map(|(p, _)| *p).unwrap_or(Performance::Custom));
        }
    }

    /// What stops Save, in words for the user.
    pub fn problem(&self) -> Option<&'static str> {
        self.proxy.as_ref().and_then(ProxyForm::problem)
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
        // A preset sets its own knobs, and a knob sent with it would win over it: so a preset
        // goes alone, and changed knobs go only when they are the user's own (`custom`).
        let preset = self.performance.filter(|p| *p != Performance::Custom && self.performance != base.performance);
        let knobs = preset.is_none();
        put("performance", self.performance != base.performance, self.performance.map(Performance::wire).into());
        put("connections", knobs && self.conn != base.conn, self.conn.into());
        put("max_concurrent", knobs && self.max != base.max, self.max.into());
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
        // Both or neither: turning the limiter on must carry the rate shown, which may be the
        // suggested 1 MB/s rather than anything the service has (a limit of 0 limits nothing).
        let limit = self.limit_on != base.limit_on || self.limit_kb != base.limit_kb;
        put("limit_enabled", limit, self.limit_on.into());
        put("speed_limit", limit, (self.limit_kb * 1024).into());
        let opt = |a: Option<u64>, b: Option<u64>| a.is_some() && a != b;
        let n = |v: Option<u32>| v.map(u64::from);
        put("write_buffer_kib", knobs && opt(n(self.buffer_kib), n(base.buffer_kib)), self.buffer_kib.into());
        put("retries", opt(n(self.retries), n(base.retries)), self.retries.into());
        put("retry_wait_secs", opt(n(self.retry_wait), n(base.retry_wait)), self.retry_wait.into());
        put("idle_timeout_secs", opt(n(self.idle_timeout), n(base.idle_timeout)), self.idle_timeout.into());
        put("connect_timeout_secs", opt(n(self.connect_timeout), n(base.connect_timeout)), self.connect_timeout.into());
        put("min_free_space_mib", opt(self.min_free_mib, base.min_free_mib), self.min_free_mib.into());
        put("keep_awake", self.keep_awake.is_some() && self.keep_awake != base.keep_awake, self.keep_awake.into());
        put("keep_awake_seeding", self.keep_awake_seeding.is_some() && self.keep_awake_seeding != base.keep_awake_seeding, self.keep_awake_seeding.into());
        if let (Some(p), Some(b)) = (&self.proxy, &base.proxy) {
            put("proxy", p != b, p.patch(b));
        }
        m
    }
}

/// What one row of the *Advanced* grid holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AdvValue {
    Num(u64),
    Flag(bool),
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Num { min: u64, max: u64, default: u64 },
    Flag { default: bool },
}

/// One row of the *Advanced* grid: a name, its value, a unit and a reset to the default. A row
/// whose setting the service doesn't report isn't shown.
pub struct Advanced {
    /// The settings field, for tests and egui ids.
    pub key: &'static str,
    /// What it does, with "requires restart" when it only applies once reIDM starts again. At
    /// most about 50 characters: the grid's first column has a fixed width.
    pub label: &'static str,
    pub unit: &'static str,
    /// One sentence more, shown on hover.
    pub hint: &'static str,
    pub kind: Kind,
    get: fn(&OptionsForm) -> Option<AdvValue>,
    set: fn(&mut OptionsForm, AdvValue),
    /// Whether the row applies right now (the proxy's switches only with a manual proxy).
    applies: fn(&OptionsForm) -> bool,
}

impl Advanced {
    pub fn get(&self, f: &OptionsForm) -> Option<AdvValue> {
        (self.get)(f)
    }

    pub fn set(&self, f: &mut OptionsForm, v: AdvValue) {
        (self.set)(f, v)
    }

    pub fn applies(&self, f: &OptionsForm) -> bool {
        (self.applies)(f)
    }

    pub fn default(&self) -> AdvValue {
        match self.kind {
            Kind::Num { default, .. } => AdvValue::Num(default),
            Kind::Flag { default } => AdvValue::Flag(default),
        }
    }

    pub fn is_default(&self, f: &OptionsForm) -> bool {
        self.get(f).is_none_or(|v| v == self.default())
    }
}

fn num(v: AdvValue) -> u64 {
    match v {
        AdvValue::Num(n) => n,
        AdvValue::Flag(b) => b as u64,
    }
}

fn flag(v: AdvValue) -> bool {
    match v {
        AdvValue::Flag(b) => b,
        AdvValue::Num(n) => n != 0,
    }
}

fn always(_: &OptionsForm) -> bool {
    true
}

fn manual_proxy(f: &OptionsForm) -> bool {
    f.proxy.as_ref().is_some_and(|p| p.mode == ProxyMode::Manual)
}

/// Every row of the *Advanced* grid, in the order shown. The defaults and ranges are the
/// service's (`docs/design/0.8-phase2.md`).
pub const ADVANCED: &[Advanced] = &[
    Advanced {
        key: "connections",
        label: "Connections per download",
        unit: "",
        hint: "When a connection finishes early it takes over half of the slowest one's remaining part, so more connections rarely hurt. Part of the performance preset.",
        kind: Kind::Num { min: 1, max: 32, default: 8 },
        get: |f| Some(AdvValue::Num(f.conn as u64)),
        set: |f, v| {
            f.conn = num(v) as usize;
            f.knob_changed();
        },
        applies: always,
    },
    Advanced {
        key: "write_buffer_kib",
        label: "Memory per connection before writing to disk",
        unit: "KiB",
        hint: "Bigger means fewer, larger writes: easier on a hard disk, harder on a computer short of memory. Part of the performance preset.",
        kind: Kind::Num { min: 64, max: 4096, default: 256 },
        get: |f| f.buffer_kib.map(|v| AdvValue::Num(v.into())),
        set: |f, v| {
            f.buffer_kib = Some(num(v) as u32);
            f.knob_changed();
        },
        applies: always,
    },
    Advanced {
        key: "retries",
        label: "Retries per connection without progress",
        unit: "times (0 = forever)",
        hint: "A connection that gets data starts counting again from zero.",
        kind: Kind::Num { min: 0, max: 99, default: 5 },
        get: |f| f.retries.map(|v| AdvValue::Num(v.into())),
        set: |f, v| f.retries = Some(num(v) as u32),
        applies: always,
    },
    Advanced {
        key: "retry_wait_secs",
        label: "First wait between retries (then doubles)",
        unit: "s",
        hint: "How long to wait before trying a failed connection again. Each further wait is twice as long, up to a minute.",
        kind: Kind::Num { min: 1, max: 600, default: 3 },
        get: |f| f.retry_wait.map(|v| AdvValue::Num(v.into())),
        set: |f, v| f.retry_wait = Some(num(v) as u32),
        applies: always,
    },
    Advanced {
        key: "idle_timeout_secs",
        label: "Retry a connection that sends nothing for",
        unit: "s",
        hint: "Then it is retried, so a server that stalls doesn't hold a download up for ever.",
        kind: Kind::Num { min: 10, max: 3600, default: 60 },
        get: |f| f.idle_timeout.map(|v| AdvValue::Num(v.into())),
        set: |f, v| f.idle_timeout = Some(num(v) as u32),
        applies: always,
    },
    Advanced {
        key: "connect_timeout_secs",
        label: "Give up on a server that doesn't answer after",
        unit: "s",
        hint: "How long to wait for a server to accept a connection.",
        kind: Kind::Num { min: 5, max: 300, default: 30 },
        get: |f| f.connect_timeout.map(|v| AdvValue::Num(v.into())),
        set: |f, v| f.connect_timeout = Some(num(v) as u32),
        applies: always,
    },
    Advanced {
        key: "min_free_space_mib",
        label: "Pause downloads when the disk has less free than",
        unit: "MiB",
        hint: "It resumes by itself when there is room again, instead of failing half-way with a full disk.",
        kind: Kind::Num { min: 0, max: 1_048_576, default: 200 },
        get: |f| f.min_free_mib.map(AdvValue::Num),
        set: |f, v| f.min_free_mib = Some(num(v)),
        applies: always,
    },
    Advanced {
        key: "keep_awake_seeding",
        label: "Keep the computer awake while sharing too",
        unit: "",
        hint: "Normally only downloading keeps it awake.",
        kind: Kind::Flag { default: false },
        get: |f| f.keep_awake_seeding.map(AdvValue::Flag),
        set: |f, v| f.keep_awake_seeding = Some(flag(v)),
        applies: |f| f.keep_awake != Some(false),
    },
    Advanced {
        key: "torrent_port",
        label: "Torrent listening port (requires restart)",
        unit: "0 = any free port",
        hint: "Where other peers connect to you; reIDM asks your router to forward it.",
        kind: Kind::Num { min: 0, max: 65535, default: 6881 },
        get: |f| Some(AdvValue::Num(f.port.into())),
        set: |f, v| f.port = num(v) as u16,
        applies: always,
    },
    Advanced {
        key: "torrent_dht",
        label: "Use the DHT to find peers (requires restart)",
        unit: "",
        hint: "Not only through trackers. Most magnet links need it.",
        kind: Kind::Flag { default: true },
        get: |f| Some(AdvValue::Flag(f.dht)),
        set: |f, v| f.dht = flag(v),
        applies: always,
    },
    Advanced {
        key: "proxy.remote_dns",
        label: "Look up names through the SOCKS5 proxy",
        unit: "",
        hint: "The proxy, not this computer, turns server names into addresses (socks5h).",
        kind: Kind::Flag { default: true },
        get: |f| f.proxy.as_ref().map(|p| AdvValue::Flag(p.remote_dns)),
        set: |f, v| {
            if let Some(p) = &mut f.proxy {
                p.remote_dns = flag(v)
            }
        },
        applies: |f| manual_proxy(f) && f.proxy.as_ref().is_some_and(|p| p.kind == ProxyKind::Socks5),
    },
    Advanced {
        key: "proxy.torrents",
        label: "Torrents use the proxy (requires restart)",
        unit: "",
        hint: "Only a SOCKS5 proxy can carry torrent traffic.",
        kind: Kind::Flag { default: false },
        get: |f| f.proxy.as_ref().map(|p| AdvValue::Flag(p.torrents)),
        set: |f, v| {
            if let Some(p) = &mut f.proxy {
                p.torrents = flag(v)
            }
        },
        applies: |f| manual_proxy(f) && f.proxy.as_ref().is_some_and(|p| p.kind == ProxyKind::Socks5),
    },
    Advanced {
        key: "proxy.videos",
        label: "Videos use the proxy",
        unit: "",
        hint: "Passes the proxy to yt-dlp, which fetches videos.",
        kind: Kind::Flag { default: true },
        get: |f| f.proxy.as_ref().map(|p| AdvValue::Flag(p.videos)),
        set: |f, v| {
            if let Some(p) = &mut f.proxy {
                p.videos = flag(v)
            }
        },
        applies: manual_proxy,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded() -> Settings {
        serde_json::from_value(json!({
            "default_dir": "/home/me/Downloads", "connections": 8, "max_concurrent": 32,
            "speed_limit": 0, "limit_enabled": false,
            "folders": { "sort": true, "dirs": { "Video": "/media/films", "Music": "/home/me/Downloads/Music" }, "custom": ["Video"], "temp_dir": "/fast/tmp", "temp_custom": true },
            "seed_ratio": 2.0, "upload_limit": 102400, "torrent_port": 51413, "torrent_dht": false,
        }))
        .unwrap()
    }

    /// The same settings from a 0.8 service, with everything phase 2 adds.
    fn loaded_08() -> Settings {
        serde_json::from_value(json!({
            "default_dir": "/home/me/Downloads", "connections": 8, "max_concurrent": 4,
            "speed_limit": 2097152, "limit_enabled": false,
            "folders": { "sort": true, "dirs": {}, "custom": [], "temp_dir": "", "temp_custom": false },
            "seed_ratio": 1.0, "upload_limit": 0, "torrent_port": 6881, "torrent_dht": true,
            "performance": "balanced", "write_buffer_kib": 256, "retries": 5, "retry_wait_secs": 3,
            "idle_timeout_secs": 60, "connect_timeout_secs": 30, "keep_awake": true, "keep_awake_seeding": false,
            "min_free_space_mib": 200,
            "profiles": { "light": { "connections": 4, "max_concurrent": 2, "write_buffer_kib": 64, "torrent_peers": 40 },
                          "balanced": { "connections": 8, "max_concurrent": 4, "write_buffer_kib": 256, "torrent_peers": 100 },
                          "maximum": { "connections": 16, "max_concurrent": 8, "write_buffer_kib": 1024, "torrent_peers": 200 } },
            "proxy": { "mode": "manual", "kind": "socks5", "host": "proxy.lan", "port": 1080, "username": "me",
                       "password_set": true, "bypass": ["*.lan", "10.0.0.0/8"], "remote_dns": true, "torrents": false, "videos": true },
        }))
        .unwrap()
    }

    #[test]
    fn saving_unchanged_options_sends_nothing() {
        for s in [loaded(), loaded_08()] {
            let base = OptionsForm::from_settings(&s);
            assert!(base.clone().changes(&base).is_empty());
        }
        assert_eq!(OptionsForm::from_settings(&loaded()).max, 32, "a value above the old slider's 16 survives loading");
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
        assert_eq!(c["category_dirs"], json!({ "Video": "/media/films", "Music": "/music" }));
    }

    #[test]
    fn an_older_service_is_sent_nothing_it_does_not_know() {
        let s = loaded();
        let base = OptionsForm::from_settings(&s);
        assert_eq!(base.performance(), None, "no presets without the service's support");
        assert!(base.proxy.is_none() && base.retries.is_none() && base.keep_awake.is_none());
        let mut f = base.clone();
        f.max = 2;
        f.knob_changed();
        let c = f.changes(&base);
        assert_eq!(serde_json::Value::Object(c), json!({ "max_concurrent": 2 }), "no performance for a service without presets");
        // And no Advanced row for what it lacks.
        let shown: Vec<&str> = ADVANCED.iter().filter(|a| a.get(&base).is_some()).map(|a| a.key).collect();
        assert_eq!(shown, ["connections", "torrent_port", "torrent_dht"]);
    }

    #[test]
    fn a_preset_goes_alone_and_hand_set_knobs_make_it_custom() {
        let base = OptionsForm::from_settings(&loaded_08());
        assert_eq!(base.performance(), Some(Performance::Balanced));
        let mut f = base.clone();
        f.set_performance(Performance::Maximum);
        assert_eq!((f.conn, f.max, f.buffer_kib), (16, 8, Some(1024)), "the service's table, shown in Advanced");
        let c = f.changes(&base);
        // A knob sent with the preset would win over it: the preset sets its own.
        assert_eq!(serde_json::Value::Object(c), json!({ "performance": "maximum" }));

        // A knob changed by hand afterwards makes it custom, and goes with it.
        f.conn = 12;
        f.knob_changed();
        assert_eq!(f.performance(), Some(Performance::Custom));
        let c = f.changes(&base);
        assert_eq!((c["performance"].clone(), c["connections"].clone(), c["max_concurrent"].clone()), (json!("custom"), json!(12), json!(8)));
        // Knobs that match a preset again are that preset.
        f.conn = 16;
        f.knob_changed();
        assert_eq!(f.performance(), Some(Performance::Maximum));
        // And back where it started, nothing at all is sent.
        f.set_performance(Performance::Balanced);
        assert!(f.changes(&base).is_empty());
    }

    #[test]
    fn presets_need_the_services_table() {
        let mut s = loaded_08();
        s.profiles.clear();
        let f = OptionsForm::from_settings(&s);
        assert_eq!(f.performance(), None, "nothing to choose from without it");
        let mut g = f.clone();
        g.set_performance(Performance::Light);
        assert!(g.changes(&f).is_empty(), "and nothing is sent");
    }

    #[test]
    fn unknown_values_from_a_newer_service_still_load() {
        let mut v = serde_json::to_value(loaded_08().default_dir).unwrap();
        v = json!({ "default_dir": v, "performance": "eco", "proxy": { "mode": "pac", "kind": "socks4" } });
        let s: Settings = serde_json::from_value(v).unwrap();
        assert_eq!(s.performance, Some(Performance::Custom));
        let p = s.proxy.unwrap();
        assert_eq!((p.mode, p.kind), (ProxyMode::System, Some(ProxyKind::Http)));
    }

    #[test]
    fn turning_the_limiter_on_sends_the_rate_shown() {
        let base = OptionsForm::from_settings(&loaded());
        assert_eq!(base.limit_kb, 1024, "no rate yet: 1 MB/s is suggested");
        let mut f = base.clone();
        f.limit_on = true;
        let c = f.changes(&base);
        assert_eq!((c["limit_enabled"].clone(), c["speed_limit"].clone()), (json!(true), json!(1024 * 1024)));
        // Changing only the rate of a limiter that is off keeps it off, with the new rate.
        let base = OptionsForm::from_settings(&loaded_08());
        assert_eq!(base.limit_kb, 2048);
        let mut f = base.clone();
        f.limit_kb = 512;
        let c = f.changes(&base);
        assert_eq!((c["limit_enabled"].clone(), c["speed_limit"].clone()), (json!(false), json!(512 * 1024)));
    }

    #[test]
    fn the_proxy_password_is_write_only() {
        let base = OptionsForm::from_settings(&loaded_08());
        let p = base.proxy.clone().unwrap();
        assert!(p.password.is_empty() && p.password_set);
        assert_eq!(p.bypass, "*.lan\n10.0.0.0/8");

        // Another host: the whole proxy goes, without a password, so the saved one is kept.
        let mut f = base.clone();
        f.proxy.as_mut().unwrap().host = "other.lan".into();
        let c = f.changes(&base);
        assert_eq!(c.len(), 1);
        assert_eq!(c["proxy"]["host"], "other.lan");
        assert_eq!(c["proxy"]["bypass"], json!(["*.lan", "10.0.0.0/8"]));
        assert!(c["proxy"].get("password").is_none(), "{c:?}");

        // A typed password goes; forgetting it sends an empty one.
        let mut f = base.clone();
        f.proxy.as_mut().unwrap().password = "s3cret".into();
        assert_eq!(f.changes(&base)["proxy"]["password"], "s3cret");
        let mut f = base.clone();
        f.proxy.as_mut().unwrap().password_set = false;
        assert_eq!(f.changes(&base)["proxy"]["password"], "");
    }

    #[test]
    fn a_manual_proxy_needs_an_address() {
        let mut f = OptionsForm::from_settings(&loaded_08());
        assert_eq!(f.problem(), None);
        f.proxy.as_mut().unwrap().host = "proxy.lan/path".into();
        assert!(f.problem().is_some(), "the service refuses anything but a name or an address");
        f.proxy.as_mut().unwrap().host = "socks5://127.0.0.1:1080".into();
        assert_eq!(f.problem(), None, "a whole proxy address, as pasted, is taken apart on Save");
        let p = f.proxy.as_mut().unwrap();
        p.host.clear();
        assert!(f.problem().is_some());
        f.proxy.as_mut().unwrap().mode = ProxyMode::System;
        assert_eq!(f.problem(), None, "only a manual proxy needs one");
    }

    #[test]
    fn a_pasted_proxy_address_is_taken_apart() {
        let mut p = ProxyForm::from_proxy(&Proxy::default());
        p.host = " socks5h://proxy.lan:1080/ ".into();
        p.tidy_host();
        assert_eq!((p.kind, p.host.as_str(), p.port), (ProxyKind::Socks5, "proxy.lan", 1080));
        p.host = "[::1]:3128".into();
        p.tidy_host();
        assert_eq!((p.host.as_str(), p.port), ("[::1]", 3128));
        p.host = "fe80::1".into();
        p.port = 8080;
        p.tidy_host();
        assert_eq!((p.host.as_str(), p.port), ("fe80::1", 8080), "a bare IPv6 address keeps its colons");
    }

    #[test]
    fn advanced_rows_reset_to_the_services_defaults() {
        let mut f = OptionsForm::from_settings(&loaded_08());
        for a in ADVANCED {
            assert!(a.get(&f).is_some(), "{} is reported by a 0.8 service", a.key);
            if let Kind::Num { min, max, default } = a.kind {
                assert!(min <= default && default <= max, "{}", a.key);
            }
        }
        let retries = ADVANCED.iter().find(|a| a.key == "retries").unwrap();
        retries.set(&mut f, AdvValue::Num(0));
        assert!(!retries.is_default(&f));
        assert_eq!(f.retries, Some(0));
        retries.set(&mut f, retries.default());
        assert!(retries.is_default(&f));
        // The SOCKS5-only switches apply only to a manual SOCKS5 proxy.
        let torrents = ADVANCED.iter().find(|a| a.key == "proxy.torrents").unwrap();
        assert!(torrents.applies(&f));
        f.proxy.as_mut().unwrap().kind = ProxyKind::Http;
        assert!(!torrents.applies(&f));
    }

    #[test]
    fn advanced_labels_fit_their_column() {
        for a in ADVANCED {
            assert!(a.label.chars().count() <= 52, "{}: {} characters", a.key, a.label.chars().count());
        }
    }

    #[test]
    fn ranges_match_the_service() {
        // `reidm_core::Manager::update_settings` clamps connections to 1..=32 and downloads at
        // once to 1..=64.
        assert_eq!(CONNECTIONS, 1..=32);
        assert_eq!(AT_ONCE, 1..=64);
        let conn = ADVANCED.iter().find(|a| a.key == "connections").unwrap();
        assert!(matches!(conn.kind, Kind::Num { min: 1, max: 32, .. }));
    }
}
