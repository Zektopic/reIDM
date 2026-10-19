//! BitTorrent: magnet links and `.torrent` files, downloaded and seeded with librqbit.
//!
//! A torrent is a download in reIDM's list like any other: it takes a slot while it downloads,
//! shows its progress, stops and resumes, and is removed with or without its files. When it is
//! complete it goes on *seeding* — sharing it with others — until it has uploaded as much as the
//! share-ratio setting says, then stops; "Stop seeding" and "Seed again" do it by hand.
//!
//! One librqbit session serves every torrent. It keeps its own records (and what each torrent
//! already has, so a restart doesn't check every piece again) in `torrents/` in the data folder;
//! reIDM's list stays the one that says what exists and in what state, and the two are matched
//! by info hash.

use super::*;
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, ListenerMode, ListenerOptions, Session, SessionOptions, SessionPersistenceConfig};
use std::num::NonZeroU32;

/// What to download of a torrent; on [`Request::torrent`]. The request's URL is the magnet link,
/// or the address of the `.torrent` file.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TorrentRequest {
    /// The files to download, by index in the torrent; all of them when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<usize>>,
    /// The `.torrent` file it was added from, kept in the data folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// Peers to try before any others, as `ip:port` — someone on the same network, say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<String>,
}

/// A torrent's state beyond what every download has; on [`Item::torrent`].
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TorrentStatus {
    pub info_hash: String,
    pub name: String,
    /// Uploaded to others, in total.
    pub uploaded: u64,
    /// Peers connected now.
    pub peers: u32,
    /// Sharing it with others now that it is complete.
    pub seeding: bool,
    pub files: Vec<TorrentFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TorrentFile {
    pub path: String,
    pub size: u64,
    /// Downloaded and checked.
    pub done: u64,
    pub selected: bool,
}

/// What a torrent holds, before adding it: for choosing its files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentInfo {
    pub name: String,
    pub info_hash: String,
    pub files: Vec<TorrentFile>,
    pub size: u64,
}

/// Whether `url` is a torrent rather than a file to fetch: a magnet link, or an address whose
/// file is a `.torrent`.
pub fn is_torrent_url(url: &str) -> bool {
    let u = url.trim();
    if u.len() >= 7 && u[..7].eq_ignore_ascii_case("magnet:") {
        return true;
    }
    url::Url::parse(u).is_ok_and(|u| u.path().to_ascii_lowercase().ends_with(".torrent"))
}

/// The name a magnet link gives (`dn=`), if any.
pub fn magnet_name(url: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| kv.strip_prefix("dn=")).map(|v| percent_encoding::percent_decode_str(&v.replace('+', " ")).decode_utf8_lossy().into_owned()).filter(|n| !n.is_empty())
}

fn hash_hex(h: &librqbit::dht::Id20) -> String {
    h.as_string()
}

/// librqbit's limits want a non-zero `u32`; reIDM's 0 means none.
fn limit(bps: u64) -> Option<NonZeroU32> {
    NonZeroU32::new(bps.min(u32::MAX as u64) as u32)
}

impl Manager {
    /// Add a torrent from a `.torrent` file's bytes: a copy is kept in the data folder, and the
    /// download is known by its info hash, as one added by magnet link is.
    pub fn add_torrent_file(&self, bytes: &[u8], files: Option<Vec<usize>>, later: bool) -> Result<AddOutcome> {
        let meta = librqbit::torrent_from_bytes(bytes).map_err(|e| anyhow!("not a .torrent file: {e}"))?;
        let hash = hash_hex(&meta.info_hash);
        let name = meta.info.data.name.as_ref().map(|n| String::from_utf8_lossy(n.as_ref()).into_owned()).unwrap_or_default();
        let dir = self.data_dir().join("torrents").join("files");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("{hash}.torrent"));
        std::fs::write(&file, bytes)?;
        let enc = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
        let url = format!("magnet:?xt=urn:btih:{hash}{}", if name.is_empty() { String::new() } else { format!("&dn={}", enc(&name)) });
        let req = Request { url, torrent: Some(TorrentRequest { files, file: Some(file), peers: Vec::new() }), ..Default::default() };
        self.add_with(req, AddOptions { later, ..Default::default() })
    }

    /// At start-up: if the list has torrents still seeding, start the session so they carry on.
    /// (Unfinished ones start with their turn in the queue.)
    pub async fn resume_seeding(&self) {
        let seeding = self.list().iter().any(|i| i.status == Status::Done && i.torrent.as_ref().is_some_and(|t| t.seeding));
        if seeding {
            if let Err(e) = self.session().await {
                tracing::warn!("torrents can't seed: {e:#}");
            }
        }
    }

    /// The torrent session, started the first time a torrent needs it.
    pub(crate) async fn session(&self) -> Result<Arc<Session>> {
        let s = self
            .0
            .torrents
            .get_or_try_init(|| async {
                let settings = self.settings();
                let dir = self.data_dir().join("torrents");
                std::fs::create_dir_all(&dir)?;
                let dht = || librqbit::DhtSessionConfig {
                    persistence: Some(librqbit::dht::DhtPersistenceConfig { config_filename: Some(dir.join("dht.json")), ..Default::default() }),
                    ..Default::default()
                };
                let opts = |port: u16| SessionOptions {
                    dht: settings.torrent_dht.then(dht),
                    fastresume: true,
                    persistence: Some(SessionPersistenceConfig::Json { folder: Some(dir.clone()) }),
                    listen: Some(ListenerOptions {
                        mode: ListenerMode::TcpAndUtp,
                        listen_addr: (std::net::Ipv6Addr::UNSPECIFIED, port).into(),
                        // Ask the router to forward the port, so peers can reach us too.
                        enable_upnp_port_forwarding: port != 0,
                        ..Default::default()
                    }),
                    ratelimits: librqbit::limits::LimitsConfig { upload_bps: limit(settings.upload_limit), download_bps: limit(settings.effective_limit()) },
                    client_name_and_version: Some(concat!("reIDM ", env!("CARGO_PKG_VERSION")).into()),
                    ..Default::default()
                };
                let session = match Session::new_with_opts(settings.default_dir.clone(), opts(settings.torrent_port)).await {
                    Ok(s) => s,
                    // The port is taken (another client, another reIDM): any free one will do.
                    Err(e) if settings.torrent_port != 0 => {
                        tracing::warn!("torrent port {} unavailable ({e:#}); using another", settings.torrent_port);
                        Session::new_with_opts(settings.default_dir.clone(), opts(0)).await?
                    }
                    Err(e) => return Err(e),
                };
                self.reconcile(&session).await;
                let me = self.clone();
                tokio::spawn(async move { me.watch_seeding().await });
                anyhow::Ok(session)
            })
            .await?;
        Ok(s.clone())
    }

    /// Bring the session's torrents in line with the list after a start: ones the list no
    /// longer has go, finished ones still seeding carry on, and the rest wait for a slot.
    async fn reconcile(&self, s: &Arc<Session>) {
        let wanted: HashMap<String, (Status, bool)> = self
            .list()
            .into_iter()
            .filter_map(|i| i.torrent.map(|t| (t.info_hash, (i.status, t.seeding))))
            .collect();
        let present: Vec<(usize, String, Arc<librqbit::ManagedTorrent>)> = s.with_torrents(|it| it.map(|(id, h)| (id, hash_hex(&h.info_hash()), h.clone())).collect());
        for (id, hash, h) in present {
            match wanted.get(&hash) {
                None => {
                    let _ = s.delete(librqbit::api::TorrentIdOrHash::Id(id), false).await;
                }
                Some((Status::Done, true)) => {
                    if h.is_paused() {
                        let _ = s.unpause(&h).await;
                    }
                }
                Some(_) => {
                    if !h.is_paused() {
                        let _ = s.pause(&h).await;
                    }
                }
            }
        }
    }

    /// The torrent with this info hash in the session.
    fn handle(s: &Session, hash: &str) -> Option<Arc<librqbit::ManagedTorrent>> {
        s.with_torrents(|it| it.map(|(_, h)| h.clone()).find(|h| hash_hex(&h.info_hash()) == hash))
    }

    /// What a torrent holds, without downloading it — for a magnet link, once peers have
    /// answered with its description, which can take a while.
    pub async fn inspect_torrent(&self, url: Option<&str>, bytes: Option<Vec<u8>>) -> Result<TorrentInfo> {
        let s = self.session().await?;
        let add = match (bytes, url) {
            (Some(b), _) => AddTorrent::from_bytes(b),
            (None, Some(u)) => AddTorrent::from_url(u.trim().to_string()),
            (None, None) => bail!("give a magnet link, a .torrent address, or a .torrent file"),
        };
        let r = tokio::time::timeout(Duration::from_secs(120), s.add_torrent(add, Some(AddTorrentOptions { list_only: true, ..Default::default() })))
            .await
            .context("no peer described the torrent within two minutes; it may have no one sharing it")??;
        let AddTorrentResponse::ListOnly(l) = r else {
            bail!("that torrent is already in reIDM");
        };
        let files: Vec<TorrentFile> = l.info.iter_file_details().map(|f| TorrentFile { path: f.filename.to_vec().join("/"), size: f.len, done: 0, selected: true }).collect();
        Ok(TorrentInfo {
            name: l.info.name().map(|n| n.to_string()).unwrap_or_default(),
            info_hash: hash_hex(&l.info_hash),
            size: files.iter().map(|f| f.size).sum(),
            files,
        })
    }

    /// Download a torrent. Ok(true) when complete (it then goes on seeding, if it should),
    /// Ok(false) when stopped.
    pub(crate) async fn run_torrent(&self, id: u64, live: Live, item: Item) -> Result<bool> {
        let req = item.request.torrent.clone().unwrap_or_default();
        self.set_note(id, Some("starting BitTorrent"));
        let s = self.session().await?;
        let dir = item.path.parent().context("download has no folder")?.to_path_buf();
        std::fs::create_dir_all(&dir)?;

        let known = item.torrent.as_ref().and_then(|t| Self::handle(&s, &t.info_hash));
        let handle = match known {
            Some(h) => {
                if h.is_paused() {
                    s.unpause(&h).await?;
                }
                h
            }
            None => {
                let add = match &req.file {
                    Some(f) => AddTorrent::from_bytes(std::fs::read(f).with_context(|| format!("cannot read {}", f.display()))?),
                    None => AddTorrent::from_url(item.request.url.trim().to_string()),
                };
                let peers: Vec<std::net::SocketAddr> = req.peers.iter().filter_map(|p| p.parse().ok()).collect();
                let opts = AddTorrentOptions {
                    output_folder: Some(dir.to_string_lossy().into_owned()),
                    only_files: req.files.clone(),
                    // Resuming, or seeding what is already on disk, writes over its own files.
                    overwrite: true,
                    initial_peers: (!peers.is_empty()).then_some(peers),
                    ratelimits: librqbit::limits::LimitsConfig { upload_bps: None, download_bps: limit(item.speed_limit.unwrap_or(0)) },
                    ..Default::default()
                };
                self.set_note(id, Some(if item.request.url.starts_with("magnet:") { "asking peers for the torrent" } else { "reading the torrent" }));
                let adding = s.add_torrent(add, Some(opts));
                let r = tokio::select! {
                    r = adding => r?,
                    _ = async { while !live.cancel.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(250)).await } } => return Ok(false),
                };
                r.into_handle().context("the torrent could not be added")?
            }
        };

        // Its name, files and where it goes, now that the torrent is described.
        let (name, files) = handle
            .with_metadata(|m| {
                let selected = handle.only_files();
                let files = m.file_infos.iter().enumerate().map(|(i, f)| TorrentFile {
                    path: f.relative_filename.to_string_lossy().into_owned(),
                    size: f.len,
                    done: 0,
                    selected: selected.as_ref().is_none_or(|s| s.contains(&i)),
                });
                (m.info.name().map(|n| n.to_string()).unwrap_or_default(), files.collect::<Vec<_>>())
            })
            .unwrap_or_default();
        {
            let mut db = self.0.db.lock().unwrap();
            if let Some(it) = db.items.get_mut(&id) {
                // A torrent of one file is that file; of several, a folder of its name.
                if !name.is_empty() {
                    it.path = dir.join(sanitize_filename(&name));
                }
                let t = it.torrent.get_or_insert_with(Default::default);
                t.info_hash = hash_hex(&handle.info_hash());
                t.name = name;
                t.files = files;
            }
        }
        self.save();

        let mut last_save = Instant::now();
        loop {
            if live.cancel.load(Ordering::SeqCst) {
                let _ = s.pause(&handle).await;
                if let Some(t) = self.0.db.lock().unwrap().items.get_mut(&id).and_then(|i| i.torrent.as_mut()) {
                    t.peers = 0;
                }
                return Ok(false);
            }
            let st = handle.stats();
            if let Some(e) = &st.error {
                bail!("{e}");
            }
            // Running here means running there. After a restart the session pauses what it
            // restored until its turn comes; a pause that lands while the torrent is still
            // checking its files leaves it paused once the check ends, after the unpause above
            // has already looked. So: whenever it is found paused, start it again.
            if matches!(st.state, librqbit::TorrentStatsState::Paused) {
                s.unpause(&handle).await?;
            }
            self.record(id, &st);
            live.progress.store(st.progress_bytes, Ordering::Relaxed);
            let note = match &st.state {
                librqbit::TorrentStatsState::Initializing { .. } => Some("checking what is already on disk"),
                _ if st.progress_bytes == 0 && peers_of(&st) == 0 => Some("looking for peers"),
                _ => None,
            };
            self.set_note(id, note);
            if st.finished {
                // 0: not at all; above 0: until that ratio; below 0: for as long as reIDM runs.
                let seed = self.0.settings.lock().unwrap().seed_ratio != 0.0;
                if !seed {
                    let _ = s.pause(&handle).await;
                }
                if let Some(t) = self.0.db.lock().unwrap().items.get_mut(&id).and_then(|i| i.torrent.as_mut()) {
                    t.seeding = seed;
                }
                return Ok(true);
            }
            if last_save.elapsed() > Duration::from_secs(5) {
                self.save();
                last_save = Instant::now();
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Copy a torrent's numbers into its row.
    fn record(&self, id: u64, st: &librqbit::TorrentStats) {
        let mut db = self.0.db.lock().unwrap();
        let Some(it) = db.items.get_mut(&id) else { return };
        it.downloaded = st.progress_bytes;
        it.size = Some(st.total_bytes);
        if let Some(t) = it.torrent.as_mut() {
            t.uploaded = st.uploaded_bytes;
            t.peers = peers_of(st);
            for (f, done) in t.files.iter_mut().zip(&st.file_progress) {
                f.done = *done;
            }
        }
    }

    /// Finished torrents that are seeding: keep their numbers current, and stop each once it has
    /// shared as much as the share ratio says.
    async fn watch_seeding(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let Some(s) = self.0.torrents.get() else { continue };
            let ratio = self.0.settings.lock().unwrap().seed_ratio;
            let seeding: Vec<(u64, String)> = self
                .list()
                .into_iter()
                .filter(|i| i.status == Status::Done)
                .filter_map(|i| i.torrent.filter(|t| t.seeding).map(|t| (i.id, t.info_hash)))
                .collect();
            let mut changed = false;
            for (id, hash) in seeding {
                let Some(h) = Self::handle(s, &hash) else { continue };
                let st = h.stats();
                self.record(id, &st);
                changed = true;
                let reached = ratio >= 0.0 && st.total_bytes > 0 && st.uploaded_bytes as f64 >= ratio * st.total_bytes as f64;
                if reached {
                    let _ = s.pause(&h).await;
                    if let Some(t) = self.0.db.lock().unwrap().items.get_mut(&id).and_then(|i| i.torrent.as_mut()) {
                        t.seeding = false;
                        t.peers = 0;
                    }
                    tracing::info!("torrent {id} shared to ratio {ratio}; stopped seeding");
                }
            }
            if changed {
                self.save();
            }
        }
    }

    /// Start or stop sharing a finished torrent.
    pub async fn set_seeding(&self, id: u64, on: bool) -> Result<()> {
        let it = self.get(id).ok_or_else(|| anyhow!("no such download {id}"))?;
        let t = it.torrent.as_ref().filter(|_| it.status == Status::Done).context("only a finished torrent can be seeded")?;
        let s = self.session().await?;
        let h = match Self::handle(&s, &t.info_hash) {
            Some(h) => h,
            // Not in the session any more: add it again over the files it has.
            None if on => {
                let req = it.request.torrent.clone().unwrap_or_default();
                let add = match &req.file {
                    Some(f) => AddTorrent::from_bytes(std::fs::read(f)?),
                    None => AddTorrent::from_url(it.request.url.clone()),
                };
                let dir = it.path.parent().context("no folder")?.to_string_lossy().into_owned();
                let opts = AddTorrentOptions { output_folder: Some(dir), only_files: req.files.clone(), overwrite: true, ..Default::default() };
                s.add_torrent(add, Some(opts)).await?.into_handle().context("the torrent could not be added")?
            }
            None => return Ok(()),
        };
        if on {
            s.unpause(&h).await?;
        } else {
            s.pause(&h).await?;
        }
        if let Some(t) = self.0.db.lock().unwrap().items.get_mut(&id).and_then(|i| i.torrent.as_mut()) {
            t.seeding = on;
            if !on {
                t.peers = 0;
            }
        }
        self.save();
        Ok(())
    }

    /// Take a torrent out of the session, with its files or without.
    pub(crate) fn forget_torrent(&self, hash: String, delete_files: bool) {
        let me = self.clone();
        tokio::spawn(async move {
            if let Some(s) = me.0.torrents.get() {
                if Self::handle(s, &hash).is_some() {
                    let id = librqbit::api::TorrentIdOrHash::Hash(librqbit::dht::Id20::from_str(&hash).unwrap_or_default());
                    if let Err(e) = s.delete(id, delete_files).await {
                        tracing::warn!("could not remove torrent {hash}: {e:#}");
                    }
                }
            }
        });
    }

    /// New limits for the torrents: the global download limit and the upload limit.
    pub(crate) fn torrent_limits(&self) {
        if let Some(s) = self.0.torrents.get() {
            let st = self.0.settings.lock().unwrap();
            s.ratelimits.set_upload_bps(limit(st.upload_limit));
            s.ratelimits.set_download_bps(limit(st.effective_limit()));
        }
    }
}

/// Peers connected now, from librqbit's statistics.
fn peers_of(st: &librqbit::TorrentStats) -> u32 {
    serde_json::to_value(st).ok().and_then(|v| v["live"]["snapshot"]["peer_stats"]["live"].as_u64()).unwrap_or(0) as u32
}

use std::collections::HashMap;
use std::str::FromStr;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torrents_are_recognised() {
        assert!(is_torrent_url("magnet:?xt=urn:btih:abc"));
        assert!(is_torrent_url("MAGNET:?xt=urn:btih:abc"));
        assert!(is_torrent_url("https://releases.example.org/debian-13.iso.torrent"));
        assert!(is_torrent_url("https://example.org/get/file.TORRENT?token=1"));
        assert!(!is_torrent_url("https://example.org/debian-13.iso"));
        assert!(!is_torrent_url("https://example.org/torrent?x=1"));
    }

    #[test]
    fn magnet_names() {
        assert_eq!(magnet_name("magnet:?xt=urn:btih:abc&dn=Big+Buck+Bunny%20%282008%29").as_deref(), Some("Big Buck Bunny (2008)"));
        assert_eq!(magnet_name("magnet:?xt=urn:btih:abc"), None);
    }
}
