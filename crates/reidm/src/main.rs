mod install;
mod launch;
mod web;
mod session;
mod ws;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use reidm_core::{default_data_dir, default_download_dir, Manager, Request, Status};
use reidm_proto::Message;
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as Ws;

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
        url: String,
        #[arg(short, long, default_value_t = 8)]
        connections: usize,
        #[arg(short, long)]
        dir: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<String>,
        #[arg(long)]
        referer: Option<String>,
        #[arg(long)]
        cookie: Option<String>,
    },
    /// Queue one or more URLs in the running daemon. Re-adding a URL resumes it.
    Add {
        #[arg(required = true)]
        urls: Vec<String>,
        #[arg(long)]
        referer: Option<String>,
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Queue every URL in a text file (one per line, `#` comments); `-` reads stdin.
    Import {
        file: String,
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },
    /// Write the URLs of the current list to a text file (`-` for stdout).
    Export {
        #[arg(default_value = "-")]
        file: String,
    },
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
    Pause { id: u64 },
    Resume { id: u64 },
    /// Remove a download from the list (add --purge to delete its files).
    Rm { id: u64, #[arg(long)] purge: bool },
    /// Open the web UI in your browser (starts the daemon if needed).
    Ui,
    /// Check GitHub for a newer release.
    UpdateCheck,
    /// Register the native-messaging host (com.tonec.idm) with installed browsers.
    InstallHost,
    UninstallHost,
    /// Make a local copy of the official IDM extension usable on this OS (see docs/EXTENSION.md).
    PatchExtension { src: PathBuf, dst: PathBuf },
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

async fn send_to_daemon(msgs: Vec<Message>) -> Result<Vec<Message>> {
    let url = format!("ws://127.0.0.1:{}/", ws::LOCAL_PORT);
    let (mut s, _) = tokio_tungstenite::connect_async(&url)
        .await
        .with_context(|| format!("cannot reach the reidm daemon at {url} (start it with `reidm daemon`)"))?;
    let mut replies = Vec::new();
    for m in msgs {
        s.send(Ws::Binary(m.encode().into_bytes())).await?;
        if m.flags & 1 != 0 {
            if let Ok(Some(Ok(Ws::Binary(b)))) = tokio::time::timeout(Duration::from_secs(5), s.next()).await {
                if let Ok((r, _)) = Message::decode(&b) {
                    replies.push(r);
                }
            }
        }
    }
    let _ = s.close(None).await;
    Ok(replies)
}

/// The daemon's API token, as published in its data directory. A local, same-user client can
/// already read that file, which is why the token is stored there.
fn api_token() -> Result<String> {
    let path = default_data_dir().join("api-token");
    let t = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {} — is the daemon running?", path.display()))?
        .trim()
        .to_string();
    if t.is_empty() {
        bail!("the daemon's API token is empty; restart `reidm daemon`");
    }
    Ok(t)
}

/// POST JSON to the daemon and decode the reply.
async fn api_post(path: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let url = format!("http://127.0.0.1:{}{path}", web::UI_PORT);
    let resp = reqwest::Client::new()
        .post(&url)
        .header("X-Reidm-Token", api_token()?)
        .json(&body)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .with_context(|| format!("cannot reach the daemon at {url} (start it with `reidm daemon`)"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("daemon said {status}: {text}");
    }
    serde_json::from_str(&text).context("decoding the daemon's reply")
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

fn ctl(ty: u32, id: u64, extra: Option<i64>) -> Message {
    let mut m = Message::new(1, ty, 1, 1).with_args([id as i64]);
    if let Some(e) = extra {
        m.args.push(reidm_proto::Arg::Int(e));
    }
    m
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "reidm=info,reidm_core=info".into()))
        .init();
    match Cli::parse().cmd {
        Cmd::Daemon { connections, dir, no_idm_port } => {
            let mgr = Manager::open(&default_data_dir(), dir.unwrap_or_else(default_download_dir), connections)?;
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
            {
                let m = mgr.clone();
                let bound = idm_port.clone();
                tokio::spawn(async move {
                    if let Err(e) = web::serve(m, bound).await {
                        tracing::error!("web UI unavailable: {e} (is another reidm running?)");
                    }
                });
            }
            let local = format!("127.0.0.1:{}", ws::LOCAL_PORT);
            tokio::select! {
                r = ws::listen(&local, mgr) => r?,
                _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
            }
        }
        Cmd::Get { url, connections, dir, output, referer, cookie } => {
            let mgr = Manager::open(&std::env::temp_dir().join(format!("reidm-get-{}", std::process::id())), dir.clone().unwrap_or_else(default_download_dir), connections)?;
            let id = mgr.add(Request { url, filename: output, referer, cookie, dir, ..Default::default() })?;
            loop {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let it = mgr.get(id).context("download vanished")?;
                match &it.status {
                    Status::Done => {
                        eprintln!("\rdone: {}                                   ", it.path.display());
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
        Cmd::Add { urls, referer, output } => {
            if output.is_some() && urls.len() > 1 {
                bail!("--output only makes sense for a single URL");
            }
            // `referer` applies to the whole batch; `output` (a file name) is only meaningful
            // for one URL. Both go through the single-URL path when given.
            if referer.is_some() || output.is_some() {
                let mut body = serde_json::json!({ "urls": urls });
                if let Some(r) = referer {
                    body["referer"] = serde_json::Value::String(r);
                }
                if let Some(o) = output {
                    body["filename"] = serde_json::Value::String(o);
                }
                let v = api_post("/api/downloads/batch", body).await?;
                report_batch(&v);
            } else {
                let v = api_post("/api/downloads/batch", serde_json::json!({ "urls": urls })).await?;
                report_batch(&v);
            }
        }
        Cmd::Import { file, dir } => {
            let text = read_input(&file)?;
            let urls = reidm_core::grabber::parse_url_list(&text);
            if urls.is_empty() {
                bail!("no http(s) URLs found in {file}");
            }
            println!("importing {} URL(s)", urls.len());
            let mut body = serde_json::json!({ "urls": urls });
            if let Some(d) = dir {
                body["dir"] = serde_json::Value::String(d.to_string_lossy().into_owned());
            }
            let v = api_post("/api/downloads/batch", body).await?;
            report_batch(&v);
        }
        Cmd::Export { file } => {
            let path = default_data_dir().join("downloads.json");
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("no downloads yet ({})", path.display()))?)?;
            let mut urls: Vec<String> = v["items"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(_, it)| it["request"]["url"].as_str().map(str::to_string))
                .collect();
            urls.sort();
            urls.dedup();
            let text = reidm_core::grabber::url_list_text(&urls);
            match file.as_str() {
                "-" => print!("{text}"),
                f => {
                    std::fs::write(f, &text)?;
                    println!("wrote {} URL(s) to {f}", urls.len());
                }
            }
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
            let path = default_data_dir().join("downloads.json");
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("no downloads yet ({})", path.display()))?)?;
            for (id, it) in v["items"].as_object().into_iter().flatten() {
                println!(
                    "{:>4}  {:<10} {:>10}/{:<10}  {}",
                    id,
                    it["status"]["state"].as_str().unwrap_or("?"),
                    human(it["downloaded"].as_u64().unwrap_or(0)),
                    it["size"].as_u64().map(human).unwrap_or("?".into()),
                    it["path"].as_str().unwrap_or("")
                );
            }
        }
        Cmd::Pause { id } => drop(send_to_daemon(vec![ctl(session::T_CTL_PAUSE, id, None)]).await?),
        Cmd::Resume { id } => drop(send_to_daemon(vec![ctl(session::T_CTL_RESUME, id, None)]).await?),
        Cmd::Rm { id, purge } => drop(send_to_daemon(vec![ctl(session::T_CTL_REMOVE, id, Some(purge as i64))]).await?),
        Cmd::Ui => {
            let up = || std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{}", web::UI_PORT).parse().unwrap(), Duration::from_millis(300)).is_ok();
            if !up() {
                launch::spawn_daemon()?;
                for _ in 0..40 {
                    if up() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
            launch::open_url(&format!("http://127.0.0.1:{}/", web::UI_PORT));
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
    js = js.replacen(GATE, "", 1).replacen(PORTS, &format!(r#"La=["127.0.0.1:{}"]"#, ws::LOCAL_PORT), 1);
    std::fs::write(&bg, js)?;
    println!("patched extension written to {}\nLoad it via chrome://extensions -> Developer mode -> Load unpacked.", dst.display());
    Ok(())
}
