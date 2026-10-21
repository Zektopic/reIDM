//! *Start reIDM when you sign in*: the login entry that runs `reidm-gui --background`, which
//! starts the download service and exits without a window, so the browser's downloads are
//! caught before anyone opens reIDM.
//!
//! Each system keeps it where its own tools look: the `Run` key on Windows (the installer
//! writes the same value, so the installer and this checkbox agree), an XDG autostart entry on
//! Linux, a launch agent on macOS. Only the current user's, so no administrator rights are
//! needed.

use std::path::{Path, PathBuf};

/// The program the login entry starts. Windows' `\\?\` prefix is dropped: the `Run` key is
/// read by Explorer, which doesn't take it.
fn program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't tell where reIDM is installed: {e}"))?;
    let s = exe.to_string_lossy();
    Ok(match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => exe,
    })
}

/// Whether reIDM starts at sign-in; `None` when that can't be told here.
pub fn enabled() -> Option<bool> {
    imp::enabled()
}

/// Turn starting at sign-in on or off.
pub fn set(on: bool) -> Result<(), String> {
    imp::set(&program()?, on)
}

/// An XDG desktop entry's `Exec` value: arguments with spaces or specials quoted, and inside
/// quotes `"`, `` ` ``, `$` and `\` escaped, as the Desktop Entry specification asks.
#[cfg_attr(not(any(test, all(unix, not(target_os = "macos")))), allow(dead_code))]
fn desktop_exec(args: &[&str]) -> String {
    args.iter()
        .map(|a| {
            if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c)) {
                a.to_string()
            } else {
                let mut q = String::from('"');
                for c in a.chars() {
                    if matches!(c, '"' | '`' | '$' | '\\') {
                        q.push('\\');
                    }
                    q.push(c);
                }
                q.push('"');
                q
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg_attr(not(any(test, all(unix, not(target_os = "macos")))), allow(dead_code))]
fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=reIDM\nComment=Starts reIDM's download service\nExec={}\nIcon=reidm\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        desktop_exec(&[&exe.to_string_lossy(), "--background"])
    )
}

/// An autostart entry the desktop would run. Desktops switch one off by marking it hidden or
/// disabled rather than deleting it.
#[cfg_attr(not(any(test, all(unix, not(target_os = "macos")))), allow(dead_code))]
fn desktop_entry_runs(text: &str) -> bool {
    !text.lines().map(str::trim).any(|l| l.eq_ignore_ascii_case("Hidden=true") || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false"))
}

#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
fn launch_agent(exe: &Path) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>com.zektopic.reidm</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t\t<string>--background</string>\n\t</array>\n\t<key>RunAtLoad</key>\n\t<true/>\n</dict>\n</plist>\n",
        esc(&exe.to_string_lossy())
    )
}

/// Write `text` to `file`, or remove it; a file already gone counts as removed.
#[cfg_attr(not(any(test, unix)), allow(dead_code))]
fn write_or_remove(file: &Path, text: Option<String>) -> Result<(), String> {
    match text {
        Some(t) => {
            if let Some(d) = file.parent() {
                std::fs::create_dir_all(d).map_err(|e| format!("can't create {}: {e}", d.display()))?;
            }
            std::fs::write(file, t).map_err(|e| format!("can't write {}: {e}", file.display()))
        }
        None => match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("can't remove {}: {e}", file.display())),
            _ => Ok(()),
        },
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use super::*;

    fn file() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(config.join("autostart").join("reidm.desktop"))
    }

    pub fn enabled() -> Option<bool> {
        let f = file()?;
        Some(std::fs::read_to_string(f).is_ok_and(|t| desktop_entry_runs(&t)))
    }

    pub fn set(exe: &Path, on: bool) -> Result<(), String> {
        let f = file().ok_or("can't find the autostart folder: HOME is not set")?;
        write_or_remove(&f, on.then(|| desktop_entry(exe)))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    fn file() -> Option<PathBuf> {
        Some(PathBuf::from(std::env::var_os("HOME")?).join("Library/LaunchAgents/com.zektopic.reidm.plist"))
    }

    pub fn enabled() -> Option<bool> {
        Some(file()?.exists())
    }

    pub fn set(exe: &Path, on: bool) -> Result<(), String> {
        let f = file().ok_or("can't find your Library folder: HOME is not set")?;
        write_or_remove(&f, on.then(|| launch_agent(exe)))
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    const KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
    /// The name the installer (`packaging/windows/reidm.iss`) gives the value.
    const NAME: PCWSTR = w!("reIDM");

    pub fn enabled() -> Option<bool> {
        let mut len = 0u32;
        // SAFETY: a size query: no buffer, `len` receives the size.
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, KEY, NAME, RRF_RT_REG_SZ, None, None, Some(&mut len)) };
        match r {
            ERROR_SUCCESS => Some(true),
            ERROR_FILE_NOT_FOUND => Some(false),
            _ => None,
        }
    }

    pub fn set(exe: &Path, on: bool) -> Result<(), String> {
        if on {
            let cmd = HSTRING::from(format!("\"{}\" --background", exe.display()));
            let bytes = (cmd.len() + 1) * 2; // UTF-16 with its terminating zero
            // SAFETY: `cmd` is a zero-terminated UTF-16 string of `bytes` bytes, alive for the call.
            let r = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, KEY, NAME, REG_SZ.0, Some(cmd.as_ptr().cast()), bytes as u32) };
            if r != ERROR_SUCCESS {
                return Err(format!("Windows refused to add reIDM to the programs that start at sign-in (error {})", r.0));
            }
        } else {
            // SAFETY: constant, zero-terminated key and value names.
            let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, KEY, NAME) };
            if r != ERROR_SUCCESS && r != ERROR_FILE_NOT_FOUND {
                return Err(format!("Windows refused to remove reIDM from the programs that start at sign-in (error {})", r.0));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_entry_quotes_what_needs_it() {
        assert_eq!(desktop_exec(&["/usr/bin/reidm-gui", "--background"]), "/usr/bin/reidm-gui --background");
        assert_eq!(desktop_exec(&["/home/me/My Apps/reidm-gui", "--background"]), "\"/home/me/My Apps/reidm-gui\" --background");
        assert_eq!(desktop_exec(&["/opt/$x/a\"b"]), r#""/opt/\$x/a\"b""#);
        let e = desktop_entry(Path::new("/opt/reIDM/reidm-gui"));
        assert!(e.starts_with("[Desktop Entry]\n") && e.contains("\nExec=/opt/reIDM/reidm-gui --background\n"), "{e}");
        assert!(desktop_entry_runs(&e));
        assert!(!desktop_entry_runs(&e.replace("X-GNOME-Autostart-enabled=true", "X-GNOME-Autostart-enabled=false")));
        assert!(!desktop_entry_runs(&format!("{e}Hidden=true\n")));
    }

    #[test]
    fn the_launch_agent_escapes_the_path() {
        let p = launch_agent(Path::new("/Applications/R&D/reIDM.app/Contents/MacOS/reidm-gui"));
        assert!(p.contains("<string>/Applications/R&amp;D/reIDM.app/Contents/MacOS/reidm-gui</string>"), "{p}");
        assert!(p.contains("<key>RunAtLoad</key>\n\t<true/>"));
    }

    #[test]
    fn turning_it_off_twice_is_fine() {
        let dir = std::env::temp_dir().join(format!("reidm-autostart-{}", std::process::id()));
        let f = dir.join("autostart/reidm.desktop");
        write_or_remove(&f, Some("x".into())).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "x");
        write_or_remove(&f, None).unwrap();
        write_or_remove(&f, None).unwrap();
        assert!(!f.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
