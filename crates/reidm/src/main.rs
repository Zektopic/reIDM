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
    /// Queue a URL in the running daemon.
    Add { url: String, #[arg(long)] referer: Option<String>, #[arg(short, long)] output: Option<String> },
    /// List downloads known to the daemon.
    List,
    Pause { id: u64 },
    Resume { id: u64 },
    /// Remove a download from the list (add --purge to delete its files).
    Rm { id: u64, #[arg(long)] purge: bool },
    /// Open the web UI in your browser (starts the daemon if needed).
    Ui,
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
        Cmd::Add { url, referer, output } => {
            let mut m = Message::new(1, 14, 1, 1).with_args([1]).with_str(6, url);
            if let Some(r) = referer {
                m = m.with_str(50, r);
            }
            if let Some(o) = output {
                m = m.with_str(100, o);
            }
            send_to_daemon(vec![m]).await?;
            println!("queued");
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
