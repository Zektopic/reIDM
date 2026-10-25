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
    /// Keep only the sound, converted to this format (`mp3`, `m4a`, `opus`); needs ffmpeg.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
}

/// The audio formats [`Media::audio`] may name.
pub const AUDIO_FORMATS: [&str; 3] = ["mp3", "m4a", "opus"];

/// A video codec to prefer, for [`selector`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    /// Whatever is best at that resolution.
    Any,
    /// Plays anywhere: every phone, TV and editor.
    H264,
    Vp9,
    Av1,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 4] = [VideoCodec::Any, VideoCodec::H264, VideoCodec::Vp9, VideoCodec::Av1];

    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::Any => "Best available",
            VideoCodec::H264 => "H.264",
            VideoCodec::Vp9 => "VP9",
            VideoCodec::Av1 => "AV1",
        }
    }

    /// yt-dlp's filter for it.
    fn filter(self) -> &'static str {
        match self {
            VideoCodec::Any => "",
            VideoCodec::H264 => "[vcodec^=avc1]",
            VideoCodec::Vp9 => "[vcodec^=vp]",
            VideoCodec::Av1 => "[vcodec^=av01]",
        }
    }

    /// The codec a yt-dlp `vcodec` value names, if one of these.
    fn of(vcodec: &str) -> Option<VideoCodec> {
        let v = vcodec.to_ascii_lowercase();
        if v.starts_with("avc1") || v.starts_with("h264") {
            Some(VideoCodec::H264)
        } else if v.starts_with("vp09") || v.starts_with("vp9") {
            Some(VideoCodec::Vp9)
        } else if v.starts_with("av01") {
            Some(VideoCodec::Av1)
        } else {
            None
        }
    }
}

/// The format selector for "this resolution, this codec": the best video at most `height`
/// tall (any height for `None`) in `codec`, with the best sound — and, where the video has no
/// such format, the best at that height in any codec, then the best there is. A choice made for
/// a whole playlist must not fail on the one video that lacks it.
pub fn selector(height: Option<u32>, codec: VideoCodec) -> String {
    let cap = height.map(|h| format!("[height<={h}]")).unwrap_or_default();
    let c = codec.filter();
    let mut alts = Vec::new();
    if !c.is_empty() {
        alts.push(format!("bestvideo{cap}{c}+bestaudio[ext=m4a]"));
        alts.push(format!("bestvideo{cap}{c}+bestaudio"));
    }
    alts.push(format!("bestvideo{cap}+bestaudio"));
    alts.push(format!("best{cap}"));
    alts.push("best".into());
    alts.join("/")
}

/// The selector for sound only: `audio` names the format wanted (see [`AUDIO_FORMATS`]).
pub fn audio_selector(audio: &str) -> String {
    match audio {
        "m4a" => "bestaudio[ext=m4a]/bestaudio".into(),
        "opus" => "bestaudio[acodec=opus]/bestaudio".into(),
        _ => "bestaudio/best".into(),
    }
}

/// One video of a playlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistEntry {
    pub url: String,
    pub id: String,
    pub title: String,
    /// Seconds.
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub title: String,
    pub entries: Vec<PlaylistEntry>,
}

/// What one video offers: each height, with the codecs it comes in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoInfo {
    pub title: String,
    /// Tallest first.
    pub heights: Vec<HeightInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeightInfo {
    pub height: u32,
    pub codecs: Vec<VideoCodec>,
    /// The largest of its formats, in bytes, sound not included.
    pub size: Option<u64>,
}

/// Run yt-dlp for information only (`-J`), and parse what it prints.
async fn ytdlp_json(m: &Manager, args: &[&str], url: &str) -> Result<serde_json::Value> {
    use tokio::io::AsyncReadExt;
    let data_dir = m.data_dir().to_path_buf();
    let exe = ytdlp(&m.http(), &data_dir).await?;
    let js = js_runtime(&m.http(), &data_dir).await;
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.args(["-J", "--no-warnings", "--no-colors", "--encoding", "utf-8"]).args(args);
    if let Some(js) = &js {
        cmd.args(["--js-runtimes", js]);
    }
    let proxy = m.ytdlp_proxy(&data_dir);
    if let Some(p) = &proxy {
        cmd.arg("--config-locations").arg(&p.0);
    }
    cmd.arg("--").arg(url);
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1").stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    // A group of its own, so that giving up on it stops all of it: the standalone builds run as
    // two processes, and the second outlived a kill of the first.
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().with_context(|| format!("cannot run {}", exe.display()))?;
    let (mut so, mut se) = (child.stdout.take().context("no stdout")?, child.stderr.take().context("no stderr")?);
    let out = tokio::spawn(async move {
        let mut v = Vec::new();
        let _ = so.read_to_end(&mut v).await;
        v
    });
    let err = tokio::spawn(async move {
        let mut v = Vec::new();
        let _ = se.read_to_end(&mut v).await;
        v
    });
    let status = match tokio::time::timeout(Duration::from_secs(300), child.wait()).await {
        Ok(s) => s?,
        Err(_) => {
            kill_tree(&mut child).await;
            bail!("yt-dlp took too long to answer");
        }
    };
    let grace = Duration::from_secs(3);
    let stdout = tokio::time::timeout(grace, out).await.ok().and_then(Result::ok).unwrap_or_default();
    let stderr = tokio::time::timeout(grace, err).await.ok().and_then(Result::ok).unwrap_or_default();
    if !status.success() {
        let err: Vec<String> = String::from_utf8_lossy(&stderr).lines().map(String::from).collect();
        bail!("{}", explain(&err, status.code()));
    }
    serde_json::from_slice(&stdout).context("yt-dlp's answer was not JSON")
}

impl Manager {
    /// The videos of a playlist (or a channel's tab), without looking into each: quick even for
    /// hundreds. A single video's address gives a list of one.
    pub async fn playlist(&self, url: &str) -> Result<Playlist> {
        let page = url::Url::parse(url.trim()).context("invalid URL")?;
        if !matches!(page.scheme(), "http" | "https") {
            bail!("unsupported scheme {:?}", page.scheme());
        }
        let v = ytdlp_json(self, &["--flat-playlist"], page.as_str()).await?;
        let s = |v: &serde_json::Value, k: &str| v[k].as_str().map(str::to_string);
        let entry = |e: &serde_json::Value| -> Option<PlaylistEntry> {
            let id = s(e, "id")?;
            let url = s(e, "webpage_url").or_else(|| s(e, "url").filter(|u| u.starts_with("http"))).unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));
            // Deleted and private videos are listed with placeholder titles and nothing to fetch.
            let title = s(e, "title").unwrap_or_else(|| id.clone());
            if title == "[Deleted video]" || title == "[Private video]" {
                return None;
            }
            Some(PlaylistEntry { url, id, title, duration: e["duration"].as_f64() })
        };
        let entries: Vec<PlaylistEntry> = match v["entries"].as_array() {
            Some(list) => list.iter().filter_map(entry).collect(),
            None => entry(&v).into_iter().collect(),
        };
        if entries.is_empty() {
            bail!("no videos found at that address");
        }
        Ok(Playlist { title: s(&v, "title").unwrap_or_else(|| "Playlist".into()), entries })
    }

    /// The heights and codecs one video comes in.
    pub async fn video_info(&self, url: &str) -> Result<VideoInfo> {
        let v = ytdlp_json(self, &["--no-playlist"], url.trim()).await?;
        let mut by: BTreeMap<u32, (Vec<VideoCodec>, Option<u64>)> = BTreeMap::new();
        for f in v["formats"].as_array().into_iter().flatten() {
            let (Some(h), Some(vc)) = (f["height"].as_u64(), f["vcodec"].as_str()) else { continue };
            if vc == "none" || h == 0 {
                continue;
            }
            let e = by.entry(h as u32).or_default();
            if let Some(c) = VideoCodec::of(vc) {
                if !e.0.contains(&c) {
                    e.0.push(c);
                }
            }
            let size = f["filesize"].as_u64().or(f["filesize_approx"].as_u64());
            e.1 = e.1.max(size);
        }
        let heights = by
            .into_iter()
            .rev()
            .map(|(height, (mut codecs, size))| {
                codecs.sort_by_key(|c| VideoCodec::ALL.iter().position(|x| x == c));
                HeightInfo { height, codecs, size }
            })
            .collect();
        Ok(VideoInfo { title: v["title"].as_str().unwrap_or("").to_string(), heights })
    }
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
/// After a failed look for a newer yt-dlp, how long to keep using the copy there is before
/// asking again. Offline or behind a captive portal, asking at every video made each one wait
/// for GitHub's timeouts first — minutes for a queue of them.
const RECHECK_AFTER_FAILURE: Duration = Duration::from_secs(3600);
/// After failing to fetch a tool reIDM does not have at all, how long before trying again.
/// Short, because a video may not be had without it and the network may be back in a minute;
/// long enough that a queue of videos doesn't each sit through the same failure.
const REFETCH_AFTER_FAILURE: Duration = Duration::from_secs(120);
/// How long a tool server may take to answer, and then to send the next piece: a stalled
/// connection must fail, not hold the video (and its Pause) behind it for ever.
const ANSWER_WITHIN: Duration = Duration::from_secs(30);
const STALL_AFTER: Duration = Duration::from_secs(30);
/// The largest tool reIDM will download (ffmpeg's Windows build is about 100 MB), and the largest
/// checksum list: a misbehaving server must not be able to fill the disk or the memory.
const MAX_TOOL: u64 = 512 << 20;
const MAX_SUMS: usize = 1 << 20;
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

/// One fetch or update of each tool at a time — but one lock per tool, and only once the quick
/// "is it already here?" check has failed: a video whose yt-dlp is installed must not wait
/// while another fetches ffmpeg over a slow link.
static YTDLP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static DENO_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static FFMPEG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The last failed fetch or update check of each tool (by where it is installed), and why. In
/// memory: a restart is a fair moment to try again.
static FAILED: std::sync::Mutex<BTreeMap<PathBuf, (Instant, String)>> = std::sync::Mutex::new(BTreeMap::new());

/// Why fetching `tool` failed, if it did within `within`.
fn failed_recently(tool: &Path, within: Duration) -> Option<String> {
    FAILED.lock().unwrap().get(tool).filter(|(at, _)| at.elapsed() < within).map(|(_, why)| why.clone())
}

fn note_failure(tool: &Path, e: &anyhow::Error) {
    FAILED.lock().unwrap().insert(tool.to_path_buf(), (Instant::now(), format!("{e:#}")));
}

fn clear_failure(tool: &Path) {
    FAILED.lock().unwrap().remove(tool);
}

/// Whether `path` was modified within `within`.
fn modified_within(path: &Path, within: Duration) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e < within))
}

/// The unit [`with_retries`] pauses in: seconds, and in unit tests hundredths of one, so the
/// tests of failing servers don't sit through the real pauses.
const RETRY_UNIT: Duration = if cfg!(test) { Duration::from_millis(10) } else { Duration::from_secs(1) };

/// Try `op` up to three times, pausing longer each time: a connection dropped halfway through
/// a 40 MB download is common enough not to fail a video over.
async fn with_retries<T, F: std::future::Future<Output = Result<T>>>(mut op: impl FnMut() -> F) -> Result<T> {
    let mut last = None;
    for wait in [0, 2, 6] {
        tokio::time::sleep(RETRY_UNIT * wait).await;
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

/// A GET that must be answered within [`ANSWER_WITHIN`]. The engine's client has no overall
/// timeout (a download may rightly take hours), so a tool fetch sets its own.
async fn get(http: &reqwest::Client, url: &str) -> Result<reqwest::Response> {
    let r = tokio::time::timeout(ANSWER_WITHIN, http.get(url).send()).await.context("the server did not answer")??;
    Ok(r.error_for_status()?)
}

/// A small file (a checksum list), in memory.
async fn fetch_small(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    with_retries(|| async { crate::torrent::read_capped(get(http, url).await?, MAX_SUMS, STALL_AFTER, "the checksum list").await })
        .await
        .with_context(|| format!("downloading {url}"))
}

/// Download `url` into the file `dest`, a network read at a time, and return its SHA-256 — the
/// hash is worked out as the bytes arrive, so a 100 MB archive never sits in memory.
async fn fetch_to(http: &reqwest::Client, url: &str, dest: &Path) -> Result<String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    with_retries(|| async {
        let r = get(http, url).await?;
        if r.content_length().is_some_and(|n| n > MAX_TOOL) {
            bail!("the file is larger than {} MiB; not downloading it", MAX_TOOL >> 20);
        }
        // Created afresh on every attempt: a retry starts from the first byte, so a half-written
        // file from a dropped connection can't be mistaken for the start of this one.
        let mut f = tokio::fs::File::create(dest).await.with_context(|| format!("cannot write {}", dest.display()))?;
        let mut hash = Sha256::new();
        let (mut body, mut got) = (r.bytes_stream(), 0u64);
        loop {
            let chunk = match tokio::time::timeout(STALL_AFTER, body.next()).await {
                Err(_) => bail!("the download stalled for {}s", STALL_AFTER.as_secs()),
                Ok(None) => break,
                Ok(Some(c)) => c?,
            };
            got += chunk.len() as u64;
            if got > MAX_TOOL {
                bail!("the file is larger than {} MiB; not downloading it", MAX_TOOL >> 20);
            }
            hash.update(&chunk);
            f.write_all(&chunk).await?;
        }
        // tokio reports a failed write only at the next operation: flush to see it now.
        f.flush().await?;
        f.sync_all().await?;
        Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>())
    })
    .await
    .with_context(|| format!("downloading {url}"))
}

/// Download `asset` into `dest` and check it against the SHA-256 listed for it in `sums_url`.
/// On any failure `dest` is gone: nothing unverified is left where it could be taken for a tool.
async fn fetch_verified(http: &reqwest::Client, url: &str, sums_url: &str, asset: &str, dest: &Path) -> Result<()> {
    let sums = String::from_utf8_lossy(&fetch_small(http, sums_url).await?).into_owned();
    let want = expected_hash(&sums, asset).with_context(|| format!("{asset} is not in the release's checksums"))?;
    let got = fetch_to(http, url, dest).await.inspect_err(|_| {
        let _ = std::fs::remove_file(dest);
    })?;
    if got != want {
        let _ = std::fs::remove_file(dest);
        bail!("{asset} did not match its published checksum; not using it");
    }
    Ok(())
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

fn le16(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(b.get(o..o + 2).context("truncated zip")?.try_into()?))
}

fn le32(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(b.get(o..o + 4).context("truncated zip")?.try_into()?))
}

/// Unpack the entries of the zip archive at `zip` for which `dest` names a place. Enough of the
/// format for release archives: stored and deflated entries, no zip64. Read from the file an
/// entry at a time — only the directory is held in memory, never the archive or a whole entry.
fn unzip(zip: &Path, dest: impl Fn(&str) -> Option<PathBuf>) -> Result<usize> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(zip)?;
    let len = f.metadata()?.len();
    // The end-of-directory record is in the last 22 bytes, after a comment of up to 64 KiB.
    let tail_len = len.min(22 + 65535);
    f.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    f.read_exact(&mut tail)?;
    let eocd = (0..tail.len().saturating_sub(21)).rev().find(|&o| tail[o..o + 4] == [0x50, 0x4b, 0x05, 0x06]).context("not a zip archive")?;
    let entries = le16(&tail, eocd + 10)? as usize;
    let (dir_len, dir_at) = (le32(&tail, eocd + 12)? as u64, le32(&tail, eocd + 16)? as u64);
    if dir_at + dir_len > len {
        bail!("truncated zip");
    }
    f.seek(SeekFrom::Start(dir_at))?;
    let mut cd = vec![0u8; dir_len as usize];
    f.read_exact(&mut cd)?;
    let mut at = 0usize;
    let mut written = 0;
    for _ in 0..entries {
        if le32(&cd, at)? != 0x0201_4b50 {
            bail!("damaged zip directory");
        }
        let method = le16(&cd, at + 10)?;
        let (csize, size) = (le32(&cd, at + 20)?, le32(&cd, at + 24)?);
        let (n, e, c) = (le16(&cd, at + 28)? as usize, le16(&cd, at + 30)? as usize, le16(&cd, at + 32)? as usize);
        let local = le32(&cd, at + 42)? as u64;
        let name = String::from_utf8_lossy(cd.get(at + 46..at + 46 + n).context("truncated zip")?).into_owned();
        at += 46 + n + e + c;
        if name.ends_with('/') || name.contains("..") {
            continue;
        }
        let Some(out) = dest(&name) else { continue };
        if csize == u32::MAX || size == u32::MAX {
            bail!("zip64 archives are not supported");
        }
        let mut head = [0u8; 30];
        f.seek(SeekFrom::Start(local))?;
        f.read_exact(&mut head)?;
        if le32(&head, 0)? != 0x0403_4b50 {
            bail!("damaged zip entry {name}");
        }
        let data = local + 30 + le16(&head, 26)? as u64 + le16(&head, 28)? as u64;
        f.seek(SeekFrom::Start(data))?;
        let raw = (&mut f).take(csize as u64);
        match method {
            0 => install_from(&out, raw, size as u64)?,
            8 => install_from(&out, flate2::read::DeflateDecoder::new(raw), size as u64)?,
            m => bail!("unsupported zip compression method {m}"),
        }
        written += 1;
    }
    Ok(written)
}

/// Write what `from` gives — exactly `len` bytes, or it is a damaged entry — to `path` as an
/// executable, replacing what is there only once it is complete.
fn install_from(path: &Path, from: impl std::io::Read, len: u64) -> Result<()> {
    let tmp = path.with_extension("download");
    let written = (|| -> Result<()> {
        let mut out = std::fs::File::create(&tmp)?;
        // One byte more than promised is enough to know the entry lied about its size.
        let n = std::io::copy(&mut from.take(len + 1), &mut out)?;
        if n != len {
            bail!("damaged archive entry for {}", path.display());
        }
        out.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    place_executable(&tmp, path)
}

/// Make the verified file `tmp` executable and move it to `path`.
fn place_executable(tmp: &Path, path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    // On Windows a running yt-dlp can't be replaced; the update then waits for the next check.
    std::fs::rename(tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(tmp);
    })?;
    Ok(())
}

/// The yt-dlp to run: reIDM's own copy, fetched or updated as needed; a system one only when
/// reIDM's can't be had (offline on first use, or no build for this system).
pub(crate) async fn ytdlp(http: &reqwest::Client, data_dir: &Path) -> Result<PathBuf> {
    ytdlp_from(http, data_dir, YTDLP_REPO).await
}

async fn ytdlp_from(http: &reqwest::Client, data_dir: &Path, repo: &str) -> Result<PathBuf> {
    let dir = tools_dir(data_dir);
    let path = dir.join(format!("yt-dlp{EXE}"));
    let stamp = dir.join("yt-dlp.version");
    // Installed, and either checked within the day or the last check failed within the hour.
    let current = || path.is_file() && (modified_within(&stamp, CHECK_EVERY) || failed_recently(&path, RECHECK_AFTER_FAILURE).is_some());
    if current() {
        return Ok(path);
    }
    let Some(asset) = ytdlp_asset() else {
        return find_on_path("yt-dlp").context("yt-dlp publishes no build for this system; install yt-dlp and put it on the PATH");
    };
    let _one = YTDLP_LOCK.lock().await;
    // Another download may have done it while this one waited.
    if current() {
        return Ok(path);
    }
    if !path.is_file() {
        if let Some(why) = failed_recently(&path, REFETCH_AFTER_FAILURE) {
            return find_on_path("yt-dlp").ok_or_else(|| anyhow!("could not get yt-dlp, which reIDM needs for this download: {why} (reIDM tries again in a couple of minutes; check the connection to github.com)"));
        }
    }
    let fetched = async {
        let tag = latest_tag(repo).await?;
        if path.is_file() && std::fs::read_to_string(&stamp).ok().as_deref().map(str::trim) == Some(tag.as_str()) {
            return Ok(tag); // still the newest
        }
        tracing::info!("fetching yt-dlp {tag}");
        std::fs::create_dir_all(&dir)?;
        let tmp = path.with_extension("download");
        fetch_verified(http, &format!("{repo}/download/{tag}/{asset}"), &format!("{repo}/download/{tag}/SHA2-256SUMS"), asset, &tmp).await?;
        place_executable(&tmp, &path)?;
        anyhow::Ok(tag)
    }
    .await;
    match fetched {
        Ok(tag) => {
            clear_failure(&path);
            let _ = std::fs::write(&stamp, tag);
            Ok(path)
        }
        Err(e) => {
            note_failure(&path, &e);
            if path.is_file() {
                // Keep working with what there is, and look again in an hour.
                tracing::warn!("could not check for a newer yt-dlp: {e:#}");
                Ok(path)
            } else {
                find_on_path("yt-dlp").ok_or_else(|| e.context("could not get yt-dlp, which reIDM needs for this download"))
            }
        }
    }
}

/// ffmpeg for joining video and audio: the system's, or on Windows one reIDM fetches.
pub(crate) async fn ffmpeg(http: &reqwest::Client, data_dir: &Path) -> Result<Option<PathBuf>> {
    let own = tools_dir(data_dir).join("ffmpeg");
    let found = || {
        if own.join(format!("ffmpeg{EXE}")).is_file() {
            Some(own.clone())
        } else {
            find_on_path("ffmpeg")
        }
    };
    if let Some(p) = found() {
        return Ok(Some(p));
    }
    if !cfg!(windows) {
        return Ok(None);
    }
    if let Some(why) = failed_recently(&own, REFETCH_AFTER_FAILURE) {
        bail!("{why}");
    }
    let _one = FFMPEG_LOCK.lock().await;
    if let Some(p) = found() {
        return Ok(Some(p));
    }
    if let Some(why) = failed_recently(&own, REFETCH_AFTER_FAILURE) {
        bail!("{why}");
    }
    let r = fetch_ffmpeg(http, data_dir, &own).await;
    match &r {
        Ok(_) => clear_failure(&own),
        Err(e) => note_failure(&own, e),
    }
    r.map(Some)
}

async fn fetch_ffmpeg(http: &reqwest::Client, data_dir: &Path, own: &Path) -> Result<PathBuf> {
    // The "shared" build: half the size of the static one, its DLLs beside ffmpeg.exe.
    let asset = if std::env::consts::ARCH == "aarch64" { "ffmpeg-master-latest-winarm64-gpl-shared.zip" } else { "ffmpeg-master-latest-win64-gpl-shared.zip" };
    tracing::info!("fetching ffmpeg ({asset})");
    let tools = tools_dir(data_dir);
    std::fs::create_dir_all(&tools)?;
    let zip = tools.join("ffmpeg.zip.download");
    fetch_verified(http, &format!("{FFMPEG_REPO}/download/latest/{asset}"), &format!("{FFMPEG_REPO}/download/latest/checksums.sha256"), asset, &zip).await?;
    // Unpacked into a folder of its own and renamed into place only when complete: a disk that
    // filled halfway used to leave an ffmpeg.exe without its DLLs, taken for installed, and
    // every merge failed from then on.
    let staging = tools.join("ffmpeg.partial");
    let (z, s, dest) = (zip.clone(), staging.clone(), own.to_path_buf());
    let unpacked = tokio::task::spawn_blocking(move || -> Result<()> {
        let _ = std::fs::remove_dir_all(&s);
        std::fs::create_dir_all(&s)?;
        // From its `bin` folder: ffmpeg.exe, ffprobe.exe and the libraries they load (not ffplay).
        unzip(&z, |name| {
            let (dir, file) = name.rsplit_once('/')?;
            let wanted = file.eq_ignore_ascii_case("ffmpeg.exe") || file.eq_ignore_ascii_case("ffprobe.exe") || file.to_ascii_lowercase().ends_with(".dll");
            ((dir.ends_with("/bin") || dir == "bin") && wanted).then(|| s.join(file))
        })?;
        anyhow::ensure!(s.join("ffmpeg.exe").is_file(), "the archive did not contain ffmpeg.exe");
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&s, &dest)?;
        Ok(())
    })
    .await?;
    let _ = std::fs::remove_file(&zip);
    if unpacked.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    unpacked.with_context(|| format!("unpacking {asset}"))?;
    Ok(own.to_path_buf())
}

/// The JavaScript runtime yt-dlp needs to read YouTube's player, as yt-dlp's `--js-runtimes`
/// value (`deno:/path/to/deno`): reIDM's own Deno, an installed Deno, Node or Bun, or else a
/// Deno fetched now. `None` when there is none to be had: yt-dlp then finds few formats or none.
pub(crate) async fn js_runtime(http: &reqwest::Client, data_dir: &Path) -> Option<String> {
    let own = tools_dir(data_dir).join(format!("deno{EXE}"));
    let found = || {
        let f = if own.is_file() { Some(("deno", own.clone())) } else { ["deno", "node", "bun"].into_iter().find_map(|n| find_on_path(n).map(|p| (n, p))) };
        f.map(|(name, path)| format!("{name}:{}", path.display()))
    };
    if let Some(f) = found() {
        return Some(f);
    }
    // Without a runtime yt-dlp still runs (with fewer formats), so a failed fetch is not
    // retried at every video.
    if failed_recently(&own, REFETCH_AFTER_FAILURE).is_some() {
        return None;
    }
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "x86_64-pc-windows-msvc",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", _) => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    };
    let _one = DENO_LOCK.lock().await;
    if let Some(f) = found() {
        return Some(f);
    }
    if failed_recently(&own, REFETCH_AFTER_FAILURE).is_some() {
        return None;
    }
    let fetched = async {
        let tag = latest_tag(DENO_REPO).await?;
        let asset = format!("deno-{target}.zip");
        tracing::info!("fetching Deno {tag}, which yt-dlp needs for YouTube");
        let tools = tools_dir(data_dir);
        std::fs::create_dir_all(&tools)?;
        let zip = tools.join("deno.zip.download");
        fetch_verified(http, &format!("{DENO_REPO}/download/{tag}/{asset}"), &format!("{DENO_REPO}/download/{tag}/{asset}.sha256sum"), &asset, &zip).await?;
        let (z, o) = (zip.clone(), own.clone());
        let exe = format!("deno{EXE}");
        let unpacked = tokio::task::spawn_blocking(move || unzip(&z, |name| (name == exe).then(|| o.clone()))).await?;
        let _ = std::fs::remove_file(&zip);
        unpacked?;
        anyhow::ensure!(own.is_file(), "{asset} did not contain deno{EXE}");
        anyhow::Ok(())
    }
    .await;
    match fetched {
        Ok(()) => {
            clear_failure(&own);
            Some(format!("deno:{}", own.display()))
        }
        Err(e) => {
            tracing::warn!("no JavaScript runtime for yt-dlp: {e:#}");
            note_failure(&own, &e);
            None
        }
    }
}

/// `fut`, unless `cancel` is set first: then `None`, and `fut` is dropped — a tool fetch it was
/// running stops with it.
async fn unless_stopped<T>(cancel: &AtomicBool, fut: impl std::future::Future<Output = T>) -> Option<T> {
    tokio::select! {
        v = fut => Some(v),
        _ = async { while !cancel.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(100)).await } } => None,
    }
}

/// Where yt-dlp keeps download `id`'s pieces until the file is complete: a folder of its own
/// in the temporary folder, so they never fill the download folder.
pub(crate) fn work_dir(temp: &Path, id: u64) -> PathBuf {
    temp.join(format!("video-{id}"))
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
            // Through tokio: run synchronously, taskkill held up a runtime thread for as long
            // as it took.
            let mut c = tokio::process::Command::new("taskkill");
            c.args(["/F", "/T", "/PID", &pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            let _ = tokio::time::timeout(Duration::from_secs(5), c.status()).await;
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
    // Waiting reaps it; a child that is never waited for stays a zombie as long as reIDM runs.
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
}

/// The next line from yt-dlp, decoded leniently: a title in another encoding must not end the
/// reading (yt-dlp would then block on a full pipe, and the download hang). `buf` keeps a line
/// read halfway when the caller's `select!` moves on, so this is safe to cancel.
async fn next_line<R: tokio::io::AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> std::io::Result<Option<String>> {
    loop {
        if let Some(end) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=end).collect();
            return Ok(Some(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string()));
        }
        let had = buf.len();
        if r.read_until(b'\n', buf).await? == 0 {
            if had == 0 && buf.is_empty() {
                return Ok(None);
            }
            let line = String::from_utf8_lossy(buf).trim_end_matches('\r').to_string();
            buf.clear();
            return Ok(Some(line));
        }
    }
}

/// How one run of yt-dlp ended.
enum Ended {
    Stopped,
    /// Finished, with the file it saved.
    Done(PathBuf),
    /// Failed. `progressed`: it moved the download on before failing.
    Failed { stderr: Vec<String>, code: Option<i32>, progressed: bool },
    /// Stopped by reIDM to start it again with new settings (a new speed limit).
    Restart,
}

/// How many times one download runs yt-dlp again after a failure partway through. Each run
/// reads the video's page again, so it gets fresh addresses — YouTube's expire after about six
/// hours, and a long download outlives them. Every retry needs the run before to have made
/// progress, so a video that fails at once still fails at once.
const CARRY_ON: u32 = 20;

/// The fewest seconds between two restarts of yt-dlp for a changed speed limit: dragging a
/// slider must not restart it at every step.
const RESTART_AT_MOST_EVERY: Duration = Duration::from_secs(5);

/// An options file telling yt-dlp which proxy to use (`--proxy`), so the proxy's password is
/// never on a command line, where every user of the computer could read it. Readable by this
/// user only, and removed when dropped.
struct ProxyFile(PathBuf);

impl Drop for ProxyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// yt-dlp's options-file line for `proxy` (an empty one means "connect directly"). The address
/// is quoted; it holds no quote of its own (its user name and password are percent-encoded, and
/// a host name with one is refused).
fn proxy_option(proxy: &str) -> String {
    format!("--proxy \"{proxy}\"\n")
}

impl Manager {
    /// The `--config-locations` file with the proxy yt-dlp should use, written into `dir`; none
    /// when yt-dlp should follow the system, as reIDM does.
    fn ytdlp_proxy(&self, dir: &Path) -> Option<ProxyFile> {
        static N: AtomicU64 = AtomicU64::new(0);
        let proxy = self.0.settings.lock().unwrap().proxy.for_videos()?;
        let path = dir.join(format!("yt-dlp-proxy-{}-{}.conf", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        match write_private(&path, &proxy_option(&proxy)) {
            Ok(()) => Some(ProxyFile(path)),
            Err(e) => {
                tracing::warn!("cannot give yt-dlp the proxy: {e}");
                None
            }
        }
    }
}

/// The cookies file for yt-dlp, readable by this user only from the moment it exists.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)?.write_all(text.as_bytes())
}

impl Manager {
    /// The speed yt-dlp may use, in bytes per second (0: no limit): the download's own limit,
    /// and its share of the global one — split evenly between the downloads running now, since
    /// yt-dlp can't draw from reIDM's shared allowance the way reIDM's own connections do.
    fn media_rate(&self, own: u64) -> u64 {
        let global = self.0.settings.lock().unwrap().effective_limit();
        let share = if global == 0 {
            0
        } else {
            let running = self.0.live.lock().unwrap().len().max(1) as u64;
            (global / running).max(1)
        };
        match (own, share) {
            (0, s) => s,
            (o, 0) => o,
            (o, s) => o.min(s),
        }
    }

    /// Run a [`Media`] download through yt-dlp. Ok(true) when finished, Ok(false) when stopped.
    pub(crate) async fn run_media(&self, id: u64, live: Live, item: Item) -> Result<bool> {
        let media = item.request.media.clone().context("not a media download")?;
        let data_dir = self.data_dir().to_path_buf();
        let tools = tools_dir(&data_dir);
        // Say so while fetching a tool: the first video can wait minutes on it, with nothing
        // else to show. Each step gives way to Pause.
        if !tools.join(format!("yt-dlp{EXE}")).is_file() {
            self.set_note(id, Some("getting yt-dlp (first time only)"));
        }
        let Some(ytdlp) = unless_stopped(&live.cancel, ytdlp(&self.http(), &data_dir)).await else { return Ok(false) };
        let ytdlp = ytdlp?;
        if !tools.join(format!("deno{EXE}")).is_file() && ["deno", "node", "bun"].iter().all(|n| find_on_path(n).is_none()) {
            self.set_note(id, Some("getting Deno for yt-dlp (first time only)"));
        }
        let Some(js) = unless_stopped(&live.cancel, js_runtime(&self.http(), &data_dir)).await else { return Ok(false) };
        let merging = media.format.contains('+') || media.audio.is_some();
        let ffmpeg = if merging {
            if cfg!(windows) && !tools.join("ffmpeg").join("ffmpeg.exe").is_file() && find_on_path("ffmpeg").is_none() {
                self.set_note(id, Some("getting ffmpeg (first time only)"));
            }
            let Some(f) = unless_stopped(&live.cancel, ffmpeg(&self.http(), &data_dir)).await else { return Ok(false) };
            f.unwrap_or_else(|e| {
                tracing::warn!("ffmpeg unavailable: {e:#}");
                None
            })
        } else {
            None
        };
        self.set_note(id, Some("looking up the video"));

        let page = url::Url::parse(&item.request.url)?;
        let dir = item.path.parent().context("download has no folder")?.to_path_buf();
        std::fs::create_dir_all(&dir)?;
        // The name the browser suggested ("Title 1080p"), or else the video's own title.
        let name = match item.request.filename.as_deref().and_then(|_| item.path.file_stem()).and_then(|s| s.to_str()) {
            Some(stem) => format!("{}.%(ext)s", stem.replace('%', "%%")),
            // The height keeps two qualities of one video apart: with the same name, yt-dlp
            // would take the first file for the second and download nothing.
            None => "%(title).150B [%(id)s] %(height&{}p|audio)s.%(ext)s".to_string(),
        };
        // Pieces in a folder of their own in the temporary folder, the finished file in `dir`.
        // A download begun before there was a temporary folder carries on beside its file.
        let work = work_dir(&self.temp_dir(), id);
        let beside = !leftovers(&item.path).is_empty();
        if !beside {
            std::fs::create_dir_all(&work)?;
        }

        // In pieces over several connections first; over one if the partial file was begun
        // that way and yt-dlp can't continue it in pieces.
        let mut pieces = true;
        let mut carried_on = 0;
        loop {
            if live.cancel.load(Ordering::SeqCst) {
                return Ok(false);
            }
            let mut cmd = tokio::process::Command::new(&ytdlp);
            // As many connections as a normal download: yt-dlp fetches a format's pieces in
            // parallel. For YouTube that needs its formats treated as pieces ("dashy"); one
            // connection is throttled, and a long one is often cut off with a 403.
            let connections = self.0.settings.lock().unwrap().connections.clamp(1, 16);
            if pieces {
                cmd.args(["--concurrent-fragments", &connections.to_string()]);
                if page.host_str().is_some_and(|h| h == "youtu.be" || h == "youtube.com" || h.ends_with(".youtube.com")) {
                    cmd.args(["--extractor-args", "youtube:formats=dashy"]);
                }
            }
            cmd.args(["--newline", "--no-colors", "--no-playlist", "--continue", "--no-mtime", "--progress", "--encoding", "utf-8"])
                // yt-dlp's default skips a piece it can't get and joins the rest, which makes a
                // video with a hole in it that reIDM would call finished. Stopping instead lets
                // the next run fetch that piece from fresh addresses.
                .arg("--abort-on-unavailable-fragments")
                .args(["--progress-template", "download:REIDM-PROGRESS %(progress.downloaded_bytes)s %(progress.total_bytes)s %(info.filesize,info.filesize_approx)s %(progress.total_bytes_estimate)s %(info.format_id)s"])
                .args(["--print", "before_dl:REIDM-NAME %(filename)s"])
                // The whole download's size, video and audio together, known before either starts.
                .args(["--print", "before_dl:REIDM-SIZE %(filesize,filesize_approx)s"])
                .args(["--print", "after_move:REIDM-FILE %(filepath)s"])
                .args(["-f", &media.format])
                .args(["--merge-output-format", "mp4/webm/mkv"]);
            if beside {
                cmd.arg("-o").arg(dir.join(&name));
            } else {
                cmd.arg("-P").arg(format!("home:{}", dir.display())).arg("-P").arg(format!("temp:{}", work.display())).arg("-o").arg(&name);
            }
            if let Some(f) = &ffmpeg {
                cmd.arg("--ffmpeg-location").arg(f);
            }
            if let Some(js) = &js {
                cmd.args(["--js-runtimes", js]);
            }
            if let Some(a) = media.audio.as_deref().filter(|a| AUDIO_FORMATS.contains(a)) {
                cmd.args(["-x", "--audio-format", a]);
            }
            // What the limit is made of, to notice when it changes.
            let limits = (self.0.settings.lock().unwrap().effective_limit(), live.limiter.rate());
            let rate = self.media_rate(limits.1);
            if rate > 0 {
                cmd.args(["--limit-rate", &rate.to_string()]);
            }
            if let Some(ua) = &item.request.user_agent {
                cmd.args(["--user-agent", ua]);
            }
            // In the download's own folder when there is one, so it goes with it; written
            // private from the start, never readable by other users even for a moment.
            let cookies = item.request.cookie.as_deref().filter(|c| !c.is_empty()).map(|c| {
                let p = if beside { data_dir.join(format!("cookies-{id}.txt")) } else { work.join("cookies.txt") };
                if let Err(e) = write_private(&p, &cookie_file(c, &page)) {
                    tracing::warn!("cannot write the cookies for yt-dlp: {e}");
                }
                p
            });
            if let Some(p) = &cookies {
                cmd.arg("--cookies").arg(p);
            }
            let proxy = self.ytdlp_proxy(if beside { &data_dir } else { &work });
            if let Some(p) = &proxy {
                cmd.arg("--config-locations").arg(&p.0);
            }
            // `--` so an address can never be read as an option. Always the page, never the
            // addresses of its video: those expire, and every run reads the page again for new ones.
            cmd.arg("--").arg(page.as_str());
            cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
            cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
            #[cfg(windows)]
            cmd.creation_flags(0x0800_0000);
            #[cfg(unix)]
            cmd.process_group(0);

            let child = cmd.spawn().with_context(|| format!("cannot run {}", ytdlp.display()));
            let ended = match child {
                Ok(child) => self.watch_ytdlp(id, &live, child, limits).await,
                Err(e) => Err(e),
            };
            if let Some(p) = &cookies {
                let _ = std::fs::remove_file(p);
            }
            drop(proxy);
            match ended? {
                Ended::Stopped => return Ok(false),
                Ended::Restart => {
                    tracing::info!("download {id}: the speed limit changed; restarting yt-dlp with the new one");
                    continue;
                }
                Ended::Done(path) => {
                    let _ = std::fs::remove_dir_all(&work);
                    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    live.progress.store(len, Ordering::Relaxed);
                    if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
                        it.path = path.clone();
                        it.size = Some(len);
                        it.downloaded = len;
                    }
                    // Marked as from the internet, as the browser would have marked it.
                    let (url, referer) = (item.request.url.clone(), item.request.referer.clone());
                    let _ = tokio::task::spawn_blocking(move || disk::mark_from_internet(&path, &url, referer.as_deref())).await;
                    return Ok(true);
                }
                Ended::Failed { stderr, code, progressed } => {
                    // Partway through: most often the video's addresses expired, or a
                    // connection dropped. Run it again; it reads the page afresh and continues
                    // from its pieces.
                    if progressed && carried_on < CARRY_ON {
                        carried_on += 1;
                        tracing::info!("download {id}: yt-dlp stopped partway ({}); carrying on with fresh addresses", explain(&stderr, code));
                        self.set_note(id, Some("looking up the video again"));
                        continue;
                    }
                    // Begun by reIDM 0.5.0 over one connection, its partial file can't be
                    // continued in pieces ("fragment 1 not found, unable to continue"): carry on
                    // the way it began.
                    if pieces && stderr.iter().any(|l| l.contains("fragment") && l.contains("not found")) {
                        tracing::info!("download {id} began over one connection; continuing it that way");
                        pieces = false;
                        continue;
                    }
                    bail!("{}", explain(&stderr, code));
                }
            }
        }
    }

    /// Follow one run of yt-dlp to its end: its progress into the download's row, Pause, and
    /// changes to the speed limit.
    async fn watch_ytdlp(&self, id: u64, live: &Live, mut child: tokio::process::Child, limits: (u64, u64)) -> Result<Ended> {
        let mut out = tokio::io::BufReader::new(child.stdout.take().context("no stdout")?);
        let err_lines = {
            let mut err = tokio::io::BufReader::new(child.stderr.take().context("no stderr")?);
            tokio::spawn(async move {
                let (mut keep, mut buf): (Vec<String>, Vec<u8>) = (Vec::new(), Vec::new());
                // Read to the end, whatever it says: a pipe nobody reads fills up, and yt-dlp
                // then blocks writing to it.
                while let Ok(Some(l)) = next_line(&mut err, &mut buf).await {
                    tracing::debug!("yt-dlp: {l}");
                    keep.push(l);
                    if keep.len() > 40 {
                        keep.remove(0);
                    }
                }
                keep
            })
        };

        let started = Instant::now();
        // Per format: (first reported, last reported, total). The first is where this run
        // began — a resumed format starts at what it already had.
        let mut parts: HashMap<String, (u64, u64, Option<u64>)> = HashMap::new();
        let mut whole: Option<u64> = None;
        let mut final_path: Option<PathBuf> = None;
        let mut last_save = Instant::now();
        let mut last_space = Instant::now();
        let mut buf = Vec::new();
        let mut why_stopped = None;
        loop {
            // reIDM doesn't see yt-dlp's writes, so it looks at the space now and then.
            if last_space.elapsed() >= crate::SPACE_CHECK_EVERY {
                last_space = Instant::now();
                if let Some(it) = self.get(id) {
                    if let Some(why) = self.short_of_space(&it).await {
                        live.stop_for_space(why);
                    }
                }
            }
            // Checked on every pass: yt-dlp reports progress several times a second, which
            // would keep resetting a timer that only looked when the output went quiet.
            if live.cancel.load(Ordering::SeqCst) {
                why_stopped = Some(Ended::Stopped);
                break;
            }
            let now_limits = (self.0.settings.lock().unwrap().effective_limit(), live.limiter.rate());
            if now_limits != limits && started.elapsed() >= RESTART_AT_MOST_EVERY {
                why_stopped = Some(Ended::Restart);
                break;
            }
            tokio::select! {
                line = next_line(&mut out, &mut buf) => match line {
                    Ok(Some(l)) => self.ytdlp_said(id, live, &l, &mut parts, &mut whole, &mut final_path, &mut last_save).await,
                    _ => break,
                },
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
        if let Some(why) = why_stopped {
            kill_tree(&mut child).await;
            return Ok(why);
        }
        // Its output has ended, but the process may linger (ffmpeg still writing, say): wait
        // for it — but not past a Pause.
        let status = loop {
            tokio::select! {
                s = child.wait() => break s?,
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    if live.cancel.load(Ordering::SeqCst) {
                        kill_tree(&mut child).await;
                        return Ok(Ended::Stopped);
                    }
                }
            }
        };
        // A process yt-dlp started could still hold its stderr open; don't wait on it forever.
        let stderr = tokio::time::timeout(Duration::from_secs(3), err_lines).await.ok().and_then(Result::ok).unwrap_or_default();
        if !status.success() {
            let progressed = parts.values().any(|(first, last, _)| last > first);
            return Ok(Ended::Failed { stderr, code: status.code(), progressed });
        }
        let path = final_path.filter(|p| p.is_file()).context("yt-dlp finished but did not say where it saved the file")?;
        Ok(Ended::Done(path))
    }

    /// One line of yt-dlp's output.
    #[allow(clippy::too_many_arguments)]
    async fn ytdlp_said(
        &self,
        id: u64,
        live: &Live,
        l: &str,
        parts: &mut HashMap<String, (u64, u64, Option<u64>)>,
        whole: &mut Option<u64>,
        final_path: &mut Option<PathBuf>,
        last_save: &mut Instant,
    ) {
        if let Some(p) = parse_progress(l) {
            if parts.is_empty() {
                self.set_note(id, None);
            }
            let complete = p.total.is_some_and(|t| p.done >= t);
            let e = parts.entry(p.format).or_insert((p.done, p.done, None));
            let fresh = p.done.saturating_sub(e.1);
            e.1 = p.done;
            e.2 = p.total;
            // What yt-dlp fetched counts against the global limit, so reIDM's own connections
            // leave it room: they wait as if they had fetched it themselves.
            if fresh > 0 {
                self.0.limiter.charge(fresh);
            }
            // The last piece in: what follows is ffmpeg joining the two.
            if complete && parts.len() >= 2 && parts.values().all(|(_, d, t)| t.is_some_and(|t| *d >= t)) {
                self.set_note(id, Some("joining video and audio"));
            }
            let done: u64 = parts.values().map(|v| v.1).sum();
            // The size announced up front, unless the parts turn out bigger.
            let size = match (*whole, parts.values().map(|v| v.2).sum::<Option<u64>>()) {
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
                *last_save = Instant::now();
            }
        } else if let Some(f) = l.strip_prefix("REIDM-FILE ") {
            *final_path = Some(PathBuf::from(f.trim()));
        } else if let Some(n) = l.strip_prefix("REIDM-SIZE ") {
            *whole = n.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).map(|v| v as u64);
            if let (Some(w), Some(it)) = (*whole, self.0.db.lock().unwrap().items.get_mut(&id)) {
                it.size = Some(w);
            }
        } else if let Some(f) = l.strip_prefix("REIDM-NAME ") {
            // Where it is going, known before the first byte: the list shows the real name at
            // once, and removing the download finds its pieces.
            let p = PathBuf::from(f.trim());
            if p.is_absolute() {
                if let Some(it) = self.0.db.lock().unwrap().items.get_mut(&id) {
                    it.path = p;
                }
            }
        }
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
    fn the_proxy_reaches_yt_dlp_in_a_private_file_not_on_its_command_line() {
        assert_eq!(proxy_option("socks5h://me:p%40ss@proxy.lan:1080"), "--proxy \"socks5h://me:p%40ss@proxy.lan:1080\"\n");
        assert_eq!(proxy_option(""), "--proxy \"\"\n", "yt-dlp's way of saying: no proxy");
        let dir = std::env::temp_dir().join(format!("reidm-ytproxy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = Manager::open(&dir, dir.join("dl"), 2).unwrap();
        assert!(m.ytdlp_proxy(&dir).is_none(), "the system's proxy: yt-dlp finds it itself");
        let proxy = crate::Proxy { mode: crate::ProxyMode::Manual, host: "proxy.lan".into(), port: 3128, username: Some("me".into()), password: Some("secret".into()), ..Default::default() };
        m.update_settings(crate::SettingsPatch { proxy: Some(proxy), ..Default::default() }).unwrap();
        let f = m.ytdlp_proxy(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&f.0).unwrap(), "--proxy \"http://me:secret@proxy.lan:3128\"\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f.0).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let path = f.0.clone();
        drop(f);
        assert!(!path.exists(), "removed once yt-dlp is done");
        let _ = std::fs::remove_dir_all(&dir);
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
    fn every_choice_makes_a_valid_selector_with_fallbacks() {
        for h in [None, Some(2160), Some(1080), Some(360)] {
            for c in VideoCodec::ALL {
                let sel = selector(h, c);
                assert!(valid_format(&sel), "{sel}");
                assert!(sel.ends_with("/best"), "always something to fall back on: {sel}");
            }
        }
        assert_eq!(selector(Some(1080), VideoCodec::H264), "bestvideo[height<=1080][vcodec^=avc1]+bestaudio[ext=m4a]/bestvideo[height<=1080][vcodec^=avc1]+bestaudio/bestvideo[height<=1080]+bestaudio/best[height<=1080]/best");
        for a in AUDIO_FORMATS {
            assert!(valid_format(&audio_selector(a)));
        }
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
        let entries: [(&str, u16, Vec<u8>, usize); 2] = [("pkg/bin/a.exe", 0, b"stored".to_vec(), 6), ("pkg/b.txt", 8, deflated.clone(), 46)];
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
        let zip = d.join("t.zip");
        std::fs::write(&zip, tiny_zip()).unwrap();
        let n = unzip(&zip, |name| name.strip_prefix("pkg/bin/").map(|f| d.join(f))).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read(d.join("a.exe")).unwrap(), b"stored");
        let n = unzip(&zip, |name| (name == "pkg/b.txt").then(|| d.join("b.txt"))).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read(d.join("b.txt")).unwrap(), b"deflated bytes, deflated bytes, deflated bytes");
        std::fs::write(d.join("bad.zip"), b"not a zip at all, not at all").unwrap();
        assert!(unzip(&d.join("bad.zip"), |_| None).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_entry_that_is_not_the_size_it_claims_is_refused_and_leaves_nothing() {
        // A deflated entry that says it unpacks to 10 bytes but gives 46: damaged, or a bomb.
        let d = std::env::temp_dir().join(format!("reidm-unzip-lie-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut z = tiny_zip();
        // The second entry's uncompressed size, in its central directory record.
        let cd = z.windows(4).rposition(|w| w == [0x50, 0x4b, 0x01, 0x02]).unwrap();
        z[cd + 24..cd + 28].copy_from_slice(&10u32.to_le_bytes());
        std::fs::write(d.join("t.zip"), z).unwrap();
        assert!(unzip(&d.join("t.zip"), |name| (name == "pkg/b.txt").then(|| d.join("b.txt"))).is_err());
        assert!(!d.join("b.txt").exists() && !d.join("b.download").exists(), "nothing half-written is left");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A stand-in for GitHub: counts the connections it gets and closes each at once, as an
    /// unreachable or captive-portal network does.
    fn refusing_server() -> (String, Arc<AtomicU64>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/releases", l.local_addr().unwrap());
        let hits = Arc::new(AtomicU64::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                h.fetch_add(1, Ordering::SeqCst);
                drop(s);
            }
        });
        (base, hits)
    }

    fn tools_with_ytdlp(tag: &str, stamp: bool) -> PathBuf {
        let d = std::env::temp_dir().join(format!("reidm-tools-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(tools_dir(&d)).unwrap();
        std::fs::write(tools_dir(&d).join(format!("yt-dlp{EXE}")), b"#!/bin/sh\n").unwrap();
        if stamp {
            std::fs::write(tools_dir(&d).join("yt-dlp.version"), "test").unwrap();
        }
        d
    }

    #[tokio::test]
    async fn a_failed_update_check_is_not_repeated_at_every_video() {
        let (repo, hits) = refusing_server();
        let d = tools_with_ytdlp("backoff", false);
        let http = reqwest::Client::new();
        // No stamp: due for a check, which fails — the copy there is used all the same.
        let p = ytdlp_from(&http, &d, &repo).await.unwrap();
        assert!(p.is_file());
        let asked = hits.load(Ordering::SeqCst);
        assert!(asked >= 1, "the check went to the server");
        // The next video doesn't ask again (for an hour), and doesn't wait.
        let t = Instant::now();
        ytdlp_from(&http, &d, &repo).await.unwrap();
        // Two seconds, not 500 ms: a loaded CI machine can take that long just to schedule the
        // check. What matters is that it does not wait for the hour's back-off, and that the
        // server hears nothing more.
        assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
        assert_eq!(hits.load(Ordering::SeqCst), asked, "no second check");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_missing_tool_that_cannot_be_fetched_fails_fast_the_second_time_and_says_why() {
        // A system yt-dlp would be used instead; this test is about reIDM's own copy.
        if find_on_path("yt-dlp").is_some() {
            return;
        }
        let (repo, hits) = refusing_server();
        let d = std::env::temp_dir().join(format!("reidm-tools-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let http = reqwest::Client::new();
        assert!(ytdlp_from(&http, &d, &repo).await.is_err());
        let asked = hits.load(Ordering::SeqCst);
        let t = Instant::now();
        let e = ytdlp_from(&http, &d, &repo).await.unwrap_err();
        // See the test above: scheduling on a busy CI machine, not the remembered failure,
        // sets the pace here.
        assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
        assert_eq!(hits.load(Ordering::SeqCst), asked, "no second attempt so soon");
        assert!(format!("{e:#}").contains("tries again"), "{e:#}");
    }

    #[tokio::test]
    async fn an_installed_tool_never_waits_for_another_fetch() {
        // Another download holds every tool lock, as it does while fetching over a slow link.
        let _y = YTDLP_LOCK.lock().await;
        let _d = DENO_LOCK.lock().await;
        let _f = FFMPEG_LOCK.lock().await;
        let d = tools_with_ytdlp("fast", true);
        std::fs::write(tools_dir(&d).join(format!("deno{EXE}")), b"").unwrap();
        let http = reqwest::Client::new();
        let quick = Duration::from_secs(1);
        assert!(tokio::time::timeout(quick, ytdlp_from(&http, &d, "http://127.0.0.1:9/never")).await.is_ok(), "yt-dlp waited on the lock");
        assert!(tokio::time::timeout(quick, js_runtime(&http, &d)).await.unwrap().is_some(), "Deno waited on the lock");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_stalled_tool_fetch_gives_way_to_pause() {
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            c.store(true, Ordering::SeqCst);
        });
        let t = Instant::now();
        let r = unless_stopped(&cancel, std::future::pending::<()>()).await;
        assert!(r.is_none());
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn a_tool_is_streamed_to_disk_and_checked_as_it_arrives() {
        use std::io::{Read, Write};
        let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let want: String = Sha256::digest(&body).iter().map(|b| format!("{b:02x}")).collect();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let (b, sums, bad) = (body.clone(), format!("{want}  tool.bin\n"), format!("{}  tool.bin\n", "0".repeat(64)));
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut req = [0u8; 2048];
                let n = s.read(&mut req).unwrap_or(0);
                let path = String::from_utf8_lossy(&req[..n]).split_whitespace().nth(1).unwrap_or("").to_string();
                let data: &[u8] = if path.ends_with("BADSUMS") {
                    bad.as_bytes()
                } else if path.ends_with("SUMS") {
                    sums.as_bytes()
                } else {
                    &b
                };
                // Chunked, so nothing says the size up front.
                let _ = write!(s, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
                for c in data.chunks(65536) {
                    let _ = write!(s, "{:x}\r\n", c.len());
                    let _ = s.write_all(c);
                    let _ = s.write_all(b"\r\n");
                }
                let _ = s.write_all(b"0\r\n\r\n");
            }
        });
        let d = std::env::temp_dir().join(format!("reidm-stream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let http = reqwest::Client::new();
        fetch_verified(&http, &format!("{base}/tool.bin"), &format!("{base}/SUMS"), "tool.bin", &d.join("t")).await.unwrap();
        assert_eq!(std::fs::read(d.join("t")).unwrap(), body);
        // A file that doesn't match its checksum leaves nothing behind.
        let e = fetch_verified(&http, &format!("{base}/tool.bin"), &format!("{base}/BADSUMS"), "tool.bin", &d.join("u")).await;
        assert!(format!("{:#}", e.unwrap_err()).contains("checksum"));
        assert!(!d.join("u").exists());
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
