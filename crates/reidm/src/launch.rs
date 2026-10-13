//! Opening files/URLs and starting the daemon without a console window.
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(windows)]
fn quiet(c: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
fn quiet(c: &mut Command) -> &mut Command {
    c
}

pub fn open_url(url: &str) {
    #[cfg(windows)]
    let mut c = {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut c = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut c = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = quiet(&mut c).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Open a downloaded file, or reveal it in the file manager.
#[allow(dead_code)]
pub fn open_path(p: &Path, reveal: bool) {
    #[cfg(windows)]
    {
        let mut c = Command::new("explorer");
        if reveal { c.arg(format!("/select,{}", p.display())); } else { c.arg(p); }
        let _ = c.spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        if reveal { c.arg("-R"); }
        let _ = c.arg(p).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let t = if reveal { p.parent().unwrap_or(p) } else { p };
        let _ = Command::new("xdg-open").arg(t).spawn();
    }
}

/// Spawn `reidm daemon` detached, hidden, next to the current executable.
pub fn spawn_daemon() -> std::io::Result<()> {
    let exe = std::env::current_exe()?.with_file_name(if cfg!(windows) { "reidm.exe" } else { "reidm" });
    let mut c = Command::new(exe);
    c.arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0000_0008 | 0x0800_0000); // DETACHED_PROCESS | CREATE_NO_WINDOW
    }
    c.spawn().map(|_| ())
}
