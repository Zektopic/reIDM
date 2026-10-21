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
    /// The folder its files were put in. Recorded before the first file is created, so that
    /// files found there later are known to be this torrent's own; `None` for torrents begun
    /// by reIDM 0.7.0, whose files are beside the download's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
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

/// The largest `.torrent` file reIDM reads. Real ones are kilobytes to a few megabytes (a
/// torrent of tens of thousands of files with small pieces); anything bigger is not one, and an
/// address that streams without end must not be read into memory until it runs out.
pub const MAX_TORRENT_FILE: usize = 10 << 20;

/// A response's body, at most `cap` bytes of it, failing when the server pauses longer than
/// `stall` between two pieces. `Content-Length` is checked first, but a chunked body says
/// nothing up front, so the count is kept while reading as well.
pub(crate) async fn read_capped(resp: reqwest::Response, cap: usize, stall: Duration, what: &str) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    let too_big = || anyhow!("{what} is larger than {} MiB; not reading it", cap >> 20);
    if resp.content_length().is_some_and(|n| n > cap as u64) {
        return Err(too_big());
    }
    let mut body = resp.bytes_stream();
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(stall, body.next()).await {
            Err(_) => bail!("the server stopped sending {what}"),
            Ok(None) => return Ok(out),
            Ok(Some(chunk)) => {
                let chunk = chunk?;
                if out.len() + chunk.len() > cap {
                    return Err(too_big());
                }
                out.extend_from_slice(&chunk);
            }
        }
    }
}

/// A `.torrent` file kept on disk, read with the same limit.
fn read_torrent_file(f: &Path) -> Result<Vec<u8>> {
    let len = std::fs::metadata(f).with_context(|| format!("cannot read {}", f.display()))?.len();
    if len > MAX_TORRENT_FILE as u64 {
        bail!("{} is larger than {} MiB, too large to be a .torrent", f.display(), MAX_TORRENT_FILE >> 20);
    }
    std::fs::read(f).with_context(|| format!("cannot read {}", f.display()))
}

/// The torrents being taken out of each manager's session right now (see
/// [`Manager::forget_torrent`]): each flips to `true` once its removal, files and all, is over.
/// Keyed by the manager, so tests that run several managers in one process don't wait on each
/// other.
static FORGETTING: std::sync::Mutex<BTreeMap<usize, Vec<tokio::sync::watch::Receiver<bool>>>> = std::sync::Mutex::new(BTreeMap::new());

/// What [`Manager::add_to_session`] decided: where the torrent's files go, and whether that is
/// reIDM carrying on its own files there.
struct Target {
    folder: PathBuf,
    /// What the list shows: the one file, or the folder of several.
    path: PathBuf,
    resuming: bool,
}

impl Manager {
    fn manager_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }

    /// Add a torrent from a `.torrent` file's bytes: a copy is kept in the data folder, and the
    /// download is known by its info hash, as one added by magnet link is.
    pub fn add_torrent_file(&self, bytes: &[u8], files: Option<Vec<usize>>, later: bool) -> Result<AddOutcome> {
        if bytes.len() > MAX_TORRENT_FILE {
            bail!("that file is larger than {} MiB, too large to be a .torrent", MAX_TORRENT_FILE >> 20);
        }
        let meta = librqbit::torrent_from_bytes(bytes).map_err(|e| anyhow!("not a .torrent file: {e}"))?;
        let hash = hash_hex(&meta.info_hash);
        let name = meta.info.data.name.as_ref().map(|n| String::from_utf8_lossy(n.as_ref()).into_owned()).unwrap_or_default();
        let file = self.keep_torrent_file(&hash, bytes)?;
        let enc = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
        let url = format!("magnet:?xt=urn:btih:{hash}{}", if name.is_empty() { String::new() } else { format!("&dn={}", enc(&name)) });
        let req = Request { url, torrent: Some(TorrentRequest { files, file: Some(file), peers: Vec::new() }), ..Default::default() };
        self.add_with(req, AddOptions { later, ..Default::default() })
    }

    /// Keep a `.torrent` in the data folder, named by its info hash.
    fn keep_torrent_file(&self, hash: &str, bytes: &[u8]) -> Result<PathBuf> {
        let dir = self.data_dir().join("torrents").join("files");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("{hash}.torrent"));
        std::fs::write(&file, bytes)?;
        Ok(file)
    }

    /// Fetch a `.torrent` from the web, with the download's cookies, referer and user agent (a
    /// private tracker's site wants them), and at most [`MAX_TORRENT_FILE`] of it.
    pub(crate) async fn fetch_torrent_file(&self, req: &Request) -> Result<Vec<u8>> {
        let r = self.http().get(req.url.trim()).headers(Self::headers(req)).timeout(Duration::from_secs(30)).send().await?.error_for_status()?;
        read_capped(r, MAX_TORRENT_FILE, Duration::from_secs(30), "that .torrent").await
    }

    /// At start-up: if the list has torrents still seeding, start the session so they carry on.
    /// (Unfinished ones start with their turn in the queue.)
    pub async fn resume_seeding(&self) {
        let seeding = self.0.db.lock().unwrap().items.values().any(|i| i.status == Status::Done && i.torrent.as_ref().is_some_and(|t| t.seeding));
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
                    // The performance preset's: fewer peers is less work for an old computer.
                    peer_limit: Some(settings.performance.torrent_peers()),
                    // Peers through the SOCKS5 proxy, when the user asked for that.
                    connect: settings.proxy.for_torrents().map(|u| librqbit::ConnectionOptions { proxy_url: Some(u), ..Default::default() }),
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
                // The background work holds the manager weakly, and ends with it.
                let me = Arc::downgrade(&self.0);
                tokio::spawn(async move { watch_seeding(me).await });
                let me = Arc::downgrade(&self.0);
                tokio::spawn(async move { share_the_limit(me).await });
                anyhow::Ok(session)
            })
            .await?;
        Ok(s.clone())
    }

    /// Bring the session's torrents in line with the list after a start: ones the list no
    /// longer has go, finished ones still seeding carry on, and the rest wait for a slot.
    async fn reconcile(&self, s: &Arc<Session>) {
        let wanted: HashMap<String, (Status, bool)> =
            self.0.db.lock().unwrap().items.values().filter_map(|i| i.torrent.as_ref().map(|t| (t.info_hash.clone(), (i.status.clone(), t.seeding)))).collect();
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
        let bytes = match (bytes, url) {
            (Some(b), _) if b.len() > MAX_TORRENT_FILE => bail!("that file is larger than {} MiB, too large to be a .torrent", MAX_TORRENT_FILE >> 20),
            (Some(b), _) => Some(b),
            // A .torrent on the web is fetched here, within the size limit, not by the engine.
            (None, Some(u)) if !u.trim().to_ascii_lowercase().starts_with("magnet:") => Some(self.fetch_torrent_file(&Request { url: u.trim().to_string(), ..Default::default() }).await?),
            (None, Some(_)) => None,
            (None, None) => bail!("give a magnet link, a .torrent address, or a .torrent file"),
        };
        let add = match bytes {
            Some(b) => AddTorrent::from_bytes(b),
            None => AddTorrent::from_url(url.unwrap_or_default().trim().to_string()),
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

    /// Wait until every torrent this manager is taking out of its session is gone. A torrent
    /// started again ("Start again", or a re-add after Remove) must not find the old one still
    /// there: it would carry on with that handle, whose files the pending removal then deletes
    /// from under it.
    async fn forgotten(&self) {
        let pending: Vec<tokio::sync::watch::Receiver<bool>> = FORGETTING.lock().unwrap().get(&self.manager_key()).cloned().unwrap_or_default();
        for mut rx in pending {
            // An error means the removal's task ended (it was the last sender): done as well.
            let _ = rx.wait_for(|done| *done).await;
        }
    }

    /// Download a torrent. Ok(true) when complete (it then goes on seeding, if it should),
    /// Ok(false) when stopped.
    pub(crate) async fn run_torrent(&self, id: u64, live: Live, item: Item) -> Result<bool> {
        self.set_note(id, Some("starting BitTorrent"));
        let s = self.session().await?;
        self.forgotten().await;

        let known = item.torrent.as_ref().and_then(|t| Self::handle(&s, &t.info_hash));
        let handle = match known {
            // One that failed (its disk filled, say) is added again over its files: the engine
            // would otherwise reopen them under the rules it was first added with.
            Some(h) if matches!(h.stats().state, librqbit::TorrentStatsState::Error) => {
                let _ = s.delete(librqbit::api::TorrentIdOrHash::Id(h.id()), false).await;
                match self.add_to_session(id, &live, &s, &item).await? {
                    Some(h) => h,
                    None => return Ok(false),
                }
            }
            Some(h) => {
                if h.is_paused() {
                    s.unpause(&h).await?;
                }
                h
            }
            None => match self.add_to_session(id, &live, &s, &item).await? {
                Some(h) => h,
                None => return Ok(false),
            },
        };

        let mut last_save = Instant::now();
        let mut last_space = Instant::now();
        loop {
            // reIDM doesn't see the torrent engine's writes, so it looks at the space now and then.
            if last_space.elapsed() >= crate::SPACE_CHECK_EVERY {
                last_space = Instant::now();
                if let Some(it) = self.get(id).filter(|it| it.size.is_some_and(|sz| it.downloaded < sz)) {
                    if let Some(why) = self.short_of_space(&it).await {
                        live.stop_for_space(why);
                    }
                }
            }
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
                // Each file is marked as from the internet, as a browser's download would be.
                if let Some(it) = self.get(id) {
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Some(t) = &it.torrent {
                            let folder = t.folder.clone().or_else(|| it.path.parent().map(Path::to_path_buf));
                            for f in t.files.iter().filter(|f| f.selected) {
                                if let Some(dir) = &folder {
                                    crate::disk::mark_from_internet(&dir.join(&f.path), &it.request.url, it.request.referer.as_deref());
                                }
                            }
                        }
                    })
                    .await;
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

    /// Put a torrent the session doesn't have into it: read its description, decide where its
    /// files go, and add it. `None` when stopped meanwhile.
    async fn add_to_session(&self, id: u64, live: &Live, s: &Arc<Session>, item: &Item) -> Result<Option<Arc<librqbit::ManagedTorrent>>> {
        let req = item.request.torrent.clone().unwrap_or_default();
        let peers: Vec<std::net::SocketAddr> = req.peers.iter().filter_map(|p| p.parse().ok()).collect();
        let url = item.request.url.trim();
        // Its description first. A magnet link's comes from peers, which can take a while; it
        // is then kept as a .torrent, so this is asked of them only once.
        let bytes = match &req.file {
            Some(f) => Some(read_torrent_file(f)?),
            None if url.to_ascii_lowercase().starts_with("magnet:") => None,
            None => {
                self.set_note(id, Some("reading the torrent"));
                Some(self.fetch_torrent_file(&item.request).await?)
            }
        };
        let add = match bytes {
            Some(b) => AddTorrent::from_bytes(b),
            None => {
                self.set_note(id, Some("asking peers for the torrent"));
                AddTorrent::from_url(url.to_string())
            }
        };
        let describe = s.add_torrent(add, Some(AddTorrentOptions { list_only: true, initial_peers: (!peers.is_empty()).then(|| peers.clone()), ..Default::default() }));
        let Some(described) = unless_stopped(&live.cancel, describe).await else { return Ok(None) };
        let AddTorrentResponse::ListOnly(l) = described? else { bail!("the torrent could not be read") };
        let hash = hash_hex(&l.info_hash);
        let bytes = l.torrent_bytes.to_vec();

        // The same torrent twice (a magnet link with its hash in base32, the .torrent from
        // another site) would share one transfer and one set of files: removing either with
        // its files would delete the other's.
        let twin = self.0.db.lock().unwrap().items.values().find(|i| i.id != id && i.torrent.as_ref().is_some_and(|t| t.info_hash == hash)).map(|i| i.id);
        if let Some(other) = twin {
            bail!("this torrent is already in the list (download {other}); remove one of the two");
        }

        let file = match &req.file {
            Some(f) => f.clone(),
            None => self.keep_torrent_file(&hash, &bytes)?,
        };
        let name = l.info.name().map(|n| n.to_string()).unwrap_or_default();
        let details: Vec<(PathBuf, u64)> = l.info.iter_file_details().map(|f| (f.filename.to_pathbuf(), f.len)).collect();
        let selected = req.files.clone();
        let files: Vec<TorrentFile> = details
            .iter()
            .enumerate()
            .map(|(i, (p, len))| TorrentFile { path: p.to_string_lossy().into_owned(), size: *len, done: 0, selected: selected.as_ref().is_none_or(|s| s.contains(&i)) })
            .collect();

        // Where: recorded, and saved, before the engine creates a single file — so that from
        // then on the files there are known to be this torrent's own.
        let target = {
            let mut db = self.0.db.lock().unwrap();
            let claimed: HashSet<PathBuf> = db.items.values().filter(|i| i.id != id).map(|i| i.path.clone()).collect();
            let it = db.items.get_mut(&id).context("the download was removed")?;
            let target = choose_target(it, &hash, &name, &details, &claimed)?;
            it.path = target.path.clone();
            if let Some(r) = it.request.torrent.as_mut() {
                r.file = Some(file);
            }
            let t = it.torrent.get_or_insert_with(Default::default);
            t.info_hash = hash.clone();
            t.name = name;
            t.folder = Some(target.folder.clone());
            t.files = files;
            target
        };
        self.save();
        std::fs::create_dir_all(&target.folder)?;

        let opts = AddTorrentOptions {
            output_folder: Some(target.folder.to_string_lossy().into_owned()),
            only_files: req.files.clone(),
            // Only reIDM's own files are ever written over: those of a torrent it began, which
            // it is carrying on. A new torrent's files are created new, and the engine refuses
            // to touch any file that is already there.
            overwrite: target.resuming,
            initial_peers: (!peers.is_empty()).then_some(peers),
            ratelimits: librqbit::limits::LimitsConfig { upload_bps: None, download_bps: limit(item.speed_limit.unwrap_or(0)) },
            ..Default::default()
        };
        let adding = s.add_torrent(AddTorrent::from_bytes(bytes), Some(opts));
        let Some(added) = unless_stopped(&live.cancel, adding).await else { return Ok(None) };
        let added = added.map_err(|e| {
            if format!("{e:#}").contains("allow_overwrite") {
                anyhow!("a file this torrent would create already exists in {}; reIDM won't write over it. Move it away and resume", target.folder.display())
            } else {
                e
            }
        })?;
        Ok(Some(added.into_handle().context("the torrent could not be added")?))
    }

    /// Copy a torrent's numbers into its row. True when anything shown changed.
    fn record(&self, id: u64, st: &librqbit::TorrentStats) -> bool {
        let mut db = self.0.db.lock().unwrap();
        let peers = peers_of(st);
        // Taking an item mutably counts as a change (and schedules a save), and this runs every
        // 500 ms for every torrent: look first, and only write when a number moved, so an idle
        // seed doesn't rewrite the list.
        let Some(it) = db.items.get(&id) else { return false };
        let same = it.downloaded == st.progress_bytes
            && it.size == Some(st.total_bytes)
            && it.torrent.as_ref().is_none_or(|t| {
                t.uploaded == st.uploaded_bytes && t.peers == peers && t.files.iter().map(|f| f.done).eq(st.file_progress.iter().copied().take(t.files.len()))
            });
        if same {
            return false;
        }
        let Some(it) = db.items.get_mut(&id) else { return false };
        it.downloaded = st.progress_bytes;
        it.size = Some(st.total_bytes);
        if let Some(t) = it.torrent.as_mut() {
            t.uploaded = st.uploaded_bytes;
            t.peers = peers;
            for (f, done) in t.files.iter_mut().zip(&st.file_progress) {
                f.done = *done;
            }
        }
        true
    }

    /// Start or stop sharing a finished torrent.
    pub async fn set_seeding(&self, id: u64, on: bool) -> Result<()> {
        let it = self.get(id).ok_or_else(|| anyhow!("no such download {id}"))?;
        let t = it.torrent.as_ref().filter(|_| it.status == Status::Done).context("only a finished torrent can be seeded")?;
        let s = self.session().await?;
        let h = match Self::handle(&s, &t.info_hash) {
            Some(h) => h,
            // Not in the session any more: add it again over the files it has, where it put them.
            None if on => {
                let req = it.request.torrent.clone().unwrap_or_default();
                let add = match &req.file {
                    Some(f) => AddTorrent::from_bytes(read_torrent_file(f)?),
                    None => AddTorrent::from_url(it.request.url.clone()),
                };
                let folder = t.folder.clone().or_else(|| it.path.parent().map(Path::to_path_buf)).context("no folder")?;
                let opts = AddTorrentOptions { output_folder: Some(folder.to_string_lossy().into_owned()), only_files: req.files.clone(), overwrite: true, ..Default::default() };
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

    /// Take a torrent out of the session, with its files or without. It happens in the
    /// background; [`Manager::run_torrent`] waits for it before adding a torrent again.
    pub(crate) fn forget_torrent(&self, hash: String, delete_files: bool) {
        let Some(s) = self.0.torrents.get().cloned() else { return };
        let (tx, rx) = tokio::sync::watch::channel(false);
        {
            let mut all = FORGETTING.lock().unwrap();
            let list = all.entry(self.manager_key()).or_default();
            // Those already over need no waiting for; dropping them keeps the list short.
            list.retain(|r| !*r.borrow() && r.has_changed().is_ok());
            list.push(rx);
        }
        tokio::spawn(async move {
            if Self::handle(&s, &hash).is_some() {
                let id = librqbit::api::TorrentIdOrHash::Hash(librqbit::dht::Id20::from_str(&hash).unwrap_or_default());
                if let Err(e) = s.delete(id, delete_files).await {
                    tracing::warn!("could not remove torrent {hash}: {e:#}");
                }
            }
            let _ = tx.send(true);
        });
    }

    /// New limits for the torrents: the upload limit, and the torrents' share of the global
    /// download limit. Called when the settings change, and every moment by
    /// [`share_the_limit`], as downloads start and stop.
    pub(crate) fn torrent_limits(&self) {
        let Some(s) = self.0.torrents.get() else { return };
        let (global, up) = {
            let st = self.0.settings.lock().unwrap();
            (st.effective_limit(), st.upload_limit)
        };
        let (torrents, all) = {
            let db = self.0.db.lock().unwrap();
            let live = self.0.live.lock().unwrap();
            let torrents = live.keys().filter(|id| db.items.get(id).is_some_and(|i| i.request.torrent.is_some())).count();
            (torrents, live.len())
        };
        s.ratelimits.set_upload_bps(limit(up));
        s.ratelimits.set_download_bps(limit(torrent_share(global, torrents, all)));
    }
}

/// Where a torrent's files go, for [`Manager::add_to_session`]. A torrent reIDM began goes on
/// where it began. A new one never lands on anything already there — another download's file,
/// or the user's own: a torrent of several files gets a folder of its own named after it
/// (`Name (1)` when `Name` is taken), and a torrent of one file goes into the download's folder,
/// or, when a file of that name is already there, into a folder of its own beside it.
fn choose_target(it: &Item, hash: &str, name: &str, details: &[(PathBuf, u64)], claimed: &HashSet<PathBuf>) -> Result<Target> {
    if let Some(t) = it.torrent.as_ref().filter(|t| t.info_hash == hash) {
        // Begun by reIDM 0.7.0, which recorded no folder: its files are beside its path.
        let folder = t.folder.clone().or_else(|| it.path.parent().map(Path::to_path_buf)).context("download has no folder")?;
        return Ok(Target { folder, path: it.path.clone(), resuming: true });
    }
    let dir = it.path.parent().context("download has no folder")?.to_path_buf();
    let taken = |p: &Path| claimed.contains(p);
    let safe_name = sanitize_filename(if name.is_empty() { hash } else { name });
    match details {
        [(file, _)] => {
            let path = dir.join(file);
            if !path.exists() && !taken(&path) {
                return Ok(Target { folder: dir, path, resuming: false });
            }
            let stem = Path::new(&safe_name).file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or(&safe_name).to_string();
            let folder = unique_path(&dir, &stem, taken);
            Ok(Target { path: folder.join(file), folder, resuming: false })
        }
        _ => {
            let folder = unique_path(&dir, &safe_name, taken);
            Ok(Target { path: folder.clone(), folder, resuming: false })
        }
    }
}

/// The torrents' part of a global download limit (0: none), when `torrents` of `all` running
/// downloads are torrents: an even share per download, like yt-dlp's. What the torrents then
/// fetch is also counted against the allowance reIDM's own connections draw from, so together
/// they stay near the limit.
fn torrent_share(global: u64, torrents: usize, all: usize) -> u64 {
    if global == 0 || all == 0 || torrents >= all {
        return global;
    }
    // Never quite zero: librqbit reads 0 as "no limit".
    (global * torrents.max(1) as u64 / all as u64).max(1)
}

/// Finished torrents that are seeding: keep their numbers current, and stop each once it has
/// shared as much as the share ratio says. Ends when the manager does.
async fn watch_seeding(me: std::sync::Weak<Inner>) {
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let Some(inner) = me.upgrade() else { return };
        let m = Manager(inner);
        let Some(s) = m.0.torrents.get().cloned() else { continue };
        let ratio = m.0.settings.lock().unwrap().seed_ratio;
        // Only the seeding ones, picked out under the lock — not a copy of the whole list,
        // every few seconds, for as long as reIDM runs.
        let seeding: Vec<(u64, String)> = m
            .0
            .db
            .lock()
            .unwrap()
            .items
            .values()
            .filter(|i| i.status == Status::Done)
            .filter_map(|i| i.torrent.as_ref().filter(|t| t.seeding).map(|t| (i.id, t.info_hash.clone())))
            .collect();
        let mut changed = false;
        for (id, hash) in seeding {
            let Some(h) = Manager::handle(&s, &hash) else { continue };
            let st = h.stats();
            changed |= m.record(id, &st);
            let reached = ratio >= 0.0 && st.total_bytes > 0 && st.uploaded_bytes as f64 >= ratio * st.total_bytes as f64;
            if reached {
                let _ = s.pause(&h).await;
                if let Some(t) = m.0.db.lock().unwrap().items.get_mut(&id).and_then(|i| i.torrent.as_mut()) {
                    t.seeding = false;
                    t.peers = 0;
                }
                changed = true;
                tracing::info!("torrent {id} shared to ratio {ratio}; stopped seeding");
            }
        }
        // Saved only when something changed: an idle seed used to rewrite the list every 3 s.
        if changed {
            m.save();
        }
    }
}

/// Keep the torrents within their share of the global download limit as downloads come and
/// go, and count what they fetch against the allowance reIDM's own connections draw from, so
/// those leave room for it. Ends when the manager does.
async fn share_the_limit(me: std::sync::Weak<Inner>) {
    let mut fetched: Option<u64> = None;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let Some(inner) = me.upgrade() else { return };
        let m = Manager(inner);
        let Some(s) = m.0.torrents.get().cloned() else { continue };
        m.torrent_limits();
        let now = s.stats_snapshot().counters.fetched_bytes;
        if let Some(before) = fetched {
            let fresh = now.saturating_sub(before);
            if fresh > 0 {
                // Charged without waiting: the HTTP connections do the waiting, as if they had
                // fetched it themselves.
                m.0.limiter.charge(fresh);
            }
        }
        fetched = Some(now);
    }
}

/// Peers connected now, from librqbit's statistics.
fn peers_of(st: &librqbit::TorrentStats) -> u32 {
    st.live.as_ref().map_or(0, |l| l.snapshot.peer_stats.live)
}

/// `fut`, unless `cancel` is set first.
async fn unless_stopped<T>(cancel: &AtomicBool, fut: impl std::future::Future<Output = T>) -> Option<T> {
    tokio::select! {
        v = fut => Some(v),
        _ = async { while !cancel.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(100)).await } } => None,
    }
}

/// A magnet link's info hash, the way the list knows it: 40 lower-case hex digits. Magnet links
/// may give it in base32 instead (32 letters and digits), which is the same torrent.
pub fn btih_hex(xt: &str) -> Option<String> {
    let h = xt.trim().strip_prefix("urn:btih:").or_else(|| xt.trim().strip_prefix("URN:BTIH:")).unwrap_or(xt.trim());
    if h.len() == 40 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(h.to_ascii_lowercase());
    }
    if h.len() == 32 {
        let mut bits: u64 = 0;
        let mut n = 0;
        let mut out = String::with_capacity(40);
        for c in h.chars() {
            let v = match c.to_ascii_uppercase() {
                c @ 'A'..='Z' => c as u64 - 'A' as u64,
                c @ '2'..='7' => c as u64 - '2' as u64 + 26,
                _ => return None,
            };
            bits = (bits << 5) | v;
            n += 5;
            if n >= 8 {
                n -= 8;
                out.push_str(&format!("{:02x}", (bits >> n) & 0xff));
            }
        }
        return Some(out);
    }
    None
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
    fn info_hashes_in_base32_are_the_same_as_in_hex() {
        let hex = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(btih_hex(&format!("urn:btih:{}", hex.to_uppercase())).as_deref(), Some(hex));
        assert_eq!(btih_hex("urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH").as_deref(), Some(hex));
        assert_eq!(btih_hex("urn:btih:aeruKZ4JVPG66AJDIVTYTK6N54ASGRLH").as_deref(), Some(hex), "either case");
        assert_eq!(btih_hex("urn:btih:not-a-hash"), None);
        let key = |u: &str| crate::url_key(&url::Url::parse(u).unwrap());
        assert_eq!(key(&format!("magnet:?xt=urn:btih:{hex}&dn=a")), key("magnet:?dn=b&xt=urn:btih:AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH"));
    }

    #[test]
    fn torrents_get_their_share_of_the_global_limit() {
        assert_eq!(torrent_share(0, 1, 3), 0, "no limit, none for them either");
        assert_eq!(torrent_share(900, 1, 1), 900, "alone, all of it");
        assert_eq!(torrent_share(900, 1, 3), 300);
        assert_eq!(torrent_share(900, 2, 3), 600);
        // Only seeding (nothing of theirs running): what one more download would get.
        assert_eq!(torrent_share(900, 0, 2), 450);
        assert_eq!(torrent_share(900, 0, 0), 900);
        assert_eq!(torrent_share(1, 1, 5), 1, "never 0, which would mean no limit");
    }

    #[test]
    fn magnet_names() {
        assert_eq!(magnet_name("magnet:?xt=urn:btih:abc&dn=Big+Buck+Bunny%20%282008%29").as_deref(), Some("Big Buck Bunny (2008)"));
        assert_eq!(magnet_name("magnet:?xt=urn:btih:abc"), None);
    }
}
