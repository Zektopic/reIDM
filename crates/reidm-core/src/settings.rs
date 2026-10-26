//! What the user can set, how a partial change is applied, and the presets and proxy that
//! several settings are made from.
//!
//! Everything added after 0.7 has a serde default, so a settings file from any earlier version
//! loads unchanged and gets the defaults for what it doesn't mention.

use crate::{category::Category, schedule, yes};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// A preset for the settings that decide how hard reIDM works the network, the disk and the
/// computer: one choice for most people instead of four numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Performance {
    /// For an old computer, or a line shared with others.
    Light,
    #[default]
    Balanced,
    /// For a fast line and a fast disk.
    Maximum,
    /// The numbers were set by hand.
    Custom,
}

/// The numbers a [`Performance`] preset stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub connections: usize,
    pub max_concurrent: usize,
    pub write_buffer_kib: usize,
    /// Peers per torrent (librqbit's `peer_limit`), from the next time the torrent session starts.
    pub torrent_peers: usize,
}

impl Performance {
    /// The preset's numbers; `None` for [`Performance::Custom`].
    pub fn profile(self) -> Option<Profile> {
        let p = |connections, max_concurrent, write_buffer_kib, torrent_peers| Profile { connections, max_concurrent, write_buffer_kib, torrent_peers };
        match self {
            Performance::Light => Some(p(4, 2, 64, 40)),
            Performance::Balanced => Some(p(8, 4, 256, 100)),
            Performance::Maximum => Some(p(16, 8, 1024, 200)),
            Performance::Custom => None,
        }
    }

    /// Write the preset's numbers into `s`, and make it the preset in force. Custom leaves the
    /// numbers as they are.
    pub fn apply(self, s: &mut Settings) {
        s.performance = self;
        if let Some(p) = self.profile() {
            s.connections = p.connections;
            s.max_concurrent = p.max_concurrent;
            s.write_buffer_kib = p.write_buffer_kib;
        }
    }

    /// Peers per torrent. Custom has no number of its own for this, so it takes Balanced's.
    pub fn torrent_peers(self) -> usize {
        self.profile().or(Performance::Balanced.profile()).map_or(100, |p| p.torrent_peers)
    }

    /// Whether `s`'s numbers are this preset's.
    fn matches(self, s: &Settings) -> bool {
        self.profile().is_some_and(|p| p.connections == s.connections && p.max_concurrent == s.max_concurrent && p.write_buffer_kib == s.write_buffer_kib)
    }
}

/// How reIDM reaches the internet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    /// What the system says (the environment's `HTTPS_PROXY` and friends, Windows' settings).
    #[default]
    System,
    /// Directly, whatever the system says.
    None,
    /// Through the proxy described in [`Proxy`].
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    #[default]
    Http,
    Https,
    Socks5,
}

/// A proxy. The password is kept in the settings file (readable by its owner only, like the
/// list) but never sent back over the API: see [`Proxy::without_password`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proxy {
    #[serde(default)]
    pub mode: ProxyMode,
    #[serde(default)]
    pub kind: ProxyKind,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// In a change: `None` keeps the password there is, an empty one removes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Hosts reached directly: names (`*.lan` and `.lan` both mean the domain and every name
    /// under it), addresses, and networks (`10.0.0.0/8`).
    #[serde(default)]
    pub bypass: Vec<String>,
    /// SOCKS5: let the proxy look up host names, so the local network's resolver never sees them.
    #[serde(default = "yes")]
    pub remote_dns: bool,
    /// Torrents through the proxy too (SOCKS5 only); from the next time the torrent session starts.
    #[serde(default)]
    pub torrents: bool,
    /// Videos through the proxy too (yt-dlp's `--proxy`).
    #[serde(default = "yes")]
    pub videos: bool,
}

impl Default for Proxy {
    fn default() -> Self {
        Proxy {
            mode: ProxyMode::System,
            kind: ProxyKind::Http,
            host: String::new(),
            port: 0,
            username: None,
            password: None,
            bypass: Vec::new(),
            remote_dns: true,
            torrents: false,
            videos: true,
        }
    }
}

/// The characters a user name or password must have encoded inside a URL.
const USERINFO: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

impl Proxy {
    /// The same, with the password replaced by whether there is one: what the API shows.
    pub fn without_password(&self) -> (Proxy, bool) {
        let set = self.password.as_deref().is_some_and(|p| !p.is_empty());
        (Proxy { password: None, ..self.clone() }, set)
    }

    fn validate(&self) -> Result<()> {
        if self.mode != ProxyMode::Manual {
            return Ok(());
        }
        let host = self.host.trim();
        if host.is_empty() {
            bail!("the proxy needs a server name or address");
        }
        if !host.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']')) {
            bail!("{host:?} is not a server name or address: give just the name (like proxy.lan), without http:// or a port");
        }
        if self.port == 0 {
            bail!("the proxy needs a port (3128 and 8080 are common for HTTP proxies, 1080 for SOCKS)");
        }
        Ok(())
    }

    /// The proxy's address with its user name and password, for a manual proxy. `socks` is the
    /// scheme a SOCKS5 proxy is given as.
    fn address(&self, socks: &str) -> Option<String> {
        if self.mode != ProxyMode::Manual {
            return None;
        }
        let scheme = match self.kind {
            ProxyKind::Http => "http",
            ProxyKind::Https => "https",
            ProxyKind::Socks5 => socks,
        };
        let enc = |s: &str| percent_encoding::utf8_percent_encode(s, USERINFO).to_string();
        let auth = match (self.username.as_deref().filter(|u| !u.is_empty()), self.password.as_deref().filter(|p| !p.is_empty())) {
            (Some(u), Some(p)) => format!("{}:{}@", enc(u), enc(p)),
            (Some(u), None) => format!("{}@", enc(u)),
            _ => String::new(),
        };
        let host = self.host.trim();
        // An IPv6 address goes in brackets in a URL.
        let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
        Some(format!("{scheme}://{auth}{host}:{}", self.port))
    }

    /// The address reIDM's own connections use.
    pub fn url(&self) -> Option<String> {
        self.address(if self.remote_dns { "socks5h" } else { "socks5" })
    }

    /// What yt-dlp is told with `--proxy`: the proxy, `""` for "connect directly", or nothing
    /// (it then follows the system, as reIDM does).
    pub fn for_videos(&self) -> Option<String> {
        if !self.videos {
            return None;
        }
        match self.mode {
            ProxyMode::System => None,
            ProxyMode::None => Some(String::new()),
            ProxyMode::Manual => self.url(),
        }
    }

    /// The SOCKS5 proxy for torrents' peer connections, when torrents are to use it. librqbit
    /// takes `socks5://` only, and resolves nothing through it (peers are addresses).
    pub fn for_torrents(&self) -> Option<String> {
        (self.torrents && self.kind == ProxyKind::Socks5).then(|| self.address("socks5")).flatten()
    }

    /// Set `b` up to connect the way this says.
    pub(crate) fn configure(&self, b: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder> {
        Ok(match self.mode {
            // reqwest reads the environment (and Windows' settings) unless told otherwise.
            ProxyMode::System => b,
            ProxyMode::None => b.no_proxy(),
            ProxyMode::Manual => {
                let url = self.url().unwrap_or_default();
                let bypass: Vec<String> = self.bypass.iter().map(|h| h.trim()).filter(|h| !h.is_empty()).map(|h| h.strip_prefix('*').unwrap_or(h).to_string()).collect();
                let p = reqwest::Proxy::all(&url).map_err(|e| anyhow::anyhow!("the proxy {}:{} can't be used: {e}", self.host, self.port))?;
                b.proxy(p.no_proxy(reqwest::NoProxy::from_string(&bypass.join(","))))
            }
        })
    }
}

pub(crate) fn default_max_concurrent() -> usize {
    4
}

fn default_seed_ratio() -> f64 {
    1.0
}

fn default_torrent_port() -> u16 {
    6881
}

fn default_write_buffer() -> usize {
    256
}

fn default_retries() -> u32 {
    5
}

fn default_retry_wait() -> u64 {
    3
}

fn default_idle_timeout() -> u64 {
    60
}

fn default_connect_timeout() -> u64 {
    30
}

fn default_min_free_space() -> u64 {
    200
}

/// User-adjustable options, persisted next to the download list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub default_dir: PathBuf,
    pub connections: usize,
    /// How many downloads may transfer at once. The rest wait as `Queued`, which is what
    /// makes adding a hundred links at once reasonable.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// The global speed cap in bytes per second, applied while `limit_enabled` is on. Kept when
    /// the limiter is switched off, so switching it back on restores the same cap.
    #[serde(default)]
    pub speed_limit: u64,
    #[serde(default)]
    pub limit_enabled: bool,
    #[serde(default)]
    pub schedule: schedule::Schedule,
    /// File each download into its category's folder.
    #[serde(default = "yes")]
    pub sort_by_category: bool,
    /// A category's folder where the user moved it; otherwise `<default_dir>/<Category>`, and
    /// the download folder itself for *General*.
    #[serde(default)]
    pub category_dirs: BTreeMap<Category, PathBuf>,
    /// Where unfinished downloads are kept until they are complete; `None` is the `parts`
    /// folder in the data folder.
    #[serde(default)]
    pub temp_dir: Option<PathBuf>,
    /// Seed a finished torrent until it has uploaded this many times its size: 0 not at all,
    /// below 0 for as long as reIDM runs.
    #[serde(default = "default_seed_ratio")]
    pub seed_ratio: f64,
    /// Torrent uploads' cap in bytes per second; 0 for none.
    #[serde(default)]
    pub upload_limit: u64,
    /// The port torrents listen on for peers; 0 for any free one.
    #[serde(default = "default_torrent_port")]
    pub torrent_port: u16,
    /// Find torrents' peers through the DHT too, not only their trackers.
    #[serde(default = "yes")]
    pub torrent_dht: bool,
    /// The preset `connections`, `max_concurrent` and `write_buffer_kib` come from.
    #[serde(default)]
    pub performance: Performance,
    /// What each connection collects before writing it to the disk, in KiB.
    #[serde(default = "default_write_buffer")]
    pub write_buffer_kib: usize,
    /// How often a connection tries again without getting anywhere; 0 for ever.
    #[serde(default = "default_retries")]
    pub retries: u32,
    /// The first wait before trying again, in seconds; it doubles each time, up to a minute.
    #[serde(default = "default_retry_wait")]
    pub retry_wait_secs: u64,
    /// Give up on a connection that has sent nothing for this long.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Give up on a server that doesn't answer within this long.
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default)]
    pub proxy: Proxy,
    /// Keep the computer from sleeping while a download transfers.
    #[serde(default = "yes")]
    pub keep_awake: bool,
    /// ...and while a torrent is only seeding.
    #[serde(default)]
    pub keep_awake_seeding: bool,
    /// Stop a download before it would leave less than this much free space, in MiB.
    #[serde(default = "default_min_free_space")]
    pub min_free_space_mib: u64,
}

impl Settings {
    /// A first run's settings.
    pub(crate) fn new(default_dir: PathBuf, connections: usize, max_concurrent: usize) -> Self {
        let mut s = Settings {
            default_dir,
            connections,
            max_concurrent,
            speed_limit: 0,
            limit_enabled: false,
            schedule: Default::default(),
            sort_by_category: true,
            category_dirs: BTreeMap::new(),
            temp_dir: None,
            seed_ratio: default_seed_ratio(),
            upload_limit: 0,
            torrent_port: default_torrent_port(),
            torrent_dht: true,
            performance: Performance::Balanced,
            write_buffer_kib: default_write_buffer(),
            retries: default_retries(),
            retry_wait_secs: default_retry_wait(),
            idle_timeout_secs: default_idle_timeout(),
            connect_timeout_secs: default_connect_timeout(),
            proxy: Proxy::default(),
            keep_awake: true,
            keep_awake_seeding: false,
            min_free_space_mib: default_min_free_space(),
        };
        s.normalize();
        s
    }

    /// Bring every number into its range, and say "custom" when the numbers are not the
    /// preset's — a file from before presets, or one edited by hand.
    pub(crate) fn normalize(&mut self) {
        self.connections = self.connections.clamp(1, 32);
        self.max_concurrent = self.max_concurrent.clamp(1, 64);
        self.write_buffer_kib = self.write_buffer_kib.clamp(64, 4096);
        self.retries = self.retries.min(99);
        self.retry_wait_secs = self.retry_wait_secs.clamp(1, 600);
        self.idle_timeout_secs = self.idle_timeout_secs.clamp(10, 3600);
        self.connect_timeout_secs = self.connect_timeout_secs.clamp(5, 300);
        if self.performance != Performance::Custom && !self.performance.matches(self) {
            self.performance = Performance::Custom;
        }
    }

    /// The folder for a category: the user's choice, else its own folder under the download
    /// folder (the download folder itself for *General*).
    pub fn folder(&self, c: Category) -> PathBuf {
        match self.category_dirs.get(&c) {
            Some(d) => d.clone(),
            None if c == Category::General => self.default_dir.clone(),
            None => self.default_dir.join(c.name()),
        }
    }

    /// Where a file with this name goes when no folder was asked for.
    pub fn folder_for(&self, file_name: &str) -> PathBuf {
        if self.sort_by_category {
            self.folder(Category::of(file_name))
        } else {
            self.default_dir.clone()
        }
    }

    /// The cap actually in force, in bytes per second; 0 means none.
    pub fn effective_limit(&self) -> u64 {
        if self.limit_enabled {
            self.speed_limit
        } else {
            0
        }
    }

    /// How long a server may take to answer.
    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_secs)
    }

    /// How long a connection may go without sending anything.
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.idle_timeout_secs)
    }

    /// How long to wait before try number `attempt` (1, 2, …): the first wait, doubling each
    /// time, up to a minute (or the first wait, if that is longer).
    pub fn retry_wait(&self, attempt: u32) -> Duration {
        let first = self.retry_wait_secs.max(1);
        let secs = first.saturating_mul(1u64 << attempt.saturating_sub(1).min(16)).min(first.max(60));
        Duration::from_secs(secs)
    }

    /// Whether a connection that has failed `attempt` times in a row should give up.
    pub fn out_of_retries(&self, attempt: u32) -> bool {
        self.retries != 0 && attempt > self.retries
    }

    /// Apply `patch`, after checking it. What the HTTP client is built from (the proxy and
    /// time limits) is compared, so the caller knows whether to build a new one.
    pub(crate) fn apply(&mut self, patch: SettingsPatch) -> Result<Applied> {
        if let Some(p) = &patch.proxy {
            p.validate()?;
        }
        if let Some(s) = &patch.schedule {
            s.validate()?;
        }
        let before = (self.proxy.clone(), self.connect_timeout_secs, self.idle_timeout_secs);
        // The preset first: numbers given in the same change are the user's own and win.
        if let Some(p) = patch.performance {
            p.apply(self);
        }
        let set = |slot: &mut usize, v: Option<usize>| {
            if let Some(v) = v {
                *slot = v;
            }
        };
        set(&mut self.connections, patch.connections);
        set(&mut self.max_concurrent, patch.max_concurrent);
        set(&mut self.write_buffer_kib, patch.write_buffer_kib);
        if let Some(d) = patch.default_dir {
            self.default_dir = d;
        }
        if let Some(v) = patch.speed_limit {
            self.speed_limit = v;
        }
        if let Some(v) = patch.limit_enabled {
            self.limit_enabled = v;
        }
        if let Some(v) = patch.schedule {
            self.schedule = v;
        }
        if let Some(v) = patch.sort_by_category {
            self.sort_by_category = v;
        }
        if let Some(v) = patch.category_dirs {
            self.category_dirs = v;
        }
        if let Some(v) = patch.temp_dir {
            self.temp_dir = (!v.as_os_str().is_empty()).then_some(v);
        }
        if let Some(v) = patch.seed_ratio.filter(|v| v.is_finite()) {
            self.seed_ratio = v;
        }
        if let Some(v) = patch.upload_limit {
            self.upload_limit = v;
        }
        if let Some(v) = patch.torrent_port {
            self.torrent_port = v;
        }
        if let Some(v) = patch.torrent_dht {
            self.torrent_dht = v;
        }
        if let Some(v) = patch.retries {
            self.retries = v;
        }
        if let Some(v) = patch.retry_wait_secs {
            self.retry_wait_secs = v;
        }
        if let Some(v) = patch.idle_timeout_secs {
            self.idle_timeout_secs = v;
        }
        if let Some(v) = patch.connect_timeout_secs {
            self.connect_timeout_secs = v;
        }
        if let Some(mut p) = patch.proxy {
            p.password = match p.password {
                None => self.proxy.password.take(),
                Some(pw) if pw.is_empty() => None,
                Some(pw) => Some(pw),
            };
            p.host = p.host.trim().to_string();
            self.proxy = p;
        }
        if let Some(v) = patch.keep_awake {
            self.keep_awake = v;
        }
        if let Some(v) = patch.keep_awake_seeding {
            self.keep_awake_seeding = v;
        }
        if let Some(v) = patch.min_free_space_mib {
            self.min_free_space_mib = v;
        }
        self.normalize();
        Ok(Applied { client: before != (self.proxy.clone(), self.connect_timeout_secs, self.idle_timeout_secs) })
    }
}

/// What applying a [`SettingsPatch`] changed that needs more than the new values.
#[derive(Debug)]
pub(crate) struct Applied {
    /// The HTTP client has to be built again (the proxy, or a time limit, changed).
    pub(crate) client: bool,
}

/// A partial update to [`Settings`]: `None` leaves a field as it is.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub default_dir: Option<PathBuf>,
    #[serde(default)]
    pub connections: Option<usize>,
    #[serde(default)]
    pub max_concurrent: Option<usize>,
    #[serde(default)]
    pub speed_limit: Option<u64>,
    #[serde(default)]
    pub limit_enabled: Option<bool>,
    #[serde(default)]
    pub schedule: Option<schedule::Schedule>,
    #[serde(default)]
    pub sort_by_category: Option<bool>,
    /// Replaces every category's folder; a category left out goes back to its default.
    #[serde(default)]
    pub category_dirs: Option<BTreeMap<Category, PathBuf>>,
    /// The temporary folder; an empty path goes back to the default.
    #[serde(default)]
    pub temp_dir: Option<PathBuf>,
    #[serde(default)]
    pub seed_ratio: Option<f64>,
    #[serde(default)]
    pub upload_limit: Option<u64>,
    /// Takes effect when the torrent session next starts, as does `torrent_dht`.
    #[serde(default)]
    pub torrent_port: Option<u16>,
    #[serde(default)]
    pub torrent_dht: Option<bool>,
    /// A preset other than `custom` sets `connections`, `max_concurrent` and
    /// `write_buffer_kib`; any of those given as well wins, and makes it `custom`.
    #[serde(default)]
    pub performance: Option<Performance>,
    #[serde(default)]
    pub write_buffer_kib: Option<usize>,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub retry_wait_secs: Option<u64>,
    #[serde(default)]
    pub idle_timeout_secs: Option<u64>,
    #[serde(default)]
    pub connect_timeout_secs: Option<u64>,
    /// The whole proxy; its `password` left out keeps the one there is.
    #[serde(default)]
    pub proxy: Option<Proxy>,
    #[serde(default)]
    pub keep_awake: Option<bool>,
    #[serde(default)]
    pub keep_awake_seeding: Option<bool>,
    #[serde(default)]
    pub min_free_space_mib: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Settings {
        Settings::new(PathBuf::from("/dl"), 8, 4)
    }

    #[test]
    fn a_preset_sets_its_numbers_and_a_number_set_by_hand_makes_it_custom() {
        let mut s = fresh();
        assert_eq!(s.performance, Performance::Balanced);
        s.apply(SettingsPatch { performance: Some(Performance::Light), ..Default::default() }).unwrap();
        assert_eq!((s.connections, s.max_concurrent, s.write_buffer_kib), (4, 2, 64));
        assert_eq!(s.performance, Performance::Light);
        s.apply(SettingsPatch { connections: Some(12), ..Default::default() }).unwrap();
        assert_eq!(s.performance, Performance::Custom);
        assert_eq!(s.connections, 12);
        // A preset and a number in one change: the number is the user's, so it wins.
        s.apply(SettingsPatch { performance: Some(Performance::Maximum), max_concurrent: Some(3), ..Default::default() }).unwrap();
        assert_eq!((s.connections, s.max_concurrent, s.write_buffer_kib, s.performance), (16, 3, 1024, Performance::Custom));
        // Setting a number to what the preset has anyway changes nothing.
        s.apply(SettingsPatch { performance: Some(Performance::Maximum), connections: Some(16), ..Default::default() }).unwrap();
        assert_eq!(s.performance, Performance::Maximum);
        // Custom chosen by hand stays custom, even on a preset's numbers.
        s.apply(SettingsPatch { performance: Some(Performance::Custom), ..Default::default() }).unwrap();
        assert_eq!(s.performance, Performance::Custom);
        assert_eq!(Performance::Custom.torrent_peers(), 100);
        assert_eq!(Performance::Light.torrent_peers(), 40);
    }

    #[test]
    fn numbers_are_kept_in_range() {
        let mut s = fresh();
        s.apply(SettingsPatch {
            write_buffer_kib: Some(1),
            retries: Some(500),
            retry_wait_secs: Some(0),
            idle_timeout_secs: Some(100_000),
            connect_timeout_secs: Some(1),
            connections: Some(99),
            ..Default::default()
        })
        .unwrap();
        assert_eq!((s.write_buffer_kib, s.retries, s.retry_wait_secs, s.idle_timeout_secs, s.connect_timeout_secs, s.connections), (64, 99, 1, 3600, 5, 32));
    }

    #[test]
    fn retries_wait_longer_each_time_up_to_a_minute() {
        let s = fresh();
        let waits: Vec<u64> = (1..=6).map(|a| s.retry_wait(a).as_secs()).collect();
        assert_eq!(waits, [3, 6, 12, 24, 48, 60]);
        assert!(!s.out_of_retries(5) && s.out_of_retries(6));
        let forever = Settings { retries: 0, ..fresh() };
        assert!(!forever.out_of_retries(10_000));
        assert_eq!(forever.retry_wait(u32::MAX).as_secs(), 60, "no overflow on a long outage");
        let slow = Settings { retry_wait_secs: 300, ..fresh() };
        assert_eq!(slow.retry_wait(3).as_secs(), 300, "a first wait over a minute is not cut");
    }

    #[test]
    fn the_password_is_kept_unless_changed_and_never_shown() {
        let mut s = fresh();
        let manual = Proxy { mode: ProxyMode::Manual, kind: ProxyKind::Socks5, host: " proxy.lan ".into(), port: 1080, username: Some("me".into()), password: Some("p@ss:w/rd".into()), ..Default::default() };
        assert!(s.apply(SettingsPatch { proxy: Some(manual.clone()), ..Default::default() }).unwrap().client, "a new proxy needs a new client");
        assert_eq!(s.proxy.url().unwrap(), "socks5h://me:p%40ss%3Aw%2Frd@proxy.lan:1080");
        // The window sends the proxy back as it got it: without the password.
        let (shown, set) = s.proxy.without_password();
        assert!(set && shown.password.is_none());
        assert!(!s.apply(SettingsPatch { proxy: Some(shown.clone()), ..Default::default() }).unwrap().client, "nothing changed");
        assert_eq!(s.proxy.password.as_deref(), Some("p@ss:w/rd"));
        s.apply(SettingsPatch { proxy: Some(Proxy { password: Some(String::new()), ..shown }), ..Default::default() }).unwrap();
        assert_eq!(s.proxy.password, None);
        assert_eq!(s.proxy.url().unwrap(), "socks5h://me@proxy.lan:1080");
    }

    #[test]
    fn a_proxy_that_cannot_work_is_refused_with_a_reason() {
        let mut s = fresh();
        for (host, port, says) in [("", 3128, "server name"), ("http://proxy.lan", 3128, "without http://"), ("proxy.lan", 0, "port")] {
            let p = Proxy { mode: ProxyMode::Manual, host: host.into(), port, ..Default::default() };
            let e = s.apply(SettingsPatch { proxy: Some(p), ..Default::default() }).unwrap_err().to_string();
            assert!(e.contains(says), "{e}");
        }
        assert_eq!(s.proxy, Proxy::default(), "nothing was applied");
    }

    #[test]
    fn each_kind_of_proxy_becomes_the_right_address_for_each_user() {
        let p = Proxy { mode: ProxyMode::Manual, kind: ProxyKind::Http, host: "10.0.0.1".into(), port: 3128, ..Default::default() };
        assert_eq!(p.url().unwrap(), "http://10.0.0.1:3128");
        assert_eq!(p.for_videos().unwrap(), "http://10.0.0.1:3128");
        assert_eq!(p.for_torrents(), None, "torrents only go through SOCKS5");
        let v6 = Proxy { host: "::1".into(), kind: ProxyKind::Https, ..p.clone() };
        assert_eq!(v6.url().unwrap(), "https://[::1]:3128");
        let socks = Proxy { kind: ProxyKind::Socks5, remote_dns: false, torrents: true, ..p.clone() };
        assert_eq!(socks.url().unwrap(), "socks5://10.0.0.1:3128");
        assert_eq!(socks.for_torrents().unwrap(), "socks5://10.0.0.1:3128");
        let direct = Proxy { mode: ProxyMode::None, ..p.clone() };
        assert_eq!(direct.for_videos().as_deref(), Some(""), "yt-dlp's way of saying no proxy");
        assert_eq!(Proxy::default().for_videos(), None);
        assert_eq!(Proxy { videos: false, ..p.clone() }.for_videos(), None);
        // Every mode makes a client.
        for mode in [ProxyMode::System, ProxyMode::None, ProxyMode::Manual] {
            let q = Proxy { mode, bypass: vec!["*.lan".into(), "10.0.0.0/8".into(), " ".into()], ..p.clone() };
            q.configure(reqwest::Client::builder()).unwrap().build().unwrap();
        }
    }

    #[test]
    fn settings_from_before_presets_load_with_the_new_defaults() {
        // 0.8 phase 1 wrote exactly these fields.
        let old = r#"{"default_dir":"/home/u/Downloads","connections":8,"max_concurrent":4,"speed_limit":0,"limit_enabled":false,
            "schedule":{"enabled":false,"start":null,"stop":null,"days":[]},"sort_by_category":true,"category_dirs":{},"temp_dir":null,
            "seed_ratio":1.0,"upload_limit":0,"torrent_port":6881,"torrent_dht":true}"#;
        let mut s: Settings = serde_json::from_str(old).unwrap();
        s.normalize();
        assert_eq!(s.performance, Performance::Balanced, "8 and 4 are the balanced preset");
        assert_eq!((s.write_buffer_kib, s.retries, s.retry_wait_secs, s.idle_timeout_secs, s.connect_timeout_secs), (256, 5, 3, 60, 30));
        assert!(s.keep_awake && !s.keep_awake_seeding);
        assert_eq!(s.min_free_space_mib, 200);
        assert_eq!(s.proxy, Proxy::default());
        let tuned: Settings = serde_json::from_str(&old.replace(r#""connections":8"#, r#""connections":16"#)).unwrap();
        let mut tuned = tuned;
        tuned.normalize();
        assert_eq!(tuned.performance, Performance::Custom, "numbers set by hand before presets stay as they were");
        assert_eq!(tuned.connections, 16);
    }
}
