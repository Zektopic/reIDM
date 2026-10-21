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

/// Start a helper program (a file manager, a browser) and wait for it on a thread of its own.
/// A child nobody waits for stays in the process table as a zombie for as long as the service
/// runs, one per *Open*; `xdg-open` and `open` exit as soon as they have handed over, so the
/// thread is short-lived.
fn spawn_reaped(c: &mut Command) {
    match c.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!("could not start {:?}: {e}", c.get_program()),
    }
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
    spawn_reaped(quiet(&mut c).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()));
}

/// Open a downloaded file, or reveal it in the file manager.
#[allow(dead_code)]
pub fn open_path(p: &Path, reveal: bool) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("explorer");
        if reveal {
            // explorer.exe does not use the normal C runtime quoting rules: it wants
            // `/select,C:\path with spaces\file` as one *unquoted* argument. Rust would quote any
            // argument containing a space, which makes explorer open the default folder instead,
            // so pass the raw text.
            c.raw_arg(format!("/select,{}", p.display()));
        } else {
            c.arg(p);
        }
        spawn_reaped(&mut c);
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        if reveal { c.arg("-R"); }
        spawn_reaped(c.arg(p));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let t = if reveal { p.parent().unwrap_or(p) } else { p };
        spawn_reaped(Command::new("xdg-open").arg(t));
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
