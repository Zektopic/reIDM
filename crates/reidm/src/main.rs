mod install;
mod launch;
mod logfile;
mod rows;
mod web;
mod session;
mod ws;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use reidm_core::{default_data_dir, default_download_dir, Manager, Request, Status};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "reidm", version, about = "reIDM - cross-platform download manager with IDM browser-extension support")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the background service the browser extension talks to.
    Daemon {
        /// Parallel connections per download.
        #[arg(short, long, default_value_t = 8)]
        connections: usize,
        /// Download directory (default: your Downloads folder).
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// Do not try to bind the IDM-compatible privileged port 1001.
        #[arg(long)]
        no_idm_port: bool,
    },
    /// Download a URL right now in this terminal (no daemon needed).
    Get {
        /// The address to download.
        url: String,
        /// Parallel connections.
        #[arg(short, long, default_value_t = 8)]
        connections: usize,
        /// Folder to save into (default: your Downloads folder).
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// File name to save as (default: the server's name).
        #[arg(short, long)]
        output: Option<String>,
        /// Referer header to send.
        #[arg(long)]
        referer: Option<String>,
        /// Cookie header to send, e.g. "session=abc; theme=dark".
        #[arg(long)]
        cookie: Option<String>,
    },
    /// Queue one or more URLs in the running daemon. Re-adding a URL resumes it.
    Add {
        /// One or more addresses: web links, magnet links, or `.torrent` files on disk.
        #[arg(required = true)]
        urls: Vec<String>,
        /// Referer header to send with every address.
        #[arg(long)]
        referer: Option<String>,
        /// File name to save as (a single address only).
        #[arg(short, long)]
        output: Option<String>,
        /// Add to the scheduled queue without starting (IDM's "Download Later").
        #[arg(long)]
        later: bool,
        /// The addresses are pages with a video (YouTube and the many other sites yt-dlp
        /// supports): fetch the video through yt-dlp, which reIDM downloads on first use.
        /// FORMAT is a yt-dlp format selector; without one, the best video with its sound.
        #[arg(long, value_name = "FORMAT", num_args = 0..=1, default_missing_value = "bestvideo*+bestaudio/best")]
        video: Option<String>,
        /// The checksum the finished file should have, like `sha256:9f86d0…` (md5, sha1, sha256
        /// or sha512). Checked once it is downloaded; a file that differs is kept, and the
        /// download fails saying so. A single address only.
        #[arg(long, value_name = "ALGO:HEX")]
        checksum: Option<String>,
    },
    /// What to do once every download is done: `none`, `exit`, `sleep`, `hibernate` or
    /// `shutdown`. Without one, says what is set. Lasts until reIDM stops; there is a minute's
    /// countdown to cancel it (`reidm when-done none`).
    WhenDone {
        action: Option<String>,
    },
    /// Queue a numbered or lettered series: `reidm batch 'https://x/img*.jpg' --from 1 --to 50 --width 3`.
    Batch {
        /// An address with one `*`, replaced by each value of the range.
        pattern: String,
        /// First value: a number, or a single letter for a letter range.
        #[arg(long, default_value = "1")]
        from: String,
        /// Last value.
        #[arg(long)]
        to: String,
        /// Zero-pad numbers to this many digits.
        #[arg(long, default_value_t = 0)]
        width: usize,
        /// Only print the addresses.
        #[arg(long)]
        dry_run: bool,
        /// Add to the scheduled queue without starting.
        #[arg(long)]
        later: bool,
    },
    /// Queue every URL in a text file (one per line, `#` comments) or an IDM `.ef2` export;
    /// `-` reads stdin.
    Import {
        /// The list or `.ef2` file; `-` reads stdin.
        file: String,
        /// Folder to save into (default: reIDM's download folder).
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// Add to the scheduled queue without starting.
        #[arg(long)]
        later: bool,
    },
    /// Write the current list to a file (`-` for stdout): plain URLs, or IDM's `.ef2` format.
    Export {
        /// Where to write; `-` writes to stdout.
        #[arg(default_value = "-")]
        file: String,
        /// `txt` or `ef2`; by default taken from the file's extension.
        #[arg(long)]
        format: Option<String>,
    },
    /// Show or set the global speed limit: `reidm limit 500K`, `reidm limit off`.
    Limit {
        /// A rate like `800K`, `2M` or `1.5MB` (per second), `on`, or `off`.
        rate: Option<String>,
    },
    /// Start or stop the scheduled queue now.
    Queue {
        /// `start` or `stop`.
        action: String,
    },
    /// Give an unfinished download a new address for the same file (an expired link, a mirror).
    Refresh {
        /// The download's number, as `reidm list` shows it.
        id: u64,
        /// The file's new address.
        url: String,
    },
    /// Start a download again from the first byte.
    Restart {
        /// The download's number, as `reidm list` shows it.
        id: u64,
    },
    /// Print where reIDM's browser extension is, for the browser's "Load unpacked".
    Extension,
    /// Scan a page for downloadable files without downloading anything.
    Grab {
        url: String,
        /// 0 = only this page; 1 = also follow its links once.
        #[arg(short, long, default_value_t = 0)]
        depth: u32,
        /// Extensions to keep, comma separated (default: the built-in list).
        #[arg(short = 't', long)]
        types: Option<String>,
        /// Keep only links on the same host as the page.
        #[arg(long)]
        same_host: bool,
        /// Keep only URLs containing this text (repeatable).
        #[arg(long)]
        include: Vec<String>,
        /// Drop URLs containing this text (repeatable).
        #[arg(long)]
        exclude: Vec<String>,
        /// Ask each server for the file size as well.
        #[arg(long)]
        sizes: bool,
        /// Write the found URLs to a file (`-` for stdout). Default: print a table.
        #[arg(long)]
        export: Option<String>,
        /// Queue everything that was found.
        #[arg(long)]
        add: bool,
    },
    /// List downloads known to the daemon.
    List,
    /// Pause a download.
    Pause {
        /// The download's number, as `reidm list` shows it.
        id: u64,
    },
    /// Resume a paused or failed download.
    Resume {
        /// The download's number, as `reidm list` shows it.
        id: u64,
    },
    /// Remove a download from the list (add --purge to delete its files).
    Rm {
        /// The download's number, as `reidm list` shows it.
        id: u64,
        /// Also delete the file, finished or partial.
        #[arg(long)]
        purge: bool,
    },
    /// Open the web UI in your browser (starts the daemon if needed).
    Ui,
    /// Check GitHub for a newer release.
    UpdateCheck,
    /// Register the native-messaging hosts (com.zektopic.reidm for reIDM's extension,
    /// com.tonec.idm for Tonec's) with the installed browsers.
    InstallHost,
    /// Remove the native-messaging host registrations.
    UninstallHost,
    /// Make a local copy of the official IDM extension usable on this OS (see docs/EXTENSION.md).
    PatchExtension {
        /// The unpacked official extension.
        src: PathBuf,
        /// Where to write the patched copy (a new folder).
        dst: PathBuf,
    },
}

/// The time now, as unix seconds.
fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let (mut v, mut i) = (n as f64, 0);
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}

/// The daemon could not be reached at all (not running, or its token file is missing), as
/// opposed to answering with an error. Only this case may fall back to reading files directly.
#[derive(Debug)]
struct Unreachable(String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unreachable {}

fn unreachable(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Unreachable>().is_some()
}

/// The daemon's API token, as published in its data directory. A local, same-user client can
/// already read that file, which is why the token is stored there.
fn api_token() -> Result<String> {
    let path = default_data_dir().join("api-token");
    let t = std::fs::read_to_string(&path)
        .map_err(|_| Unreachable(format!("reIDM's service is not running (no {}); start it with `reidm daemon`", path.display())))?
        .trim()
        .to_string();
    if t.is_empty() {
        bail!("the daemon's API token is empty; restart `reidm daemon`");
    }
    Ok(t)
}

/// One call to the daemon's API. Answers the body of a successful reply; an error status
/// becomes an error carrying the daemon's own explanation.
async fn api_call(method: reqwest::Method, path: &str, body: Option<serde_json::Value>, timeout: Duration) -> Result<String> {
    let url = format!("http://127.0.0.1:{}{path}", web::ui_port());
    let mut req = reqwest::Client::new().request(method, &url).header("X-Reidm-Token", api_token()?).timeout(timeout);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(|e| {
        if e.is_connect() {
            anyhow::Error::new(Unreachable(format!("cannot reach the daemon at {url} (start it with `reidm daemon`)")))
        } else {
            anyhow::Error::new(e).context(format!("talking to the daemon at {url}"))
        }
    })?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{}", if text.trim().is_empty() { format!("the daemon said {status}") } else { text });
    }
    Ok(text)
}

/// POST JSON to the daemon and decode the reply.
async fn api_post(path: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let text = api_call(reqwest::Method::POST, path, Some(body), Duration::from_secs(120)).await?;
    serde_json::from_str(&text).context("decoding the daemon's reply")
}

/// GET from the daemon and return the body.
async fn api_get(path: &str) -> Result<String> {
    api_call(reqwest::Method::GET, path, None, Duration::from_secs(30)).await
}

/// POST JSON to an endpoint that answers with no body.
async fn api_post_empty(path: &str, body: serde_json::Value) -> Result<()> {
    api_call(reqwest::Method::POST, path, Some(body), Duration::from_secs(120)).await.map(|_| ())
}

/// `500K`, `2M`, `1.5MB`, `1.5MiB`, `800kb/s`, `1048576` -> bytes per second. Units are binary
/// (K = 1024), as IDM uses. A lowercase `b` before `ps` (`8Mbps`, `500kbps`) means bits, the way
/// connection speeds are quoted, and is divided by eight.
fn parse_rate(s: &str) -> Result<u64> {
    let raw = s.trim();
    let bits = raw.ends_with("bps") && !raw.ends_with("Bps");
    let t = raw.to_ascii_lowercase();
    let t = t.trim_end_matches("/s").trim_end_matches("ps").trim_end_matches('b').trim_end_matches('i');
    let (num, mul) = match t.chars().last() {
        Some('k') => (&t[..t.len() - 1], 1024.0),
        Some('m') => (&t[..t.len() - 1], 1024.0 * 1024.0),
        Some('g') => (&t[..t.len() - 1], 1024.0 * 1024.0 * 1024.0),
        _ => (t, 1.0),
    };
    let v: f64 = num.trim().parse().with_context(|| format!("{s:?} is not a rate (try 500K or 2M)"))?;
    if v.is_nan() || v <= 0.0 {
        bail!("a speed limit must be above zero; use `reidm limit off` to remove it");
    }
    let bytes = v * mul / if bits { 8.0 } else { 1.0 };
    if bytes < 1.0 {
        bail!("{s:?} is less than one byte per second");
    }
    Ok(bytes as u64)
}

/// Read a file, or stdin when the path is `-`.
fn read_input(path: &str) -> Result<String> {
    if path == "-" {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).with_context(|| format!("cannot read {path}"))
    }
}

/// Print a per-URL summary of a batch add.
fn report_batch(v: &serde_json::Value) {
    let mut added = 0;
    let mut existing = 0;
    let mut complete = 0;
    let mut failed = 0;
    for r in v["results"].as_array().into_iter().flatten() {
        if let Some(err) = r["error"].as_str() {
            failed += 1;
            eprintln!("  failed:  {}  ({err})", r["url"].as_str().unwrap_or(""));
            continue;
        }
        match r["outcome"].as_str().unwrap_or("") {
            "new" => added += 1,
            "already_complete" => {
                complete += 1;
                println!("  already have: {}", r["url"].as_str().unwrap_or(""));
            }
            _ => {
                existing += 1;
                println!("  resuming:     {}", r["url"].as_str().unwrap_or(""));
            }
        }
    }
    println!("{added} queued, {existing} resumed, {complete} already complete, {failed} failed");
}

/// One line per download, for `reidm list`, from the API's items or from `downloads.json`.
fn print_list(mut items: Vec<serde_json::Value>) {
    if items.is_empty() {
        println!("no downloads");
        return;
    }
    // Numerically: ids are keys of a JSON object on disk, which would otherwise sort 1, 10, 2.
    items.sort_by_key(|it| it["id"].as_u64().unwrap_or(0));
    for it in &items {
        let state = match it["status"]["state"].as_str().unwrap_or("?") {
            // Download Later: stopped, but waiting for the queue rather than for the user.
            "Paused" if it["scheduled"].as_bool() == Some(true) => "Scheduled",
            s => s,
        };
        println!(
            "{:>4}  {:<10} {:>10}/{:<10}  {}",
            it["id"].as_u64().unwrap_or(0),
            state,
            human(it["downloaded"].as_u64().unwrap_or(0)),
            it["size"].as_u64().map(human).unwrap_or("?".into()),
            it["path"].as_str().unwrap_or("")
        );
        if let Some(e) = it["status"]["error"].as_str() {
            println!("      {e}");
        }
        if let Some(n) = it["note"].as_str().filter(|_| state == "Running") {
            println!("      {n}…");
        }
    }
}

/// Where `reidm get` keeps its list for one address: the same folder for the same address and
/// destination, so running the same command again after an interruption resumes it.
fn get_data_dir(url: &str, dir: Option<&std::path::Path>) -> PathBuf {
    // FNV-1a: stable across builds, unlike the standard library's hasher.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes().chain([0]).chain(dir.map(|d| d.to_string_lossy().into_owned()).unwrap_or_default().into_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    std::env::temp_dir().join(format!("reidm-get-{h:016x}"))
}

/// Hold this for as long as the service runs: a second `reidm daemon` for the same data folder
/// would otherwise load the same `downloads.json`, and the two would overwrite each other's
/// changes.
fn single_instance(data: &std::path::Path) -> Result<std::fs::File> {
    let path = data.join("daemon.lock");
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).with_context(|| format!("cannot create {}", path.display()))?;
    match f.try_lock() {
        Ok(()) => Ok(f),
        Err(std::fs::TryLockError::WouldBlock) => bail!(
            "reIDM's service is already running for {} (it holds {}). Use that one, or stop it first.",
            data.display(),
            path.display()
        ),
        Err(std::fs::TryLockError::Error(e)) => Err(e).with_context(|| format!("cannot lock {}", path.display())),
    }
}

/// Resolves when the system asks the service to stop (logout, `systemctl stop`, `kill`).
async fn terminated() {
    #[cfg(unix)]
    {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
            return;
        }
    }
    std::future::pending::<()>().await
}

async fn run_daemon(connections: usize, dir: Option<PathBuf>, no_idm_port: bool) -> Result<()> {
    let data = default_data_dir();
    std::fs::create_dir_all(&data).with_context(|| format!("cannot create reIDM's data folder {}", data.display()))?;
    let _lock = single_instance(&data)?;

    // Both ports before anything else: a service without its API is invisible to the window
    // and the extension, which would wait for it forever, so losing either is fatal.
    let ui_port = web::ui_port();
    let ui = tokio::net::TcpListener::bind(("127.0.0.1", ui_port)).await.with_context(|| {
        format!("cannot serve reIDM's API on 127.0.0.1:{ui_port}: another program is using that port. Close it, then start reIDM again")
    })?;
    let local_port = ws::local_port();
    let local = ws::bind(&format!("127.0.0.1:{local_port}"))
        .await
        .with_context(|| format!("cannot serve the extension socket on 127.0.0.1:{local_port}: another program is using that port"))?;

    let mgr = Manager::open(&data, dir.unwrap_or_else(default_download_dir), connections)?;
    mgr.spawn_scheduler();
    // Finished torrents go on sharing where they left off.
    {
        let m = mgr.clone();
        tokio::spawn(async move { m.resume_seeding().await });
    }
    // Keep browser integration current: a fresh copy of the extension for "Load unpacked",
    // and the native host the extension uses to start reIDM. Both are idempotent and only
    // write what changed, so doing it at every start is what keeps them right after an
    // update or a move.
    std::thread::spawn(|| {
        match install::stage_extension() {
            Ok((dir, true)) => tracing::info!("browser extension updated in {}", dir.display()),
            Ok(_) => {}
            Err(e) => tracing::debug!("browser extension not staged: {e:#}"),
        }
        match install::ensure_reidm_host() {
            Ok(changes) => {
                for c in changes {
                    tracing::info!("native host: {c}");
                }
            }
            Err(e) => tracing::debug!("native host not registered: {e:#}"),
        }
    });
    let idm_port = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if !no_idm_port {
        let m = mgr.clone();
        let bound = idm_port.clone();
        tokio::spawn(async move {
            match ws::bind(&format!("127.0.0.1:{}", ws::IDM_PORT)).await {
                Ok(l) => {
                    bound.store(true, std::sync::atomic::Ordering::Relaxed);
                    if let Err(e) = ws::serve(l, m).await {
                        tracing::error!("extension port {} stopped: {e:#}", ws::IDM_PORT);
                    }
                }
                Err(e) => {
                    // On Unix 1001 is privileged, so the usual cause there is the port
                    // number itself rather than a competing download manager.
                    let hint = if e.to_string().contains("os error 13") {
                        "ports below 1024 need privileges on this system"
                    } else {
                        "another download manager may be using it"
                    };
                    tracing::warn!(
                        "not serving the extension's default port {}: {e} ({hint}). \
                         The native-messaging host and `reidm patch-extension` still work — \
                         see docs/EXTENSION.md.",
                        ws::IDM_PORT
                    );
                }
            }
        });
    }

    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let stop = std::sync::Arc::new(stop);
    let app = web::App::new(mgr.clone(), ui_port, idm_port, stop.clone());
    // Keep the computer awake while downloads run, and do the when-done action; *exit* stops
    // the service the way the window's Quit does, after telling the window.
    let exit_app = app.clone();
    mgr.spawn_power(
        std::sync::Arc::new(reidm_core::power::SystemInhibitor),
        std::sync::Arc::new(reidm_core::power::SystemPower),
        std::sync::Arc::new(move || exit_app.begin_exit()),
    );
    let mut web = tokio::spawn(web::serve(ui, app));
    let mut socket = tokio::spawn(ws::serve(local, mgr.clone()));
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminated() => {}
        _ = stopped.wait_for(|s| *s) => {}
        r = &mut web => bail!("reIDM's API stopped: {}", r.map_err(anyhow::Error::from).and_then(|r| r).err().map(|e| format!("{e:#}")).unwrap_or_default()),
        r = &mut socket => bail!("the extension socket stopped: {}", r.map_err(anyhow::Error::from).and_then(|r| r).err().map(|e| format!("{e:#}")).unwrap_or_default()),
    }
    tracing::info!("shutting down");
    // Stop serving and take the token file away; downloads carry on from their last checkpoint
    // at the next start.
    let _ = stop.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(3), web).await;
    // The list is written a moment after each change; make sure the last ones are on disk.
    if !tokio::task::spawn_blocking(move || mgr.flush()).await.unwrap_or(false) {
        tracing::error!("could not save the download list before stopping; the most recent changes may be lost");
    }
    Ok(())
}

/// Stderr always. The daemon also keeps a small log beside its data, where a hang or a failed
/// download can be looked at afterwards — started from the window or a login item, it has no
/// terminal to print to.
fn init_logging(daemon: bool) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "reidm=info,reidm_core=info".into());
    if !daemon {
        tracing_subscriber::fmt().with_env_filter(filter).init();
        return;
    }
    let path = default_data_dir().join("reidm.log");
    let log = logfile::LogFile::new(path);
    let stderr = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    let file = tracing_subscriber::fmt::layer().with_writer(log).with_ansi(false);
    tracing_subscriber::registry().with(filter).with(stderr).with(file).init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_logging(matches!(&cli.cmd, Cmd::Daemon { .. }));
    match cli.cmd {
        Cmd::Daemon { connections, dir, no_idm_port } => run_daemon(connections, dir, no_idm_port).await?,
        Cmd::Get { url, connections, dir, output, referer, cookie } => {
            let data = get_data_dir(&url, dir.as_deref());
            let mgr = Manager::open(&data, dir.clone().unwrap_or_else(default_download_dir), connections)?;
            let id = mgr.add(Request { url, filename: output, referer, cookie, dir, ..Default::default() })?;
            loop {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let it = mgr.get(id).context("download vanished")?;
                match &it.status {
                    Status::Done => {
                        eprintln!("\rdone: {}                                   ", it.path.display());
                        // Its list is only there to resume an interrupted run.
                        drop(mgr);
                        let _ = std::fs::remove_dir_all(&data);
                        break;
                    }
                    Status::Failed(e) => bail!("failed: {e}"),
                    Status::Paused => bail!("paused"),
                    _ => {
                        let pct = it.size.filter(|s| *s > 0).map(|s| it.downloaded * 100 / s);
                        eprint!("\r{} / {} {}", human(it.downloaded), it.size.map(human).unwrap_or("?".into()), pct.map(|p| format!("({p}%)")).unwrap_or_default());
                    }
                }
            }
        }
        Cmd::Add { checksum: Some(_), video: Some(_), .. } => bail!("--checksum is for files; a video's file is made by yt-dlp, and has no published checksum to check"),
        Cmd::Add { urls, referer, output, later, video: None, checksum: Some(sum) } => {
            let [url] = urls.as_slice() else { bail!("--checksum makes sense for a single address only") };
            let sum = reidm_core::Checksum::parse(&sum)?;
            let body = serde_json::json!({ "url": url, "referer": referer, "filename": output, "later": later, "checksum": sum });
            let v = api_post("/api/downloads", body).await?;
            report_batch(&serde_json::json!({ "results": [{ "url": url, "outcome": v["outcome"] }] }));
        }
        Cmd::WhenDone { action } => {
            let v: serde_json::Value = match action {
                None => serde_json::from_str(&api_get("/api/when-done").await?)?,
                Some(a) => {
                    let Some(a) = reidm_core::WhenDone::from_name(&a) else { bail!("use none, exit, sleep, hibernate or shutdown") };
                    api_post("/api/when-done", serde_json::json!({ "action": a })).await?
                }
            };
            let action = v["action"].as_str().unwrap_or("none");
            match v["fires_at"].as_u64() {
                _ if action == "none" => println!("when everything is done: nothing"),
                Some(t) => println!("when everything is done: {action}, in {}s (cancel with `reidm when-done none`)", t.saturating_sub(unix_now())),
                None => println!("when everything is done: {action}"),
            }
        }
        Cmd::Add { urls, referer, output, later, video: Some(format), checksum: None } => {
            if output.is_some() && urls.len() > 1 {
                bail!("--output only makes sense for a single URL");
            }
            if !reidm_core::valid_format(&format) {
                bail!("not a yt-dlp format selector: {format:?}");
            }
            let mut results = Vec::new();
            for url in urls {
                // Without --output, yt-dlp names the file after the video's title.
                let body = serde_json::json!({ "url": url, "referer": referer, "filename": output, "later": later, "media": { "format": format } });
                results.push(match api_post("/api/downloads", body).await {
                    Ok(v) => serde_json::json!({ "url": url, "outcome": v["outcome"] }),
                    Err(e) => serde_json::json!({ "url": url, "error": format!("{e:#}") }),
                });
            }
            report_batch(&serde_json::json!({ "results": results }));
        }
        Cmd::Add { urls, referer, output, later, video: None, checksum: None } => {
            if output.is_some() && urls.len() > 1 {
                bail!("--output only makes sense for a single URL");
            }
            // `.torrent` files on this disk go to the service whole; the rest are addresses.
            let (files, urls): (Vec<String>, Vec<String>) = urls.into_iter().partition(|u| u.to_ascii_lowercase().ends_with(".torrent") && std::path::Path::new(u).is_file());
            let mut results = Vec::new();
            for f in files {
                use base64::Engine;
                let data = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&f).with_context(|| format!("cannot read {f}"))?);
                let body = serde_json::json!({ "url": "", "later": later, "torrent": { "data": data } });
                results.push(match api_post("/api/downloads", body).await {
                    Ok(v) => serde_json::json!({ "url": f, "outcome": v["outcome"] }),
                    Err(e) => serde_json::json!({ "url": f, "error": format!("{e:#}") }),
                });
            }
            if urls.is_empty() {
                report_batch(&serde_json::json!({ "results": results }));
                return Ok(());
            }
            // `referer` applies to the whole batch; `output` (a file name) is only meaningful
            // for one URL.
            let mut body = serde_json::json!({ "urls": urls, "later": later });
            if let Some(r) = referer {
                body["referer"] = serde_json::Value::String(r);
            }
            if let Some(o) = output {
                body["filename"] = serde_json::Value::String(o);
            }
            let mut v = api_post("/api/downloads/batch", body).await?;
            if let Some(r) = v["results"].as_array_mut() {
                r.extend(results);
            }
            report_batch(&v);
        }
        Cmd::Batch { pattern, from, to, width, dry_run, later } => {
            use reidm_core::lists::{expand_pattern, Sequence};
            let seq = match (from.parse::<u64>(), to.parse::<u64>()) {
                (Ok(from), Ok(to)) => Sequence::Numbers { from, to, width },
                _ => {
                    let one = |s: &str| {
                        let mut c = s.chars();
                        match (c.next(), c.next()) {
                            (Some(ch), None) => Ok(ch),
                            _ => Err(anyhow::anyhow!("{s:?} is neither a number nor a single letter")),
                        }
                    };
                    Sequence::Letters { from: one(&from)?, to: one(&to)? }
                }
            };
            let urls = expand_pattern(&pattern, &seq)?;
            if dry_run {
                for u in &urls {
                    println!("{u}");
                }
                return Ok(());
            }
            println!("queueing {} URL(s)", urls.len());
            let v = api_post("/api/downloads/batch", serde_json::json!({ "urls": urls, "later": later })).await?;
            report_batch(&v);
        }
        Cmd::Import { file, dir, later } => {
            let text = read_input(&file)?;
            let mut body = if reidm_core::lists::looks_like_ef2(&text) {
                let items = reidm_core::lists::parse_ef2(&text);
                if items.is_empty() {
                    bail!("no downloads found in {file}");
                }
                println!("importing {} download(s) from an IDM export", items.len());
                serde_json::json!({ "items": items })
            } else {
                let urls = reidm_core::grabber::parse_url_list(&text);
                if urls.is_empty() {
                    bail!("no http(s) URLs found in {file}");
                }
                println!("importing {} URL(s)", urls.len());
                serde_json::json!({ "urls": urls })
            };
            body["later"] = serde_json::Value::Bool(later);
            if let Some(d) = dir {
                body["dir"] = serde_json::Value::String(d.to_string_lossy().into_owned());
            }
            let v = api_post("/api/downloads/batch", body).await?;
            report_batch(&v);
        }
        Cmd::Export { file, format } => {
            let format = format.unwrap_or_else(|| if file.to_ascii_lowercase().ends_with(".ef2") { "ef2".into() } else { "txt".into() });
            let text = match api_get(&format!("/api/export?format={format}")).await {
                Ok(t) => t,
                // No daemon: read the list straight from disk. Any other failure (a refused
                // token, a full disk) is reported rather than papered over.
                Err(e) if !unreachable(&e) => return Err(e),
                Err(_) => {
                    let path = default_data_dir().join("downloads.json");
                    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("no downloads yet ({})", path.display()))?)?;
                    let items: Vec<&serde_json::Value> = v["items"].as_object().into_iter().flatten().map(|(_, it)| it).collect();
                    if format == "ef2" {
                        let entries: Vec<reidm_core::lists::Entry> = items
                            .iter()
                            .filter_map(|it| {
                                Some(reidm_core::lists::Entry {
                                    url: it["request"]["url"].as_str()?.to_string(),
                                    referer: it["request"]["referer"].as_str().map(str::to_string),
                                    cookie: None,
                                    user_agent: it["request"]["user_agent"].as_str().map(str::to_string),
                                    post_data: None,
                                })
                            })
                            .collect();
                        reidm_core::lists::write_ef2(&entries)
                    } else {
                        let mut urls: Vec<String> = items.iter().filter_map(|it| it["request"]["url"].as_str().map(str::to_string)).collect();
                        urls.sort();
                        urls.dedup();
                        reidm_core::grabber::url_list_text(&urls)
                    }
                }
            };
            match file.as_str() {
                "-" => print!("{text}"),
                f => {
                    std::fs::write(f, &text)?;
                    println!("wrote {f}");
                }
            }
        }
        Cmd::Limit { rate } => {
            let body = match rate.as_deref().map(str::trim) {
                None => {
                    let s: serde_json::Value = serde_json::from_str(&api_get("/api/settings").await?)?;
                    let on = s["limit_enabled"].as_bool().unwrap_or(false);
                    let r = s["speed_limit"].as_u64().unwrap_or(0);
                    if on && r > 0 {
                        println!("speed limit: {}/s", human(r));
                    } else {
                        println!("speed limit: off{}", if r > 0 { format!(" (set to {}/s when on)", human(r)) } else { String::new() });
                    }
                    return Ok(());
                }
                Some("off") => serde_json::json!({ "limit_enabled": false }),
                Some("on") => serde_json::json!({ "limit_enabled": true }),
                Some(r) => serde_json::json!({ "limit_enabled": true, "speed_limit": parse_rate(r)? }),
            };
            api_post_empty("/api/settings", body).await?;
            println!("ok");
        }
        Cmd::Queue { action } => {
            if action != "start" && action != "stop" {
                bail!("use `reidm queue start` or `reidm queue stop`");
            }
            let v = api_post(&format!("/api/queue/{action}"), serde_json::json!({})).await?;
            println!("{} download(s) {}", v["count"].as_u64().unwrap_or(0), if action == "start" { "started" } else { "stopped" });
        }
        Cmd::Refresh { id, url } => {
            api_post_empty(&format!("/api/downloads/{id}/refresh"), serde_json::json!({ "url": url })).await?;
            println!("download #{id} continues from {url}");
        }
        Cmd::Restart { id } => {
            api_post_empty(&format!("/api/downloads/{id}/restart"), serde_json::json!({})).await?;
            println!("download #{id} restarted");
        }
        Cmd::Extension => {
            let (dir, _) = install::stage_extension()?;
            println!("{}", dir.display());
            eprintln!(
                "Load it once: open chrome://extensions (or edge://, brave://), turn on Developer mode, \
                 choose \"Load unpacked\" and pick the folder above. reIDM keeps that folder up to date."
            );
        }
        Cmd::Grab { url, depth, types, same_host, include, exclude, sizes, export, add } => {
            let mut opts = reidm_core::grabber::Options {
                depth,
                same_host_only: same_host,
                include,
                exclude,
                sizes,
                ..Default::default()
            };
            if let Some(t) = types {
                opts.types = t.split(',').map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|s| !s.is_empty()).collect();
            }
            if opts.types.is_empty() {
                opts.types = reidm_core::grabber::COMMON_TYPES.iter().map(|s| s.to_string()).collect();
            }
            eprintln!("scanning {url} …");
            let files = reidm_core::grabber::grab_page(&url, &opts).await?;
            if files.is_empty() {
                println!("no downloadable files found");
                return Ok(());
            }
            match export.as_deref() {
                Some("-") | None => {
                    for f in &files {
                        match f.size {
                            Some(sz) => println!("{:>12}  {}", human(sz), f.url),
                            None => println!("{:>12}  {}", "-", f.url),
                        }
                    }
                }
                Some(path) => {
                    let urls: Vec<String> = files.iter().map(|f| f.url.clone()).collect();
                    std::fs::write(path, reidm_core::grabber::url_list_text(&urls))?;
                    println!("wrote {} URL(s) to {path}", urls.len());
                }
            }
            if add {
                let urls: Vec<String> = files.iter().map(|f| f.url.clone()).collect();
                println!("queueing {} file(s)…", urls.len());
                let v = api_post("/api/downloads/batch", serde_json::json!({ "urls": urls })).await?;
                report_batch(&v);
            }
        }
        Cmd::List => {
            let items: Vec<serde_json::Value> = match api_get("/api/downloads").await {
                Ok(text) => serde_json::from_str(&text).context("decoding the daemon's list")?,
                // No daemon: the list as it was last saved.
                Err(e) if unreachable(&e) => {
                    let path = default_data_dir().join("downloads.json");
                    let Ok(bytes) = std::fs::read(&path) else {
                        println!("no downloads");
                        return Ok(());
                    };
                    let v: serde_json::Value = serde_json::from_slice(&bytes).with_context(|| format!("{} is not a download list", path.display()))?;
                    v["items"].as_object().into_iter().flatten().map(|(_, it)| it.clone()).collect()
                }
                Err(e) => return Err(e),
            };
            print_list(items);
        }
        Cmd::Pause { id } => {
            api_post_empty(&format!("/api/downloads/{id}/pause"), serde_json::json!({})).await?;
            println!("download #{id} paused");
        }
        Cmd::Resume { id } => {
            api_post_empty(&format!("/api/downloads/{id}/resume"), serde_json::json!({})).await?;
            println!("download #{id} resumed");
        }
        Cmd::Rm { id, purge } => {
            api_call(reqwest::Method::DELETE, &format!("/api/downloads/{id}?purge={purge}"), None, Duration::from_secs(60)).await?;
            println!("download #{id} removed{}", if purge { ", and its file deleted" } else { "" });
        }
        Cmd::Ui => {
            let up = || std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{}", web::ui_port()).parse().unwrap(), Duration::from_millis(300)).is_ok();
            if !up() {
                launch::spawn_daemon()?;
                for _ in 0..40 {
                    if up() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
            launch::open_url(&format!("http://127.0.0.1:{}/", web::ui_port()));
        }
        Cmd::UpdateCheck => {
            let current = env!("CARGO_PKG_VERSION");
            let token = reidm_core::update::token_from_env();
            match reidm_core::update::check(reidm_core::update::REPO, current, token.as_deref()) {
                Ok(Some(rel)) => {
                    println!("reIDM {} is available (you have {current}).", rel.version());
                    println!("{}", rel.html_url);
                    if let Some(a) = rel.asset_for(std::env::consts::OS) {
                        println!("your build: {} ({})", a.name, a.browser_download_url);
                    }
                    if !rel.body.trim().is_empty() {
                        println!("\n{}", rel.body.trim());
                    }
                }
                Ok(None) => println!("reIDM {current} is up to date."),
                Err(e) => {
                    eprintln!("could not check for updates: {e:#}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::InstallHost => install::install()?,
        Cmd::UninstallHost => install::uninstall()?,
        Cmd::PatchExtension { src, dst } => patch_extension(&src, &dst)?,
    }
    Ok(())
}

/// The shipped extension (v6.43.1) refuses to start unless `runtime.getPlatformInfo().os == "win"`
/// and dials the privileged port 1001. This rewrites a *local, user-supplied* copy so it can talk to
/// reIDM on Linux/macOS. Nothing from the vendor's extension is bundled with reIDM.
fn patch_extension(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    fn copy_dir(a: &std::path::Path, b: &std::path::Path) -> Result<()> {
        std::fs::create_dir_all(b)?;
        for e in std::fs::read_dir(a)? {
            let e = e?;
            let t = b.join(e.file_name());
            if e.file_type()?.is_dir() {
                copy_dir(&e.path(), &t)?;
            } else {
                std::fs::copy(e.path(), t)?;
            }
        }
        Ok(())
    }
    if !src.join("manifest.json").exists() {
        bail!("{} is not an unpacked extension (no manifest.json)", src.display());
    }
    // `std::fs::copy` truncates its destination before writing, so `reidm patch-extension X X`
    // would erase every file as it went. Refuse rather than destroy the user's copy.
    fn absolute(p: &std::path::Path) -> std::path::PathBuf {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| p.to_path_buf())
        }
    }
    let (a, b) = (absolute(src), absolute(dst));
    if a == b {
        bail!("source and destination are the same directory ({}); nothing was changed", a.display());
    }
    if b.starts_with(&a) {
        bail!("destination {} is inside the source directory {}", b.display(), a.display());
    }
    copy_dir(src, dst)?;
    // Drop Chrome's integrity file: it would flag the modified script as corrupt.
    let _ = std::fs::remove_dir_all(dst.join("_metadata"));
    let bg = dst.join("background.js");
    let mut js = std::fs::read_to_string(&bg).context("background.js missing")?;
    const GATE: &str = r#"if(a&&a.os!="win")return Gb(!1);"#;
    const PORTS: &str = r#"La=["127.0.0.1:1001","0.1.0.1:1001"]"#;
    if !js.contains(GATE) || !js.contains(PORTS) {
        bail!("this extension version does not match the known patch points (tested with 6.43.1)");
    }
    js = js.replacen(GATE, "", 1).replacen(PORTS, &format!(r#"La=["127.0.0.1:{}"]"#, ws::DEFAULT_LOCAL_PORT), 1);
    std::fs::write(&bg, js)?;
    println!("patched extension written to {}\nLoad it via chrome://extensions -> Developer mode -> Load unpacked.", dst.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{get_data_dir, parse_rate, single_instance, Cli, Cmd};
    use clap::Parser;

    #[test]
    fn checksums_and_when_done_are_read_from_the_command_line() {
        let sum = format!("sha256:{}", "a".repeat(64));
        match Cli::try_parse_from(["reidm", "add", "--checksum", &sum, "https://example.com/f.iso"]).unwrap().cmd {
            Cmd::Add { checksum: Some(c), urls, .. } => {
                assert_eq!(urls, ["https://example.com/f.iso"]);
                assert_eq!(reidm_core::Checksum::parse(&c).unwrap().value, "a".repeat(64));
            }
            _ => panic!("not an add with a checksum"),
        }
        assert!(matches!(Cli::try_parse_from(["reidm", "when-done", "shutdown"]).unwrap().cmd, Cmd::WhenDone { action: Some(a) } if a == "shutdown"));
        assert!(matches!(Cli::try_parse_from(["reidm", "when-done"]).unwrap().cmd, Cmd::WhenDone { action: None }));
    }

    #[test]
    fn a_second_service_for_the_same_folder_is_refused() {
        let d = std::env::temp_dir().join(format!("reidm-lock-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let first = single_instance(&d).unwrap();
        let err = single_instance(&d).unwrap_err().to_string();
        assert!(err.contains("already running"), "{err}");
        drop(first);
        single_instance(&d).expect("free again once the first has stopped");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn get_resumes_in_the_same_folder() {
        let a = get_data_dir("https://x/a.iso", None);
        assert_eq!(a, get_data_dir("https://x/a.iso", None));
        assert_ne!(a, get_data_dir("https://x/b.iso", None));
        assert_ne!(a, get_data_dir("https://x/a.iso", Some(std::path::Path::new("/tmp"))));
    }

    #[test]
    fn rates() {
        assert_eq!(parse_rate("500K").unwrap(), 500 * 1024);
        assert_eq!(parse_rate("2M").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_rate("1.5MB").unwrap(), 1_572_864);
        assert_eq!(parse_rate("800kb/s").unwrap(), 800 * 1024);
        assert_eq!(parse_rate("4096").unwrap(), 4096);
        assert_eq!(parse_rate("1.5MiB").unwrap(), 1_572_864);
        assert_eq!(parse_rate("8Mbps").unwrap(), 1024 * 1024, "megabits");
        assert_eq!(parse_rate("8MBps").unwrap(), 8 * 1024 * 1024, "megabytes");
        assert_eq!(parse_rate("500kbps").unwrap(), 500 * 1024 / 8);
        assert!(parse_rate("0").is_err());
        assert!(parse_rate("fast").is_err());
    }
}
