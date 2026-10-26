//! Browser integration on disk: the native-messaging host manifest, and the copy of reIDM's
//! own extension that browsers load.
//!
//! `com.zektopic.reidm` is served by `reidm-host` and lets reIDM's extension start reIDM on
//! demand. Chromium-family browsers find a host through a JSON manifest: in a per-browser
//! directory on Linux and macOS, through a registry key pointing at the file on Windows.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// reIDM's own extension and its host. The Chromium ID is fixed by the `key` in the extension
/// manifest; Firefox has no such key, so its `browser_specific_settings.gecko.id` names it, and
/// the native host admits that ID in `allowed_extensions`.
pub const REIDM_HOST_NAME: &str = "com.zektopic.reidm";
pub const REIDM_EXTENSION_ID: &str = "pgeonmdaagbiccimgojpafdjcipcfnlc";
pub const REIDM_FIREFOX_ID: &str = "reidm@zektopic.in";

#[cfg(not(windows))]
struct Target {
    browser: &'static str,
    firefox: bool,
    dir: PathBuf,
}

#[cfg(not(windows))]
fn targets() -> Vec<Target> {
    let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()).unwrap_or_default();
    #[allow(unused_mut)]
    let mut t = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let c = home.join(".config");
        for (b, d) in [
            ("chrome", "google-chrome"),
            ("chrome-beta", "google-chrome-beta"),
            ("chromium", "chromium"),
            ("edge", "microsoft-edge"),
            ("brave", "BraveSoftware/Brave-Browser"),
            ("opera", "opera"),
            ("vivaldi", "vivaldi"),
        ] {
            t.push(Target { browser: b, firefox: false, dir: c.join(d).join("NativeMessagingHosts") });
        }
        t.push(Target { browser: "firefox", firefox: true, dir: home.join(".mozilla/native-messaging-hosts") });
    }
    #[cfg(target_os = "macos")]
    {
        let a = home.join("Library/Application Support");
        for (b, d) in [
            ("chrome", "Google/Chrome"),
            ("chrome-beta", "Google/Chrome Beta"),
            ("chromium", "Chromium"),
            ("edge", "Microsoft Edge"),
            ("brave", "BraveSoftware/Brave-Browser"),
            ("vivaldi", "Vivaldi"),
            ("opera", "com.operasoftware.Opera"),
        ] {
            t.push(Target { browser: b, firefox: false, dir: a.join(d).join("NativeMessagingHosts") });
        }
        t.push(Target { browser: "firefox", firefox: true, dir: a.join("Mozilla/NativeMessagingHosts") });
    }
    t
}

/// Registry keys under which Chromium-family browsers on Windows look up native hosts.
#[cfg(windows)]
const WINDOWS_BROWSERS: [(&str, &str); 4] = [
    ("chrome", r"HKCU\Software\Google\Chrome\NativeMessagingHosts"),
    ("edge", r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts"),
    ("brave", r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts"),
    ("vivaldi", r"HKCU\Software\Vivaldi\NativeMessagingHosts"),
];

fn reidm_manifest(host_path: &Path, firefox: bool) -> serde_json::Value {
    let mut m = serde_json::json!({
        "name": REIDM_HOST_NAME,
        "description": "Starts reIDM for the reIDM browser extension",
        "type": "stdio",
        "path": host_path,
    });
    if firefox {
        m["allowed_extensions"] = serde_json::json!([REIDM_FIREFOX_ID]);
    } else {
        m["allowed_origins"] = serde_json::json!([format!("chrome-extension://{REIDM_EXTENSION_ID}/")]);
    }
    m
}

fn host_binary() -> Result<PathBuf> {
    // Resolved first, so a `reidm` symlinked onto the PATH still finds the host beside the real
    // binary (inside reIDM.app, on macOS).
    let exe = std::env::current_exe()?.canonicalize()?;
    let name = if cfg!(windows) { "reidm-host.exe" } else { "reidm-host" };
    let p = exe.with_file_name(name);
    p.canonicalize().map(plain_path).with_context(|| format!("{} not found next to reidm", p.display()))
}

/// `canonicalize` on Windows returns an extended-length path (`\\?\C:\…`). Browsers can't start
/// a native host by such a path — Chrome reports only "Error when communicating with the native
/// messaging host" — and it reads oddly anywhere a person sees it, so drop the prefix.
fn plain_path(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(strip_verbatim) {
        Some(s) => PathBuf::from(s),
        None => p,
    }
}

fn strip_verbatim(s: &str) -> Option<String> {
    if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{unc}"));
    }
    let rest = s.strip_prefix(r"\\?\")?;
    // Only a drive path has a plain spelling; leave `\\?\Volume{…}` and the like alone.
    let b = rest.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then(|| rest.to_string())
}

/// Write `value` to `path` unless it already holds exactly that. Returns whether it wrote.
fn write_if_changed(path: &Path, value: &serde_json::Value) -> Result<bool> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if std::fs::read(path).ok().as_deref() == Some(bytes.as_slice()) {
        return Ok(false);
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, bytes)?;
    Ok(true)
}

#[cfg(windows)]
fn reg_default_value(key: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg").args(["query", key, "/ve"]).creation_flags(0x0800_0000).output().ok()?;
    if !out.status.success() {
        return None;
    }
    // `    (Default)    REG_SZ    C:\path\file.json`
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
}

#[cfg(windows)]
fn reg_set_default(key: &str, value: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg")
        .args(["add", key, "/ve", "/t", "REG_SZ", "/d"])
        .arg(value)
        .arg("/f")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        // "The operation completed successfully." would land in the service's log.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn install() -> Result<()> {
    let host = host_binary()?;
    for line in ensure_reidm_host()? {
        println!("{line}");
    }
    println!("host binary: {}", host.display());
    Ok(())
}

/// Register `com.zektopic.reidm` for every installed Chromium-family browser, for this user.
/// Idempotent and quiet: it only writes what is missing or stale, so the daemon runs it at
/// every start — which is what keeps the registration right after reIDM moves or is updated.
/// Returns a line per change.
pub fn ensure_reidm_host() -> Result<Vec<String>> {
    let host = host_binary()?;
    let mut changed = Vec::new();
    #[cfg(windows)]
    {
        let dir = reidm_core::default_data_dir();
        let chromium = dir.join(format!("{REIDM_HOST_NAME}.json"));
        if write_if_changed(&chromium, &reidm_manifest(&host, false))? {
            changed.push(format!("wrote {}", chromium.display()));
        }
        let firefox = dir.join(format!("{REIDM_HOST_NAME}.firefox.json"));
        if write_if_changed(&firefox, &reidm_manifest(&host, true))? {
            changed.push(format!("wrote {}", firefox.display()));
        }
        for (browser, base) in WINDOWS_BROWSERS {
            let key = format!(r"{base}\{REIDM_HOST_NAME}");
            // The installer registers its own manifest; leave any registration that already
            // leads to this host, so the two do not take turns rewriting the key.
            let current = reg_default_value(&key);
            if current.as_deref().is_some_and(|m| manifest_points_at(Path::new(m), &host)) {
                continue;
            }
            if reg_set_default(&key, &chromium) {
                changed.push(format!("{browser}: registered {REIDM_HOST_NAME}"));
            }
        }
        let key = format!(r"HKCU\Software\Mozilla\NativeMessagingHosts\{REIDM_HOST_NAME}");
        let current = reg_default_value(&key);
        if !current.as_deref().is_some_and(|m| manifest_points_at(Path::new(m), &host)) && reg_set_default(&key, &firefox) {
            changed.push(format!("firefox: registered {REIDM_HOST_NAME}"));
        }
    }
    #[cfg(not(windows))]
    for t in targets() {
        let Some(parent) = t.dir.parent() else { continue };
        if !parent.exists() {
            continue;
        }
        let f = t.dir.join(format!("{REIDM_HOST_NAME}.json"));
        if write_if_changed(&f, &reidm_manifest(&host, t.firefox))? {
            changed.push(format!("{}: wrote {}", t.browser, f.display()));
        }
    }
    Ok(changed)
}

/// Does the host manifest at `manifest` name `host`? A relative `path` is relative to the
/// manifest's own folder, which is how the Windows installer writes it.
#[cfg_attr(not(windows), allow(dead_code))]
fn manifest_points_at(manifest: &Path, host: &Path) -> bool {
    let Ok(bytes) = std::fs::read(manifest) else { return false };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return false };
    let Some(p) = v["path"].as_str() else { return false };
    let p = Path::new(p);
    let p = if p.is_absolute() { p.to_path_buf() } else { manifest.parent().unwrap_or(Path::new(".")).join(p) };
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    let chrome = v["allowed_origins"].as_array().is_some_and(|o| o.iter().any(|x| x.as_str() == Some(&format!("chrome-extension://{REIDM_EXTENSION_ID}/"))));
    let firefox = v["allowed_extensions"].as_array().is_some_and(|o| o.iter().any(|x| x.as_str() == Some(REIDM_FIREFOX_ID)));
    (chrome || firefox) && same(&p, host)
}

pub fn uninstall() -> Result<()> {
    #[cfg(windows)]
    {
        for (_, base) in WINDOWS_BROWSERS {
            let _ = std::process::Command::new("reg").args(["delete", &format!(r"{base}\{REIDM_HOST_NAME}"), "/f"]).status();
        }
        let _ = std::process::Command::new("reg").args(["delete", &format!(r"HKCU\Software\Mozilla\NativeMessagingHosts\{REIDM_HOST_NAME}"), "/f"]).status();
        // The manifests the keys pointed at, which `install` wrote into the data folder.
        let dir = reidm_core::default_data_dir();
        for f in [format!("{REIDM_HOST_NAME}.json"), format!("{REIDM_HOST_NAME}.firefox.json")] {
            let _ = std::fs::remove_file(dir.join(f));
        }
    }
    #[cfg(not(windows))]
    for t in targets() {
        let f = t.dir.join(format!("{REIDM_HOST_NAME}.json"));
        if f.exists() {
            std::fs::remove_file(&f)?;
            println!("{}: removed {}", t.browser, f.display());
        }
    }
    Ok(())
}

// ── the extension itself ─────────────────────────────────────────────────

/// Where the installer put reIDM's extension, if anywhere: next to the binaries on Windows,
/// in the app bundle's resources on macOS, under `/usr/share` on Linux, or the source tree when
/// running a development build from the build directory.
pub fn bundled_extension_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bin = exe.parent()?.to_path_buf();
    let mut candidates = vec![
        bin.join("extension"),
        bin.join("../Resources/extension"),
        bin.join("../share/reidm/extension"),
        PathBuf::from("/usr/share/reidm/extension"),
    ];
    // target/debug/reidm -> the repository's own copy. Development builds only: from an
    // installed `C:\Program Files\reIDM\reidm.exe` the same path is `C:\extension`, a folder
    // any user can create, whose contents would then be offered as reIDM's extension.
    if cfg!(debug_assertions) {
        candidates.push(bin.join("../../extension"));
    }
    candidates.into_iter().find(|d| d.join("manifest.json").is_file()).and_then(|d| d.canonicalize().ok()).map(plain_path)
}

/// The folder to point a browser's *Load unpacked* at: a per-user copy of the bundled extension.
///
/// A copy rather than the installed files because those can sit where a folder picker cannot
/// reach (inside a macOS app bundle) or be replaced wholesale by an update. The copy lives at a
/// stable path, and refreshing it in place means an update to reIDM updates the extension the
/// next time the browser starts, without loading it again.
pub fn extension_dir() -> Option<PathBuf> {
    let staged = reidm_core::default_data_dir().join("browser-extension");
    staged.join("manifest.json").is_file().then_some(staged)
}

/// Bring the per-user copy up to date with the bundled extension. Returns the folder, and
/// whether anything changed.
pub fn stage_extension() -> Result<(PathBuf, bool)> {
    let src = bundled_extension_dir().context("this installation does not include the browser extension")?;
    let dst = reidm_core::default_data_dir().join("browser-extension");
    let changed = sync_dir(&src, &dst)?;
    Ok((dst, changed))
}

/// Copy `src` over `dst`, writing only files whose bytes differ and deleting files `src` no
/// longer has. Returns whether anything changed.
fn sync_dir(src: &Path, dst: &Path) -> Result<bool> {
    std::fs::create_dir_all(dst)?;
    let mut changed = false;
    let mut keep = std::collections::HashSet::new();
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        keep.insert(name.clone());
        let (from, to) = (e.path(), dst.join(&name));
        if e.file_type()?.is_dir() {
            changed |= sync_dir(&from, &to)?;
        } else {
            let bytes = std::fs::read(&from)?;
            if std::fs::read(&to).ok().as_deref() != Some(bytes.as_slice()) {
                std::fs::write(&to, bytes)?;
                changed = true;
            }
        }
    }
    for e in std::fs::read_dir(dst)? {
        let e = e?;
        if !keep.contains(&e.file_name()) {
            let p = e.path();
            if e.file_type()?.is_dir() {
                std::fs::remove_dir_all(&p)?;
            } else {
                std::fs::remove_file(&p)?;
            }
            changed = true;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_length_paths_lose_their_prefix() {
        assert_eq!(strip_verbatim(r"\\?\C:\Users\a\reidm-host.exe").as_deref(), Some(r"C:\Users\a\reidm-host.exe"));
        assert_eq!(strip_verbatim(r"\\?\UNC\server\share\reidm-host.exe").as_deref(), Some(r"\\server\share\reidm-host.exe"));
        assert_eq!(strip_verbatim(r"\\?\Volume{0b1c}\x"), None);
        assert_eq!(strip_verbatim(r"C:\already\plain"), None);
        assert_eq!(strip_verbatim("/usr/bin/reidm-host"), None);
    }

    #[test]
    fn sync_copies_updates_and_prunes() {
        let root = std::env::temp_dir().join(format!("reidm-sync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (src, dst) = (root.join("src"), root.join("dst"));
        std::fs::create_dir_all(src.join("icons")).unwrap();
        std::fs::write(src.join("manifest.json"), b"{}").unwrap();
        std::fs::write(src.join("icons/16.png"), b"png").unwrap();
        assert!(sync_dir(&src, &dst).unwrap());
        assert!(!sync_dir(&src, &dst).unwrap(), "a second sync changes nothing");
        std::fs::write(src.join("manifest.json"), b"{\"v\":2}").unwrap();
        std::fs::write(dst.join("stale.js"), b"old").unwrap();
        assert!(sync_dir(&src, &dst).unwrap());
        assert_eq!(std::fs::read(dst.join("manifest.json")).unwrap(), b"{\"v\":2}");
        assert!(!dst.join("stale.js").exists());
        assert!(dst.join("icons/16.png").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_relative_host_path_is_resolved_against_the_manifest() {
        let root = std::env::temp_dir().join(format!("reidm-mf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let host = root.join("reidm-host");
        std::fs::write(&host, b"").unwrap();
        let mf = root.join("m.json");
        let mut m = reidm_manifest(Path::new("reidm-host"), false);
        std::fs::write(&mf, serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(manifest_points_at(&mf, &host));
        m["allowed_origins"] = serde_json::json!(["chrome-extension://someoneelse/"]);
        std::fs::write(&mf, serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(!manifest_points_at(&mf, &host), "a manifest for another extension does not count");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reidm_host_admits_only_reidms_extension() {
        let m = reidm_manifest(Path::new("/usr/bin/reidm-host"), false);
        assert_eq!(m["name"], REIDM_HOST_NAME);
        assert_eq!(m["allowed_origins"], serde_json::json!([format!("chrome-extension://{REIDM_EXTENSION_ID}/")]));
    }

    #[test]
    fn the_firefox_host_admits_only_reidms_firefox_id() {
        let m = reidm_manifest(Path::new("/usr/bin/reidm-host"), true);
        assert_eq!(m["allowed_extensions"], serde_json::json!([REIDM_FIREFOX_ID]));
        assert!(m.get("allowed_origins").is_none(), "Firefox hosts use allowed_extensions, not origins");
    }
}
