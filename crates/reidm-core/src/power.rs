//! The computer's power: keeping it awake while downloads run, and putting it to sleep (or
//! shutting it down) when they are all done.
//!
//! Both talk to the operating system through a trait ([`Inhibitor`], [`PowerActions`]), so the
//! decisions — when to hold, when to let go, when a countdown starts and ends — are tested
//! without ever putting the machine that runs the tests to sleep.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::Instant;

/// How long the computer is kept awake after the last transfer: a queue that moves from one
/// download to the next must not let it fall asleep in between.
pub const RELEASE_AFTER: Duration = Duration::from_secs(30);
/// How long the user has to change their mind once everything is done.
pub const COUNTDOWN: Duration = Duration::from_secs(60);

/// What to do once every download is done. Not saved: like qBittorrent's, it lasts until reIDM
/// stops, so a shutdown chosen for one night's downloads can't surprise anyone the next day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WhenDone {
    #[default]
    None,
    /// Stop reIDM (the service, and the window with it).
    Exit,
    Sleep,
    Hibernate,
    Shutdown,
}

impl WhenDone {
    pub fn from_name(s: &str) -> Option<WhenDone> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "nothing" | "off" => Some(WhenDone::None),
            "exit" | "quit" => Some(WhenDone::Exit),
            "sleep" | "suspend" => Some(WhenDone::Sleep),
            "hibernate" => Some(WhenDone::Hibernate),
            "shutdown" | "shut-down" | "poweroff" => Some(WhenDone::Shutdown),
            _ => None,
        }
    }
}

/// A sleep inhibitor held: the system may sleep again once this is dropped.
pub struct Hold {
    _held: Box<dyn Send>,
}

impl Hold {
    /// Whatever keeps the system awake until it is dropped.
    pub fn new(held: impl Send + 'static) -> Hold {
        Hold { _held: Box::new(held) }
    }
}

/// Asks the operating system not to sleep.
pub trait Inhibitor: Send + Sync {
    fn hold(&self, why: &str) -> Result<Hold>;
}

/// Puts the computer to sleep, hibernates it or shuts it down.
pub trait PowerActions: Send + Sync {
    fn act(&self, what: WhenDone) -> Result<()>;
}

/// Whether to hold the computer awake, decided once a second from whether anything transfers.
#[derive(Default)]
pub(crate) struct Awake {
    last_busy: Option<Instant>,
}

impl Awake {
    /// Should a hold be in place now?
    pub(crate) fn wanted(&mut self, enabled: bool, busy: bool, now: Instant) -> bool {
        if !enabled {
            self.last_busy = None;
            return false;
        }
        if busy {
            self.last_busy = Some(now);
        }
        self.last_busy.is_some_and(|t| now.saturating_duration_since(t) < RELEASE_AFTER)
    }
}

/// The when-done action and its countdown.
#[derive(Default)]
pub(crate) struct Countdown {
    pub(crate) action: WhenDone,
    /// When it fires, as a monotonic instant and as unix seconds (for the window).
    pub(crate) fires: Option<(Instant, u64)>,
    /// Something has run since the action was chosen. Choosing an action with nothing to do
    /// starts no countdown: "when everything is done" needs something to be done first.
    seen_work: bool,
}

impl Countdown {
    pub(crate) fn set(&mut self, action: WhenDone) {
        *self = Countdown { action, ..Default::default() };
    }

    /// One look at the downloads. Returns the action when its countdown has run out (and
    /// forgets it: it happens once).
    pub(crate) fn tick(&mut self, idle: bool, now: Instant, unix_now: u64) -> Option<WhenDone> {
        if self.action == WhenDone::None {
            self.fires = None;
            return None;
        }
        if !idle {
            self.seen_work = true;
            self.fires = None;
            return None;
        }
        if !self.seen_work {
            return None;
        }
        match self.fires {
            None => {
                self.fires = Some((now + COUNTDOWN, unix_now + COUNTDOWN.as_secs()));
                None
            }
            Some((at, _)) if now >= at => {
                let a = self.action;
                self.set(WhenDone::None);
                Some(a)
            }
            Some(_) => None,
        }
    }
}

// ── the service's side ─────────────────────────────────────────────────

/// What one look at the downloads found.
pub(crate) struct Look {
    /// A transfer that keeps the computer awake (a download; a seed, if asked).
    pub(crate) awake: bool,
    /// The action whose countdown has just run out.
    pub(crate) fire: Option<WhenDone>,
}

impl crate::Manager {
    /// Choose what happens when every download is done; [`WhenDone::None`] cancels it.
    pub fn set_when_done(&self, action: WhenDone) {
        self.0.when_done.lock().unwrap().set(action);
    }

    /// The action chosen, and when its countdown ends (unix seconds) if it is running.
    pub fn when_done(&self) -> (WhenDone, Option<u64>) {
        let c = self.0.when_done.lock().unwrap();
        (c.action, c.fires.map(|f| f.1))
    }

    /// Look at the downloads once: whether anything keeps the computer awake, and whether the
    /// when-done countdown has run out.
    pub(crate) fn power_look(&self, now: Instant) -> Look {
        let (keep_seeding, action) = (self.0.settings.lock().unwrap().keep_awake_seeding, self.0.when_done.lock().unwrap().action);
        let transferring = !self.0.live.lock().unwrap().is_empty();
        // The list is only gone through when the answer depends on it, and not while
        // something transfers (which settles both questions).
        let (seeding, queued) = if transferring || (!keep_seeding && action == WhenDone::None) {
            (false, false)
        } else {
            let db = self.0.db.lock().unwrap();
            let seeding = keep_seeding && db.items.values().any(|i| i.status == crate::Status::Done && i.torrent.as_ref().is_some_and(|t| t.seeding));
            let queued = action != WhenDone::None && db.items.values().any(|i| i.status == crate::Status::Queued);
            (seeding, queued)
        };
        let fire = self.0.when_done.lock().unwrap().tick(!transferring && !queued, now, crate::now_secs());
        Look { awake: transferring || seeding, fire }
    }

    /// For as long as the service runs, once a second: hold `inhibitor` while downloads
    /// transfer (if the settings say to), and do the when-done action once they are all done —
    /// `on_exit` for [`WhenDone::Exit`], `actions` for the rest, after saving the list.
    pub fn spawn_power(&self, inhibitor: std::sync::Arc<dyn Inhibitor>, actions: std::sync::Arc<dyn PowerActions>, on_exit: std::sync::Arc<dyn Fn() + Send + Sync>) {
        let me = std::sync::Arc::downgrade(&self.0);
        tokio::spawn(async move {
            let mut awake = Awake::default();
            let mut hold: Option<Hold> = None;
            // A failure is said once, not every second.
            let mut refused = false;
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(inner) = me.upgrade() else { break };
                let m = crate::Manager(inner);
                let look = m.power_look(Instant::now());
                let enabled = m.0.settings.lock().unwrap().keep_awake;
                let want = awake.wanted(enabled, look.awake, Instant::now());
                if want && hold.is_none() && !refused {
                    let inh = inhibitor.clone();
                    match tokio::task::spawn_blocking(move || inh.hold("downloading")).await {
                        Ok(Ok(h)) => {
                            tracing::info!("keeping the computer awake while downloads run");
                            hold = Some(h);
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("cannot keep the computer awake: {e:#}");
                            refused = true;
                        }
                        Err(_) => refused = true,
                    }
                } else if !want {
                    refused = false;
                    if let Some(h) = hold.take() {
                        tracing::info!("the computer may sleep again");
                        // Letting go waits (briefly) for a helper process to end.
                        let _ = tokio::task::spawn_blocking(move || drop(h)).await;
                    }
                }
                if let Some(action) = look.fire {
                    tracing::info!("every download is done: {action:?}");
                    let flushed = { let m = m.clone(); tokio::task::spawn_blocking(move || m.flush()).await.unwrap_or(false) };
                    if !flushed {
                        tracing::warn!("the download list could not be saved before {action:?}");
                    }
                    if let Some(h) = hold.take() {
                        let _ = tokio::task::spawn_blocking(move || drop(h)).await;
                    }
                    if action == WhenDone::Exit {
                        on_exit();
                    } else {
                        let a = actions.clone();
                        match tokio::task::spawn_blocking(move || a.act(action)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                let msg = format!("everything was done, but {action:?} failed: {e:#}");
                                tracing::error!("{msg}");
                                m.0.warnings.lock().unwrap().push(msg);
                            }
                            Err(_) => {}
                        }
                    }
                }
            }
            // The service is ending: let go.
            if let Some(h) = hold.take() {
                let _ = tokio::task::spawn_blocking(move || drop(h)).await;
            }
        });
    }
}

// ── the operating system ───────────────────────────────────────────────

/// A helper process that holds the inhibitor for as long as it runs. Its standard input is a
/// pipe from reIDM: if reIDM dies without letting go (a crash, `kill -9`), the pipe closes and
/// the helper ends, so an inhibitor can never outlive the service.
pub struct ChildHold {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
}

impl ChildHold {
    pub fn spawn(program: &str, args: &[&str]) -> std::io::Result<ChildHold> {
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        // A helper that can't hold (no permission, no logind) says so and ends at once.
        std::thread::sleep(Duration::from_millis(150));
        if let Some(status) = child.try_wait()? {
            let mut why = String::new();
            if let Some(mut e) = child.stderr.take() {
                use std::io::Read;
                let _ = e.read_to_string(&mut why);
            }
            return Err(std::io::Error::other(format!("{program} ended at once ({status}): {}", why.trim())));
        }
        Ok(ChildHold { child, stdin })
    }

    /// The helper's process id, for tests.
    pub fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for ChildHold {
    fn drop(&mut self) {
        // Closing its input ends a helper that reads it; one that doesn't is stopped. Either
        // way it is waited for, so it can't linger as a zombie.
        drop(self.stdin.take());
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The real inhibitor of the system reIDM runs on.
pub struct SystemInhibitor;

impl Inhibitor for SystemInhibitor {
    #[allow(unused_variables)]
    fn hold(&self, why: &str) -> Result<Hold> {
        #[cfg(target_os = "linux")]
        {
            // logind's sleep lock, held for as long as `cat` reads reIDM's pipe.
            let why = format!("--why={why}");
            let h = ChildHold::spawn("systemd-inhibit", &["--what=sleep", "--who=reIDM", &why, "--mode=block", "cat"])
                .context("cannot ask the system to stay awake (systemd-inhibit)")?;
            Ok(Hold::new(h))
        }
        #[cfg(target_os = "macos")]
        {
            // `-w`: caffeinate ends by itself when reIDM does.
            let pid = std::process::id().to_string();
            let h = ChildHold::spawn("caffeinate", &["-i", "-w", &pid]).context("cannot ask the system to stay awake (caffeinate)")?;
            Ok(Hold::new(h))
        }
        #[cfg(windows)]
        {
            Ok(Hold::new(windows::ThreadHold::start()?))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        bail!("keeping this system awake is not supported")
    }
}

/// The system's own sleep, hibernate and shut-down.
pub struct SystemPower;

/// The command that does `what` on Linux and macOS.
#[cfg_attr(windows, allow(dead_code))]
fn os_command(what: WhenDone) -> Option<(&'static str, &'static [&'static str])> {
    if cfg!(target_os = "macos") {
        match what {
            // macOS has no hibernate of its own to ask for: sleep, which hibernates when the
            // system is set up to (`pmset hibernatemode`).
            WhenDone::Sleep | WhenDone::Hibernate => Some(("pmset", &["sleepnow"])),
            WhenDone::Shutdown => Some(("osascript", &["-e", "tell application \"System Events\" to shut down"])),
            _ => None,
        }
    } else {
        // logind, which lets the user at the console do these without a password.
        match what {
            WhenDone::Sleep => Some(("systemctl", &["suspend"])),
            WhenDone::Hibernate => Some(("systemctl", &["hibernate"])),
            WhenDone::Shutdown => Some(("systemctl", &["poweroff"])),
            _ => None,
        }
    }
}

impl PowerActions for SystemPower {
    fn act(&self, what: WhenDone) -> Result<()> {
        #[cfg(windows)]
        {
            windows::act(what)
        }
        #[cfg(not(windows))]
        {
            let Some((program, args)) = os_command(what) else { bail!("nothing to do for {what:?}") };
            let out = std::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).output().with_context(|| format!("cannot run {program}"))?;
            if !out.status.success() {
                bail!("{program} {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::WhenDone;
    use anyhow::{bail, Result};
    use std::ffi::c_void;

    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(h: *mut c_void) -> i32;
    }

    #[repr(C)]
    struct Luid {
        low: u32,
        high: i32,
    }

    #[repr(C)]
    struct TokenPrivileges {
        count: u32,
        luid: Luid,
        attributes: u32,
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut Luid) -> i32;
        fn AdjustTokenPrivileges(token: *mut c_void, disable_all: i32, new: *const TokenPrivileges, len: u32, prev: *mut c_void, ret_len: *mut u32) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn ExitWindowsEx(flags: u32, reason: u32) -> i32;
    }

    #[link(name = "powrprof")]
    extern "system" {
        fn SetSuspendState(hibernate: u8, force: u8, wake_events_disabled: u8) -> u8;
    }

    /// `SetThreadExecutionState` belongs to the thread that calls it, and lapses when that
    /// thread ends: so a thread of its own holds it, and ends (letting go) when told to.
    pub(super) struct ThreadHold {
        stop: Option<std::sync::mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl ThreadHold {
        pub(super) fn start() -> Result<ThreadHold> {
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let (ok_tx, ok_rx) = std::sync::mpsc::channel();
            let thread = std::thread::Builder::new().name("reidm-awake".into()).spawn(move || {
                // SAFETY: plain flags in, the previous state out.
                let ok = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) } != 0;
                let _ = ok_tx.send(ok);
                // Until the sender is dropped.
                let _ = rx.recv();
                // SAFETY: as above.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            })?;
            if !ok_rx.recv().unwrap_or(false) {
                drop(tx);
                let _ = thread.join();
                bail!("Windows refused to stay awake (SetThreadExecutionState)");
            }
            Ok(ThreadHold { stop: Some(tx), thread: Some(thread) })
        }
    }

    impl Drop for ThreadHold {
        fn drop(&mut self) {
            drop(self.stop.take());
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Sleep and shutting down need the shutdown privilege switched on in the process's token.
    fn enable_shutdown_privilege() -> Result<()> {
        const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
        const TOKEN_QUERY: u32 = 0x0008;
        const SE_PRIVILEGE_ENABLED: u32 = 0x0002;
        let name: Vec<u16> = "SeShutdownPrivilege".encode_utf16().chain(Some(0)).collect();
        let mut token = std::ptr::null_mut();
        // SAFETY: out-pointers to locals; the token handle is closed below.
        unsafe {
            if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
                bail!("cannot open the process token: {}", std::io::Error::last_os_error());
            }
            let mut tp = TokenPrivileges { count: 1, luid: Luid { low: 0, high: 0 }, attributes: SE_PRIVILEGE_ENABLED };
            let ok = LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut tp.luid) != 0
                && AdjustTokenPrivileges(token, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0;
            let err = std::io::Error::last_os_error();
            CloseHandle(token);
            if !ok {
                bail!("not allowed to shut down or sleep: {err}");
            }
        }
        Ok(())
    }

    pub(super) fn act(what: WhenDone) -> Result<()> {
        enable_shutdown_privilege()?;
        // SAFETY: plain values in, a success flag out.
        let ok = unsafe {
            match what {
                WhenDone::Sleep => SetSuspendState(0, 0, 0) != 0,
                WhenDone::Hibernate => SetSuspendState(1, 0, 0) != 0,
                // EWX_SHUTDOWN | EWX_POWEROFF; planned, by an application.
                WhenDone::Shutdown => ExitWindowsEx(0x0000_0001 | 0x0000_0008, 0x8000_0000 | 0x0004_0000) != 0,
                _ => bail!("nothing to do for {what:?}"),
            }
        };
        if !ok {
            bail!("Windows refused: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awake_is_held_while_busy_and_for_a_while_after() {
        let t0 = Instant::now();
        let mut a = Awake::default();
        assert!(!a.wanted(true, false, t0), "nothing ran: no hold");
        assert!(a.wanted(true, true, t0));
        assert!(a.wanted(true, false, t0 + Duration::from_secs(29)), "the next in the queue may be a moment away");
        assert!(!a.wanted(true, false, t0 + RELEASE_AFTER));
        assert!(a.wanted(true, true, t0 + Duration::from_secs(40)));
        assert!(!a.wanted(false, true, t0 + Duration::from_secs(41)), "switched off: let go at once");
    }

    #[test]
    fn the_countdown_starts_when_all_is_done_and_stops_when_something_starts() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut c = Countdown::default();
        c.set(WhenDone::Sleep);
        assert_eq!(c.tick(true, s(0), 1000), None);
        assert!(c.fires.is_none(), "chosen with nothing to do: nothing happens");
        assert_eq!(c.tick(false, s(1), 1001), None);
        assert_eq!(c.tick(true, s(2), 1002), None);
        assert_eq!(c.fires.map(|f| f.1), Some(1062), "a minute from when the last one finished");
        // Something starts again: the countdown stops, the choice stays.
        assert_eq!(c.tick(false, s(30), 1030), None);
        assert!(c.fires.is_none());
        assert_eq!(c.action, WhenDone::Sleep);
        assert_eq!(c.tick(true, s(40), 1040), None);
        assert_eq!(c.tick(true, s(99), 1099), None);
        assert_eq!(c.tick(true, s(100), 1100), Some(WhenDone::Sleep));
        assert_eq!(c.action, WhenDone::None, "it happens once");
        assert_eq!(c.tick(true, s(200), 1200), None);
        c.set(WhenDone::None);
        assert_eq!(c.tick(false, s(201), 1201), None);
    }

    #[test]
    fn each_action_is_the_systems_own_command() {
        if cfg!(target_os = "macos") {
            assert_eq!(os_command(WhenDone::Sleep), Some(("pmset", &["sleepnow"][..])));
        } else {
            assert_eq!(os_command(WhenDone::Sleep), Some(("systemctl", &["suspend"][..])));
            assert_eq!(os_command(WhenDone::Hibernate), Some(("systemctl", &["hibernate"][..])));
            assert_eq!(os_command(WhenDone::Shutdown), Some(("systemctl", &["poweroff"][..])));
        }
        assert_eq!(os_command(WhenDone::Exit), None, "exit is the service's own business");
        assert_eq!(os_command(WhenDone::None), None);
        assert_eq!(WhenDone::from_name("Shutdown"), Some(WhenDone::Shutdown));
        assert_eq!(WhenDone::from_name("reboot"), None);
    }

    /// Only the helper's life is checked — with `cat` standing in for `systemd-inhibit`, so
    /// nothing is ever asked of the real power manager.
    #[cfg(unix)]
    #[test]
    fn a_held_helper_ends_when_let_go() {
        let h = ChildHold::spawn("cat", &[]).unwrap();
        let pid = h.id() as i32;
        // SAFETY: signal 0 only checks that the process exists.
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "running while held");
        let t = std::time::Instant::now();
        drop(h);
        assert!(t.elapsed() < Duration::from_secs(2), "letting go never blocks for long");
        // Reaped, not left a zombie: the pid no longer exists for us.
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "the helper is gone");
        // A helper that fails at once is an error, not a hold that holds nothing.
        let e = ChildHold::spawn("sh", &["-c", "echo no permission >&2; exit 1"]).err().unwrap();
        assert!(e.to_string().contains("no permission"), "{e}");
    }
}
