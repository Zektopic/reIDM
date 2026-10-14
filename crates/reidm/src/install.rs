//! Browser integration on disk: the native-messaging host manifests, and the copy of reIDM's
//! own extension that browsers load.
//!
//! Two hosts are registered, both served by `reidm-host`:
//!
//! * `com.tonec.idm`, mirroring the manifest the original IDM installer ships, for the official
//!   IDM extension;
//! * `com.zektopic.reidm`, for reIDM's own extension, which uses it to start reIDM on demand.
//!
//! Chromium-family browsers find a host through a JSON manifest: in a per-browser directory on
//! Linux and macOS, through a registry key pointing at the file on Windows.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const CHROME_ORIGINS: [&str; 2] = [
    "chrome-extension://ngpampappnmepgilojfohadhhmbhlaek/",
    "chrome-extension://llbjbkhnmlidjebalopleeepgdfgcpec/",
];
const FIREFOX_ID: &str = "mozilla_cc3@internetdownloadmanager.com";
const HOST_NAME: &str = "com.tonec.idm";

/// reIDM's own extension and its host. The ID is fixed by the `key` in the extension manifest.
pub const REIDM_HOST_NAME: &str = "com.zektopic.reidm";
pub const REIDM_EXTENSION_ID: &str = "pgeonmdaagbiccimgojpafdjcipcfnlc";

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

fn manifest(host_path: &Path, firefox: bool) -> serde_json::Value {
    let mut m = serde_json::json!({
        "name": HOST_NAME,
        "description": "reIDM native messaging host (IDM-compatible)",
        "type": "stdio",
        "path": host_path,
    });
    if firefox {
        m["allowed_extensions"] = serde_json::json!([FIREFOX_ID]);
    } else {
        m["allowed_origins"] = serde_json::json!(CHROME_ORIGINS);
    }
    m
}

fn reidm_manifest(host_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "name": REIDM_HOST_NAME,
        "description": "Starts reIDM for the reIDM browser extension",
        "type": "stdio",
        "path": host_path,
        "allowed_origins": [format!("chrome-extension://{REIDM_EXTENSION_ID}/")],
    })
}

fn host_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let name = if cfg!(windows) { "reidm-host.exe" } else { "reidm-host" };
    let p = exe.with_file_name(name);
    p.canonicalize().with_context(|| format!("{} not found next to reidm", p.display()))
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
    let out = std::process::Command::new("reg").args(["query", key, "/ve"]).output().ok()?;
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
        .status()
        .is_ok_and(|s| s.success())
}

pub fn install() -> Result<()> {
    let host = host_binary()?;
    #[cfg(windows)]
    {
        let dir = reidm_core::default_data_dir();
        std::fs::create_dir_all(&dir)?;
        let idm_file = dir.join(format!("{HOST_NAME}.chromium.json"));
        write_if_changed(&idm_file, &manifest(&host, false))?;
        let ff_file = dir.join(format!("{HOST_NAME}.firefox.json"));
        write_if_changed(&ff_file, &manifest(&host, true))?;
        for (browser, base) in WINDOWS_BROWSERS {
            let ok = reg_set_default(&format!(r"{base}\{HOST_NAME}"), &idm_file);
            println!("{browser}: {}", if ok { "registered" } else { "FAILED" });
        }
        let ok = reg_set_default(&format!(r"HKCU\Software\Mozilla\NativeMessagingHosts\{HOST_NAME}"), &ff_file);
        println!("firefox: {}", if ok { "registered" } else { "FAILED" });
    }
    #[cfg(not(windows))]
    for t in targets() {
        // Only touch browsers that are actually installed (their profile dir's parent exists).
        let Some(parent) = t.dir.parent() else { continue };
        if !parent.exists() {
            continue;
        }
        let f = t.dir.join(format!("{HOST_NAME}.json"));
        write_if_changed(&f, &manifest(&host, t.firefox))?;
        println!("{}: wrote {}", t.browser, f.display());
    }
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
        let file = dir.join(format!("{REIDM_HOST_NAME}.json"));
        if write_if_changed(&file, &reidm_manifest(&host))? {
            changed.push(format!("wrote {}", file.display()));
        }
        for (browser, base) in WINDOWS_BROWSERS {
            let key = format!(r"{base}\{REIDM_HOST_NAME}");
            if reg_default_value(&key).as_deref() != Some(&*file.to_string_lossy()) && reg_set_default(&key, &file) {
                changed.push(format!("{browser}: registered {REIDM_HOST_NAME}"));
            }
        }
    }
    #[cfg(not(windows))]
    for t in targets().into_iter().filter(|t| !t.firefox) {
        let Some(parent) = t.dir.parent() else { continue };
        if !parent.exists() {
            continue;
        }
        let f = t.dir.join(format!("{REIDM_HOST_NAME}.json"));
        if write_if_changed(&f, &reidm_manifest(&host))? {
            changed.push(format!("{}: wrote {}", t.browser, f.display()));
        }
    }
    Ok(changed)
}

pub fn uninstall() -> Result<()> {
    #[cfg(windows)]
    {
        let mut keys: Vec<String> = Vec::new();
        for (_, base) in WINDOWS_BROWSERS {
            keys.push(format!(r"{base}\{HOST_NAME}"));
            keys.push(format!(r"{base}\{REIDM_HOST_NAME}"));
        }
        keys.push(format!(r"HKCU\Software\Mozilla\NativeMessagingHosts\{HOST_NAME}"));
        for key in keys {
            let _ = std::process::Command::new("reg").args(["delete", &key, "/f"]).status();
        }
    }
    #[cfg(not(windows))]
    for t in targets() {
        for name in [HOST_NAME, REIDM_HOST_NAME] {
            let f = t.dir.join(format!("{name}.json"));
            if f.exists() {
                std::fs::remove_file(&f)?;
                println!("{}: removed {}", t.browser, f.display());
            }
        }
    }
    Ok(())
}

// ── the extension itself ─────────────────────────────────────────────────

/// Where the installer put reIDM's extension, if anywhere: next to the binaries on Windows,
/// in the app bundle's resources on macOS, under `/usr/share` on Linux, or the source tree when
/// running from a build directory.
pub fn bundled_extension_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bin = exe.parent()?.to_path_buf();
    let mut candidates = vec![
        bin.join("extension"),
        bin.join("../Resources/extension"),
        bin.join("../share/reidm/extension"),
        PathBuf::from("/usr/share/reidm/extension"),
    ];
    // target/{debug,release}/reidm -> the repository's own copy
    candidates.push(bin.join("../../extension"));
    candidates.into_iter().find(|d| d.join("manifest.json").is_file()).and_then(|d| d.canonicalize().ok())
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
    fn reidm_host_admits_only_reidms_extension() {
        let m = reidm_manifest(Path::new("/usr/bin/reidm-host"));
        assert_eq!(m["name"], REIDM_HOST_NAME);
        assert_eq!(m["allowed_origins"], serde_json::json!([format!("chrome-extension://{REIDM_EXTENSION_ID}/")]));
    }
}
