//! Downloads that only a site's own player can resolve, handed to yt-dlp.
//!
//! YouTube gives the browser scrambled addresses (`signatureCipher`), or none at all, and the
//! code that unscrambles them changes every few weeks. yt-dlp keeps up with that, so for these
//! downloads reIDM runs yt-dlp instead of its own engine. They still live in reIDM's list,
//! count against *Downloads at the same time*, show progress, and stop and resume (yt-dlp
//! continues its `.part` files).
//!
//! reIDM fetches yt-dlp itself the first time it needs it, from the project's GitHub releases,
//! checks it against the release's published SHA-256 sums, and looks for a newer release once a
//! day. yt-dlp also needs a JavaScript runtime to read YouTube's player: an installed Deno,
//! Node or Bun, or else Deno, fetched and checked the same way. ffmpeg, which joins the
//! separate video and audio streams of higher qualities, is taken from the system (`apt install
//! ffmpeg`, `brew install ffmpeg`); on Windows, where there is rarely one, reIDM fetches
//! yt-dlp's own ffmpeg build.

use super::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tokio::io::AsyncBufReadExt;

/// What to ask yt-dlp for. The request's URL is the page (`https://www.youtube.com/watch?v=…`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Media {
    /// A yt-dlp format selector, e.g. `137+bestaudio/bestvideo[height<=1080]+bestaudio`.
    pub format: String,
}

/// A selector reIDM will pass on: yt-dlp's selector syntax, nothing else. It is an argument of
/// its own (never parsed by a shell), but there is no reason to accept anything wider.
pub fn valid_format(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && s.chars().all(|c| c.is_ascii_alphanumeric() || "+/[]<>=!*_.,-:^$~?".contains(c))
}

const YTDLP_REPO: &str = "https://github.com/yt-dlp/yt-dlp/releases";
const DENO_REPO: &str = "https://github.com/denoland/deno/releases";
const FFMPEG_REPO: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases";
/// How often to look for a newer yt-dlp. YouTube breaks old versions; a day is soon enough.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);
const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

/// The release asset that runs here, if yt-dlp publishes one.
fn ytdlp_asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        // The x64 build also runs on Windows on Arm, emulated.
        ("windows", "x86_64" | "aarch64") => Some("yt-dlp.exe"),
        ("windows", "x86") => Some("yt-dlp_x86.exe"),
        ("macos", _) => Some("yt-dlp_macos"),
        ("linux", "x86_64") => Some("yt-dlp_linux"),
        ("linux", "aarch64") => Some("yt-dlp_linux_aarch64"),
        _ => None,
    }
}

fn tools_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("tools")
}

/// A program on `PATH`, or in the places a Mac's package managers use, which an app started
/// from Finder doesn't have on its `PATH`.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{EXE}");
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if cfg!(target_os = "macos") {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
    }
    dirs.into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// Only one download fetches or updates the tools at a time.
static TOOLS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Try `op` up to three times, pausing longer each time: a connection dropped halfway through
/// a 40 MB download is common enough not to fail a video over.
async fn with_retries<T, F: std::future::Future<Output = Result<T>>>(mut op: impl FnMut() -> F) -> Result<T> {
    let mut last = None;
    for wait in [0, 2, 6] {
        tokio::time::sleep(Duration::from_secs(wait)).await;
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                tracing::debug!("tool download attempt failed: {e:#}");
                last = Some(e);
            }
        }
    }
    Err(last.expect("at least one attempt"))
}

/// The newest release's tag, from where `releases/latest` redirects to, without the API (and
/// its rate limit).
async fn latest_tag(repo: &str) -> Result<String> {
    let c = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("reIDM/", env!("CARGO_PKG_VERSION")))
        .build()?;
    with_retries(|| async {
        let r = c.get(format!("{repo}/latest")).send().await?;
        let status = r.status();
        let loc = r.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()).with_context(|| format!("GitHub answered {status} instead of naming the latest release"))?;
        let tag = loc.rsplit("/tag/").next().filter(|t| !t.is_empty() && !t.contains('/')).context("unexpected release address")?;
        Ok(tag.to_string())
    })
    .await
}

async fn fetch(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    with_retries(|| async {
        let r = http.get(url).send().await?.error_for_status()?;
        Ok(r.bytes().await?.to_vec())
    })
    .await
    .with_context(|| format!("downloading {url}"))
}

/// Download `asset` and check it against the SHA-256 listed for it in `sums_url`.
async fn fetch_verified(http: &reqwest::Client, url: &str, sums_url: &str, asset: &str) -> Result<Vec<u8>> {
    let sums = String::from_utf8_lossy(&fetch(http, sums_url).await?).into_owned();
    let want = expected_hash(&sums, asset).with_context(|| format!("{asset} is not in the release's checksums"))?;
    let bytes = fetch(http, url).await?;
    let got = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if got != want {
        bail!("{asset} did not match its published checksum; not using it");
    }
    Ok(bytes)
}

/// The SHA-256 a checksums file gives for `asset`: from a `hash  name` line, or, in a file
/// for that one asset (Deno publishes PowerShell's `Hash : …` listing for Windows), its only hash.
fn expected_hash(sums: &str, asset: &str) -> Option<String> {
    let is_hash = |t: &str| t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit());
    let named = sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (hash, name) = (it.next()?, it.next()?);
        (is_hash(hash) && name.trim_start_matches('*') == asset).then(|| hash.to_ascii_lowercase())
    });
    named.or_else(|| {
        let hashes: Vec<&str> = sums.split(|c: char| !c.is_ascii_hexdigit()).filter(|t| is_hash(t)).collect();
        (hashes.len() == 1 && !sums.lines().any(|l| l.split_whitespace().nth(1).is_some_and(|n| n.ends_with(".zip") && n != asset)))
            .then(|| hashes[0].to_ascii_lowercase())
    })
}

/// Unpack the entries of a zip archive for which `dest` names a place. Enough of the format for
/// release archives: stored and deflated entries, no zip64.
fn unzip(zip: &[u8], dest: impl Fn(&str) -> Option<PathBuf>) -> Result<usize> {
    use std::io::Read;
    let u16_at = |o: usize| -> Result<u16> { Ok(u16::from_le_bytes(zip.get(o..o + 2).context("truncated zip")?.try_into()?)) };
    let u32_at = |o: usize| -> Result<u32> { Ok(u32::from_le_bytes(zip.get(o..o + 4).context("truncated zip")?.try_into()?)) };
    let floor = zip.len().saturating_sub(22 + 65535);
    let eocd = (floor..zip.len().saturating_sub(21)).rev().find(|&o| zip[o..o + 4] == [0x50, 0x4b, 0x05, 0x06]).context("not a zip archive")?;
    let entries = u16_at(eocd + 10)? as usize;
    let mut at = u32_at(eocd + 16)? as usize;
    let mut written = 0;
    for _ in 0..entries {
        if u32_at(at)? != 0x0201_4b50 {
            bail!("damaged zip directory");
        }
        let method = u16_at(at + 10)?;
        let (csize, usize_) = (u32_at(at + 20)?, u32_at(at + 24)?);
        let (n, e, c) = (u16_at(at + 28)? as usize, u16_at(at + 30)? as usize, u16_at(at + 32)? as usize);
        let local = u32_at(at + 42)? as usize;
        let name = String::from_utf8_lossy(zip.get(at + 46..at + 46 + n).context("truncated zip")?).into_owned();
        at += 46 + n + e + c;
        if name.ends_with('/') || name.contains("..") {
            continue;
        }
        let Some(out) = dest(&name) else { continue };
        if csize == u32::MAX || usize_ == u32::MAX {
            bail!("zip64 archives are not supported");
        }
        let data = local + 30 + u16_at(local + 26)? as usize + u16_at(local + 28)? as usize;
        let raw = zip.get(data..data + csize as usize).context("truncated zip")?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_ as usize);
                flate2::read::DeflateDecoder::new(raw).read_to_end(&mut v)?;
                v
            }
            m => bail!("unsupported zip compression method {m}"),
        };
        install_file(&out, &bytes)?;
        written += 1;
    }
    Ok(written)
}

/// Put `bytes` at `path` as an executable, replacing what is there only once it is complete.
fn install_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("download");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    // On Windows a running yt-dlp can't be replaced; the update then waits for the next check.
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// The yt-dlp to run: reIDM's own copy, fetched or updated as needed; a system one only when
/// reIDM's can't be had (offline on first use, or no build for this system).
pub(crate) async fn ytdlp(http: &reqwest::Client, data_dir: &Path) -> Result<PathBuf> {
    let _one = TOOLS.lock().await;
    let dir = tools_dir(data_dir);
    let path = dir.join(format!("yt-dlp{EXE}"));
    let stamp = dir.join("yt-dlp.version");
    let checked_recently = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|e| e < CHECK_EVERY));
    if path.is_file() && checked_recently {
        return Ok(path);
    }
    let Some(asset) = ytdlp_asset() else {
        return find_on_path("yt-dlp").context("yt-dlp publishes no build for this system; install yt-dlp and put it on the PATH");
    };
    let fetched = async {
        let tag = latest_tag(YTDLP_REPO).await?;
        if path.is_file() && std::fs::read_to_string(&stamp).ok().as_deref().map(str::trim) == Some(tag.as_str()) {
            return Ok(tag); // still the newest
        }
        tracing::info!("fetching yt-dlp {tag}");
        let bytes = fetch_verified(http, &format!("{YTDLP_REPO}/download/{tag}/{asset}"), &format!("{YTDLP_REPO}/download/{tag}/SHA2-256SUMS"), asset).await?;
        std::fs::create_dir_all(&dir)?;
        install_file(&path, &bytes)?;
        anyhow::Ok(tag)
    }
    .await;
    match fetched {
        Ok(tag) => {
            let _ = std::fs::write(&stamp, tag);
            Ok(path)
        }
        // Keep working with what there is; try again at the next download.
        Err(e) if path.is_file() => {
            tracing::warn!("could not check for a newer yt-dlp: {e:#}");
            Ok(path)
        }
        Err(e) => find_on_path("yt-dlp").ok_or_else(|| e.context("could not get yt-dlp, which reIDM needs for this download")),
    }
}

/// ffmpeg for joining video and audio: the system's, or on Windows one reIDM fetches.
pub(crate) async fn ffmpeg(http: &reqwest::Client, data_dir: &Path) -> Result<Option<PathBuf>> {
    let _one = TOOLS.lock().await;
    let own = tools_dir(data_dir).join("ffmpeg");
    if own.join(format!("ffmpeg{EXE}")).is_file() {
        return Ok(Some(own));
    }
    if let Some(p) = find_on_path("ffmpeg") {
        return Ok(Some(p));
    }
    if !cfg!(windows) {
        return Ok(None);
    }
    // The "shared" build: half the size of the static one, its DLLs beside ffmpeg.exe.
    let asset = if std::env::consts::ARCH == "aarch64" { "ffmpeg-master-latest-winarm64-gpl-shared.zip" } else { "ffmpeg-master-latest-win64-gpl-shared.zip" };
    tracing::info!("fetching ffmpeg ({asset})");
    let zip = fetch_verified(http, &format!("{FFMPEG_REPO}/download/latest/{asset}"), &format!("{FFMPEG_REPO}/download/latest/checksums.sha256"), asset).await?;
    std::fs::create_dir_all(&own)?;
    // From its `bin` folder: ffmpeg.exe, ffprobe.exe and the libraries they load (not ffplay).
    unzip(&zip, |name| {
        let (dir, file) = name.rsplit_once('/')?;
        let wanted = file.eq_ignore_ascii_case("ffmpeg.exe") || file.eq_ignore_ascii_case("ffprobe.exe") || file.to_ascii_lowercase().ends_with(".dll");
        ((dir.ends_with("/bin") || dir == "bin") && wanted).then(|| own.join(file))
    })?;
    if !own.join("ffmpeg.exe").is_file() {
        bail!("{asset} did not contain ffmpeg.exe");
    }
    Ok(Some(own))
}

/// The JavaScript runtime yt-dlp needs to read YouTube's player, as yt-dlp's `--js-runtimes`
/// value (`deno:/path/to/deno`): reIDM's own Deno, an installed Deno, Node or Bun, or else a
/// Deno fetched now. `None` when there is none to be had: yt-dlp then finds few formats or none.
pub(crate) async fn js_runtime(http: &reqwest::Client, data_dir: &Path) -> Option<String> {
    let _one = TOOLS.lock().await;
    let own = tools_dir(data_dir).join(format!("deno{EXE}"));
    let found = if own.is_file() { Some(("deno", own.clone())) } else { ["deno", "node", "bun"].into_iter().find_map(|n| find_on_path(n).map(|p| (n, p))) };
    if let Some((name, path)) = found {
        return Some(format!("{name}:{}", path.display()));
    }
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "x86_64-pc-windows-msvc",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", _) => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    };
    let fetched = async {
        let tag = latest_tag(DENO_REPO).await?;
        let asset = format!("deno-{target}.zip");
        tracing::info!("fetching Deno {tag}, which yt-dlp needs for YouTube");
        let zip = fetch_verified(http, &format!("{DENO_REPO}/download/{tag}/{asset}"), &format!("{DENO_REPO}/download/{tag}/{asset}.sha256sum"), &asset).await?;
        std::fs::create_dir_all(tools_dir(data_dir))?;
        let exe = format!("deno{EXE}");
        unzip(&zip, |name| (name == exe).then(|| own.clone()))?;
        anyhow::ensure!(own.is_file(), "{asset} did not contain {exe}");
        anyhow::Ok(())
    }
    .await;
    match fetched {
        Ok(()) => Some(format!("deno:{}", own.display())),
        Err(e) => {
            tracing::warn!("no JavaScript runtime for yt-dlp: {e:#}");
            None
        }
    }
}

#[cfg(windows)]
fn no_window(c: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

/// yt-dlp's leftovers for a download saved as `path`: its `.part` files, the unmerged video
/// and audio (`name.f137.mp4`), and its resume notes.
pub(crate) fn leftovers(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else { return vec![] };
    let prefix = format!("{stem}.");
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return false };
            let Some(rest) = name.strip_prefix(&prefix) else { return false };
            // `f137.mp4`, `f251-drc.webm`: a format id, which starts with a digit, then an extension.
            let unmerged = rest
                .strip_prefix('f')
                .and_then(|r| r.split_once('.'))
                .is_some_and(|(id, _)| id.starts_with(|c: char| c.is_ascii_digit()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
            name.ends_with(".part") || name.ends_with(".ytdl") || name.contains(".part-Frag") || unmerged
        })
        .collect()
}

/// One line of yt-dlp's progress, from the template [`Manager::run_media`] gives it.
#[derive(Debug, PartialEq)]
struct Progress {
    done: u64,
    total: Option<u64>,
    format: String,
}

/// `REIDM-PROGRESS <done> <total> <format's size> <estimate> <format id>`; yt-dlp writes `NA`
/// for what it doesn't know. The total, when there is one, else the size YouTube gives for the
/// format, else yt-dlp's running estimate — which, for a format fetched in pieces, swings
/// wildly until the end.
fn parse_progress(line: &str) -> Option<Progress> {
    let mut it = line.strip_prefix("REIDM-PROGRESS ")?.split_whitespace();
    let num = |s: Option<&str>| s.and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64);
    let done = num(it.next())?;
    let exact = num(it.next());
    let declared = num(it.next()).filter(|n| *n > 0);
    let estimate = num(it.next());
    let format = it.next().unwrap_or("").to_string();
    Some(Progress { done, total: exact.or(declared).or(estimate), format })
}

/// The cookie header the browser sent, as the cookies file yt-dlp reads.
fn cookie_file(cookie: &str, page: &url::Url) -> String {
    let host = page.host_str().unwrap_or("");
    // `.youtube.com` covers `www.` and `m.` as the browser's own cookies do.
    let domain = format!(".{}", host.trim_start_matches("www.").trim_start_matches("m."));
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for pair in cookie.split(';') {
        if let Some((k, v)) = pair.trim().split_once('=') {
            if !k.is_empty() && !k.contains(['\t', '\n']) && !v.contains(['\t', '\n']) {
                out.push_str(&format!("{domain}\tTRUE\t/\tTRUE\t0\t{k}\t{v}\n"));
            }
        }
    }
    out
}

/// What to tell the user when yt-dlp fails: its own `ERROR:` line, made a little plainer.
fn explain(stderr: &[String], code: Option<i32>) -> String {
    let err = stderr.iter().rev().find_map(|l| l.strip_prefix("ERROR: ")).map(|s| s.trim().to_string());
    let msg = err.or_else(|| stderr.last().cloned()).unwrap_or_else(|| format!("yt-dlp stopped (exit code {})", code.map_or("?".into(), |c| c.to_string())));
    if msg.contains("ffmpeg is not installed") || msg.contains("ffmpeg not found") {
        return "this quality comes as separate video and audio, which need ffmpeg to be joined: install ffmpeg (Linux: sudo apt install ffmpeg; macOS: brew install ffmpeg), or pick a format with sound".into();
    }
    if msg.contains("Sign in to confirm") {
        return format!("{msg} (YouTube wants a signed-in browser: open the video while signed in to YouTube and try again)");
    }
    msg
}

/// Stop yt-dlp and whatever it started (the standalone builds run as two processes, and
/// ffmpeg may be running too), and wait for it to go. At once and by force: yt-dlp ignores a
/// polite signal in the middle of a transfer, and it resumes from its `.part` files either way.
async fn kill_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        #[cfg(windows)]
        {
            let mut c = std::process::Command::new("taskkill");
            c.args(["/F", "/T", "/PID", &pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            no_window(&mut c);
            let _ = c.status();
        }
        #[cfg(unix)]
        if let Ok(group) = libc::pid_t::try_from(pid) {
            // Its own process group, set at spawn: all of it. A system call rather than
            // `kill(1)`, which an app started from Finder may not find on its PATH.
            // SAFETY: killpg only sends a signal; the group is the one this child leads.
            unsafe {
                libc::killpg(group, libc::SIGKILL);
            }
        }
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
}

impl Manager {
    /// Run a [`Media`] download through yt-dlp. Ok(true) when finished, Ok(false) when stopped.
    pub(crate) async fn run_media(&self, id: u64, live: Live, item: Item) -> Result<bool> {
        let media = item.request.media.clone().context("not a media download")?;
        let data_dir = self.data_dir().to_path_buf();
        let tools = tools_dir(&data_dir);
        // Say so while fetching a tool: the first video can wait minutes on it, with nothing
        // else to show.
        if !tools.join(format!("yt-dlp{EXE}")).is_file() {
            self.set_note(id, Some("getting yt-dlp (first time only)"));
        }
        let ytdlp = ytdlp(&self.0.http, &data_dir).await?;
        if !tools.join(format!("deno{EXE}")).is_file() && ["deno", "node", "bun"].iter().all(|n| find_on_path(n).is_none()) {
            self.set_note(id, Some("getting Deno for yt-dlp (first time only)"));
        }
        let js = js_runtime(&self.0.http, &data_dir).await;
        let merging = media.format.contains('+');
        if merging && cfg!(windows) && !tools.join("ffmpeg").join("ffmpeg.exe").is_file() && find_on_path("ffmpeg").is_none() {
            self.set_note(id, Some("getting ffmpeg (first time only)"));
        }
        let ffmpeg = if merging { ffmpeg(&self.0.http, &data_dir).await.unwrap_or_else(|e| {
            tracing::warn!("ffmpeg unavailable: {e:#}");
            None
        }) } else { None };
        self.set_note(id, Some("looking up the video"));
        if live.cancel.load(Ordering::SeqCst) {
            return Ok(false);
        }

        let page = url::Url::parse(&item.request.url)?;
        let dir = item.path.parent().context("download has no folder")?.to_path_buf();
        std::fs::create_dir_all(&dir)?;
        // The name the browser suggested ("Title 1080p"), or else the video's own title.
        let template = match item.request.filename.as_deref().and_then(|_| item.path.file_stem()).and_then(|s| s.to_str()) {
            Some(stem) => dir.join(format!("{}.%(ext)s", stem.replace('%', "%%"))),
            // The height keeps two qualities of one video apart: with the same name, yt-dlp
            // would take the first file for the second and download nothing.
            None => dir.join("%(title).150B [%(id)s] %(height&{}p|audio)s.%(ext)s"),
        };

        let mut cmd = tokio::process::Command::new(&ytdlp);
        // As many connections as a normal download: yt-dlp fetches a format's pieces in
        // parallel. For YouTube that needs its formats treated as pieces ("dashy"); one
        // connection is throttled, and a long one is often cut off with a 403.
        let connections = self.0.settings.lock().unwrap().connections.clamp(1, 16);
        cmd.args(["--concurrent-fragments", &connections.to_string()]);
        if page.host_str().is_some_and(|h| h == "youtu.be" || h == "youtube.com" || h.ends_with(".youtube.com")) {
            cmd.args(["--extractor-args", "youtube:formats=dashy"]);
        }
        cmd.args(["--newline", "--no-colors", "--no-playlist", "--continue", "--no-mtime", "--progress", "--encoding", "utf-8"])
            .args(["--progress-template", "download:REIDM-PROGRESS %(progress.downloaded_bytes)s %(progress.total_bytes)s %(info.filesize,info.filesize_approx)s %(progress.total_bytes_estimate)s %(info.format_id)s"])
            .args(["--print", "before_dl:REIDM-NAME %(filename)s"])
            // The whole download's size, video and audio together, known before either starts.
            .args(["--print", "before_dl:REIDM-SIZE %(filesize,filesize_approx)s"])
            .args(["--print", "after_move:REIDM-FILE %(filepath)s"])
            .args(["-f", &media.format])
            .args(["--merge-output-format", "mp4/webm/mkv"])
            .arg("-o")
            .arg(&template);
        if let Some(f) = &ffmpeg {
            cmd.arg("--ffmpeg-location").arg(f);
        }
        if let Some(js) = &js {
            cmd.args(["--js-runtimes", js]);
        }
        let global = self.0.settings.lock().unwrap().effective_limit();
        let rate = match (item.speed_limit.unwrap_or(0), global) {
            (0, g) => g,
            (o, 0) => o,
            (o, g) => o.min(g),
        };
        if rate > 0 {
            cmd.args(["--limit-rate", &rate.to_string()]);
        }
        if let Some(ua) = &item.request.user_agent {
            cmd.args(["--user-agent", ua]);
        }
        let cookies = item.request.cookie.as_deref().filter(|c| !c.is_empty()).map(|c| {
            let p = data_dir.join(format!("cookies-{id}.txt"));
            let _ = std::fs::write(&p, cookie_file(c, &page));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
            p
        });
        if let Some(p) = &cookies {
            cmd.arg("--cookies").arg(p);
        }
        // `--` so an address can never be read as an option.
        cmd.arg("--").arg(page.as_str());
        cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd.spawn().with_context(|| format!("cannot run {}", ytdlp.display()))?;
        let mut out = tokio::io::BufReader::new(child.stdout.take().context("no stdout")?).lines();
        let err_lines = {
            let mut err = tokio::io::BufReader::new(child.stderr.take().context("no stderr")?).lines();
            tokio::spawn(async move {
                let mut keep: Vec<String> = Vec::new();
                while let Ok(Some(l)) = err.next_line().await {
                    tracing::debug!("yt-dlp: {l}");
                    keep.push(l);
                    if keep.len() > 40 {
                        keep.remove(0);
                    }
                }
                keep
            })
        };

        let mut parts: HashMap<String, (u64, Option<u64>)> = HashMap::new();
        let mut whole: Option<u64> = None;
        let mut final_path: Option<PathBuf> = None;
        let mut last_save = Instant::now();
        let mut stopped = false;
        loop {
            // Checked on every pass: yt-dlp reports progress several times a second, which
            // would keep resetting a timer that only looked when the output went quiet.
            if live.cancel.load(Ordering::SeqCst) {
                stopped = true;
                kill_tree(&mut child).await;
                break;
            }
            tokio::select! {
                line = out.next_line() => match line {
                    Ok(Some(l)) => {
                        if let Some(p) = parse_progress(&l) {
                            if parts.is_empty() {
                                self.set_note(id, None);
                            }
                            let complete = p.total.is_some_and(|t| p.done >= t);
                            parts.insert(p.format, (p.done, p.total));
                            // The last piece in: what follows is ffmpeg joining the two.
                            if complete && parts.len() >= 2 && parts.values().all(|(d, t)| t.is_some_and(|t| *d >= t)) {
                                self.set_note(id, Some("joining video and audio"));
                            }
                            let done: u64 = parts.values().map(|v| v.0).sum();
                            // The size announced up front, unless the parts turn out bigger.
                            let size = match (whole, parts.values().map(|v| v.1).sum::<Option<u64>>()) {
                                (Some(w), Some(p)) => Some(w.max(p)),
                                (w, p) => w.or(p),
                            };
                            live.progress.store(done, Ordering::Relaxed);
                            if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
                                it.downloaded = done;
                                if size.is_some() {
                                    it.size = size;
                                }
                            }
                            if last_save.elapsed() > Duration::from_secs(3) {
                                self.save();
                                last_save = Instant::now();
                            }
                        } else if let Some(f) = l.strip_prefix("REIDM-FILE ") {
                            final_path = Some(PathBuf::from(f.trim()));
                        } else if let Some(n) = l.strip_prefix("REIDM-SIZE ") {
                            whole = n.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).map(|v| v as u64);
                            if let (Some(w), Some(it)) = (whole, self.0.db.lock().unwrap().items.get_mut(&id)) {
                                it.size = Some(w);
                            }
                        } else if let Some(f) = l.strip_prefix("REIDM-NAME ") {
                            // Where it is going, known before the first byte: the list shows the
                            // real name at once, and removing the download finds its pieces.
                            let p = PathBuf::from(f.trim());
                            if p.is_absolute() {
                                if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
                                    it.path = p;
                                }
                            }
                        }
                    }
                    _ => break,
                },
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
        let status = child.wait().await;
        // A process yt-dlp started could still hold its stderr open; don't wait on it forever.
        let stderr = tokio::time::timeout(Duration::from_secs(3), err_lines).await.ok().and_then(Result::ok).unwrap_or_default();
        if let Some(p) = &cookies {
            let _ = std::fs::remove_file(p);
        }
        if stopped {
            return Ok(false);
        }
        let status = status?;
        if !status.success() {
            bail!("{}", explain(&stderr, status.code()));
        }
        let path = final_path.filter(|p| p.is_file()).context("yt-dlp finished but did not say where it saved the file")?;
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        live.progress.store(len, Ordering::Relaxed);
        if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
            it.path = path;
            it.size = Some(len);
            it.downloaded = len;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_selectors_and_nothing_else() {
        assert!(valid_format("137+bestaudio[ext=m4a]/bestvideo[height<=1080]+bestaudio/best[height<=1080]"));
        assert!(valid_format("140"));
        assert!(!valid_format(""));
        assert!(!valid_format("137 --exec rm"));
        assert!(!valid_format("137;ls"));
        assert!(!valid_format(&"1".repeat(201)));
    }

    #[test]
    fn progress_lines_parse() {
        assert_eq!(
            parse_progress("REIDM-PROGRESS 1024 4096 NA NA 137"),
            Some(Progress { done: 1024, total: Some(4096), format: "137".into() })
        );
        assert_eq!(
            parse_progress("REIDM-PROGRESS 10 NA NA 5000.5 251"),
            Some(Progress { done: 10, total: Some(5000), format: "251".into() })
        );
        // In pieces: the format's declared size beats the swinging estimate.
        assert_eq!(
            parse_progress("REIDM-PROGRESS 68828573 NA 401308743 2370693360.0 400"),
            Some(Progress { done: 68828573, total: Some(401308743), format: "400".into() })
        );
        assert_eq!(parse_progress("REIDM-PROGRESS NA NA NA NA 18"), None);
        assert_eq!(parse_progress("[download] 50%"), None);
    }

    #[test]
    fn cookies_become_a_cookie_file_for_the_site() {
        let page = url::Url::parse("https://www.youtube.com/watch?v=x").unwrap();
        let f = cookie_file("SID=abc; PREF=f6=40000000; bad", &page);
        assert!(f.starts_with("# Netscape HTTP Cookie File\n"));
        assert!(f.contains(".youtube.com\tTRUE\t/\tTRUE\t0\tSID\tabc\n"));
        assert!(f.contains("\tPREF\tf6=40000000\n"));
        assert!(!f.contains("bad"));
    }

    #[test]
    fn leftovers_are_only_this_downloads() {
        let d = std::env::temp_dir().join(format!("reidm-media-left-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for f in ["Talk 1080p.f137.mp4.part", "Talk 1080p.f140.m4a", "Talk 1080p.f137.mp4.ytdl", "Talk 1080p.mp4", "Talk 1080p (1).mp4", "Other.f137.mp4.part", "Talk 1080p.final.mp4", "Talk 1080p.flac"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let mut got: Vec<String> = leftovers(&d.join("Talk 1080p.mp4")).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        got.sort();
        assert_eq!(got, ["Talk 1080p.f137.mp4.part", "Talk 1080p.f137.mp4.ytdl", "Talk 1080p.f140.m4a"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn checksums_in_both_formats() {
        let h = "c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490";
        let lines = format!("{h}  deno-x86_64-unknown-linux-gnu.zip\n{}  other.zip\n", "0".repeat(64));
        assert_eq!(expected_hash(&lines, "deno-x86_64-unknown-linux-gnu.zip").as_deref(), Some(h));
        assert_eq!(expected_hash(&lines, "missing.zip"), None);
        let ps = "\nAlgorithm : SHA256\nHash      : A0C3101B4158D1DFB7D6A78A7BF0F3DE80C96BB423C152BEEC8BEB22786F2238\nPath      : C:\\a\\deno.zip\n";
        assert_eq!(expected_hash(ps, "deno-x86_64-pc-windows-msvc.zip").as_deref(), Some("a0c3101b4158d1dfb7d6a78a7bf0f3de80c96bb423c152beec8beb22786f2238"));
    }

    /// A zip with one stored and one deflated entry, put together by hand.
    fn tiny_zip() -> Vec<u8> {
        use std::io::Write;
        let mut deflated = Vec::new();
        let mut enc = flate2::write::DeflateEncoder::new(&mut deflated, flate2::Compression::default());
        enc.write_all(b"deflated bytes, deflated bytes, deflated bytes").unwrap();
        enc.finish().unwrap();
        let entries: [(&str, u16, Vec<u8>, usize); 2] = [("pkg/bin/a.exe", 0, b"stored".to_vec(), 6), ("pkg/b.txt", 8, deflated.clone(), 47)];
        let (mut out, mut central) = (Vec::new(), Vec::new());
        for (name, method, data, size) in &entries {
            let off = out.len() as u32;
            out.extend([0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0]);
            out.extend(method.to_le_bytes());
            out.extend([0u8; 8]); // time, date, crc
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((*size as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(data);
            central.extend([0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0]);
            central.extend(method.to_le_bytes());
            central.extend([0u8; 8]);
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((*size as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 12]); // extra, comment, disk, attributes
            central.extend(off.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend(&central);
        out.extend([0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 2, 0, 2, 0]);
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_off.to_le_bytes());
        out.extend([0, 0]);
        out
    }

    #[test]
    fn unzip_takes_only_what_is_asked_for() {
        let d = std::env::temp_dir().join(format!("reidm-unzip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let n = unzip(&tiny_zip(), |name| name.strip_prefix("pkg/bin/").map(|f| d.join(f))).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read(d.join("a.exe")).unwrap(), b"stored");
        let n = unzip(&tiny_zip(), |name| (name == "pkg/b.txt").then(|| d.join("b.txt"))).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read(d.join("b.txt")).unwrap(), b"deflated bytes, deflated bytes, deflated bytes");
        assert!(unzip(b"not a zip at all, not at all", |_| None).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn failures_are_explained() {
        let e = explain(&["WARNING: x".into(), "ERROR: [youtube] abc: Video unavailable".into()], Some(1));
        assert_eq!(e, "[youtube] abc: Video unavailable");
        let e = explain(&["ERROR: You have requested merging of multiple formats but ffmpeg is not installed. Aborting due to errors".into()], Some(1));
        assert!(e.contains("install ffmpeg"));
        assert_eq!(explain(&[], Some(2)), "yt-dlp stopped (exit code 2)");
    }
}
