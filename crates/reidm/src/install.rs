//! Registers the native-messaging host manifest (`com.tonec.idm`) with browsers.
//! The manifest content mirrors the one shipped by the original installer.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const CHROME_ORIGINS: [&str; 2] = [
    "chrome-extension://ngpampappnmepgilojfohadhhmbhlaek/",
    "chrome-extension://llbjbkhnmlidjebalopleeepgdfgcpec/",
];
const FIREFOX_ID: &str = "mozilla_cc3@internetdownloadmanager.com";
const HOST_NAME: &str = "com.tonec.idm";

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
            ("chromium", "Chromium"),
            ("edge", "Microsoft Edge"),
            ("brave", "BraveSoftware/Brave-Browser"),
            ("vivaldi", "Vivaldi"),
        ] {
            t.push(Target { browser: b, firefox: false, dir: a.join(d).join("NativeMessagingHosts") });
        }
        t.push(Target { browser: "firefox", firefox: true, dir: a.join("Mozilla/NativeMessagingHosts") });
    }
    #[cfg(windows)]
    {
        // Windows locates hosts through the registry; manifests live next to our data.
        let _ = &home;
    }
    t
}

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

fn host_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let name = if cfg!(windows) { "reidm-host.exe" } else { "reidm-host" };
    let p = exe.with_file_name(name);
    p.canonicalize().with_context(|| format!("{} not found next to reidm", p.display()))
}

pub fn install() -> Result<()> {
    let host = host_binary()?;
    #[cfg(windows)]
    {
        use std::process::Command;
        let dir = reidm_core::default_data_dir();
        std::fs::create_dir_all(&dir)?;
        for (browser, key, firefox) in [
            ("chrome", r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.tonec.idm", false),
            ("edge", r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.tonec.idm", false),
            ("brave", r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.tonec.idm", false),
            ("firefox", r"HKCU\Software\Mozilla\NativeMessagingHosts\com.tonec.idm", true),
        ] {
            let file = dir.join(format!("{HOST_NAME}.{}.json", if firefox { "firefox" } else { "chromium" }));
            std::fs::write(&file, serde_json::to_vec_pretty(&manifest(&host, firefox))?)?;
            let st = Command::new("reg").args(["add", key, "/ve", "/t", "REG_SZ", "/d"]).arg(&file).arg("/f").status()?;
            println!("{browser}: {}", if st.success() { "registered" } else { "FAILED" });
        }
    }
    #[cfg(not(windows))]
    for t in targets() {
        // Only touch browsers that are actually installed (their profile dir's parent exists).
        let Some(parent) = t.dir.parent() else { continue };
        if !parent.exists() {
            continue;
        }
        std::fs::create_dir_all(&t.dir)?;
        let f = t.dir.join(format!("{HOST_NAME}.json"));
        std::fs::write(&f, serde_json::to_vec_pretty(&manifest(&host, t.firefox))?)?;
        println!("{}: wrote {}", t.browser, f.display());
    }
    println!("host binary: {}", host.display());
    Ok(())
}

pub fn uninstall() -> Result<()> {
    #[cfg(windows)]
    for key in [
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts\com.tonec.idm",
        r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.tonec.idm",
        r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.tonec.idm",
        r"HKCU\Software\Mozilla\NativeMessagingHosts\com.tonec.idm",
    ] {
        let _ = std::process::Command::new("reg").args(["delete", key, "/f"]).status();
    }
    #[cfg(not(windows))]
    for t in targets() {
        let f = t.dir.join(format!("{HOST_NAME}.json"));
        if f.exists() {
            std::fs::remove_file(&f)?;
            println!("{}: removed {}", t.browser, f.display());
        }
    }
    Ok(())
}
