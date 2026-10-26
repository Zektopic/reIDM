//! Telling the user a download finished or failed: a desktop notification, or the *Download
//! complete* window, as Options › General says.
//!
//! A notification can't be relied on alone: Windows lets people switch them off for every app,
//! a Linux desktop may have no notification server, and none of that is an error the sender
//! sees. So the window asks Windows first whether it would show one, and whatever can't be
//! shown falls back to the *Download complete* window (or, for a failure, a message in the
//! window).

use crate::model::{Item, Status};
use std::collections::{HashMap, HashSet};

/// What a finished download does, from Options › General.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnFinish {
    /// The *Download complete* window.
    Dialog,
    Notification,
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Finished(u64),
    Failed(u64),
}

/// Finds downloads that have just finished or failed. The first look only records what each
/// download is doing: what finished before the window opened is old news.
#[derive(Default)]
pub struct Watch {
    last: HashMap<u64, Status>,
    primed: bool,
}

impl Watch {
    pub fn update<'a>(&mut self, items: impl Iterator<Item = &'a Item>) -> Vec<Event> {
        let mut events = Vec::new();
        let mut seen = HashSet::new();
        for it in items {
            seen.insert(it.id);
            let before = self.last.insert(it.id, it.status.clone());
            if !self.primed {
                continue;
            }
            match (&it.status, before) {
                (Status::Done, Some(b)) if b != Status::Done => events.push(Event::Finished(it.id)),
                (Status::Failed(_), Some(b)) if !matches!(b, Status::Failed(_)) => events.push(Event::Failed(it.id)),
                _ => {}
            }
        }
        self.last.retain(|id, _| seen.contains(id));
        self.primed = true;
        events
    }
}

/// One notification: its title, its text, and the downloads it is about (for the fallback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub title: String,
    pub body: String,
    pub ids: Vec<u64>,
    pub failed: bool,
}

/// More than this many at once become one summary, rather than a stack of notifications.
const ONE_EACH: usize = 3;

/// The notifications for a set of events. `name` finds a download's file name; `why` a failed
/// one's error.
pub fn messages(events: &[Event], name: impl Fn(u64) -> Option<String>, why: impl Fn(u64) -> Option<String>) -> Vec<Message> {
    let finished: Vec<u64> = events.iter().filter_map(|e| if let Event::Finished(id) = e { Some(*id) } else { None }).collect();
    let failed: Vec<u64> = events.iter().filter_map(|e| if let Event::Failed(id) = e { Some(*id) } else { None }).collect();
    let mut out = Vec::new();
    let list = |ids: &[u64]| {
        let mut names: Vec<String> = ids.iter().take(ONE_EACH).filter_map(|id| name(*id)).collect();
        if ids.len() > ONE_EACH {
            names.push(format!("and {} more", ids.len() - ONE_EACH));
        }
        names.join("\n")
    };
    if finished.len() > ONE_EACH {
        out.push(Message { title: format!("{} downloads finished", finished.len()), body: list(&finished), ids: finished, failed: false });
    } else {
        for id in finished {
            out.push(Message { title: "Download complete".into(), body: name(id).unwrap_or_default(), ids: vec![id], failed: false });
        }
    }
    if failed.len() > ONE_EACH {
        out.push(Message { title: format!("{} downloads failed", failed.len()), body: list(&failed), ids: failed, failed: true });
    } else {
        for id in failed {
            let body = match (name(id), why(id)) {
                (Some(n), Some(w)) => format!("{n}\n{w}"),
                (n, w) => n.or(w).unwrap_or_default(),
            };
            out.push(Message { title: "Download failed".into(), body, ids: vec![id], failed: true });
        }
    }
    out
}

/// The app id Windows shows notifications under. reIDM's installer registers none of its own,
/// so this is the one every unpackaged program borrows (`notify-rust`'s default too); it must be
/// the same one asked about in `windows_allows`.
#[cfg(windows)]
const WINDOWS_APP_ID: &str = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

/// Whether Windows would show the notification. It can be switched off for every app, or by
/// policy, and sending one then still "succeeds".
#[cfg(windows)]
fn windows_allows() -> bool {
    use windows::core::HSTRING;
    use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(WINDOWS_APP_ID)).and_then(|n| n.Setting()).is_ok_and(|s| s == NotificationSetting::Enabled)
}

/// Show a desktop notification. Blocks for a D-Bus or WinRT round trip: call it off the
/// window's thread.
pub fn show(m: &Message) -> Result<(), String> {
    let mut n = notify_rust::Notification::new();
    n.summary(&m.title).body(&m.body).appname("reIDM");
    #[cfg(windows)]
    {
        if !windows_allows() {
            return Err("Windows notifications are switched off".into());
        }
        n.app_id(WINDOWS_APP_ID);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    n.icon("reidm");
    n.show().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::tests::row;

    #[test]
    fn only_changes_after_the_first_look_are_events() {
        let mut w = Watch::default();
        let a = [row(1, "a.zip", "Done", 1000), row(2, "b.zip", "Running", 10), row(3, "c.zip", "Running", 0)];
        assert!(w.update(a.iter()).is_empty(), "finished before the window opened");
        let b = [row(1, "a.zip", "Done", 1000), row(2, "b.zip", "Done", 1000), row(3, "c.zip", "Failed", 0), row(4, "d.zip", "Done", 1000)];
        assert_eq!(w.update(b.iter()), vec![Event::Finished(2), Event::Failed(3)], "a new download already done is not news either");
        assert!(w.update(b.iter()).is_empty(), "nothing twice");
        // Failing again, or a retry that fails, is said once per failure.
        let c = [row(3, "c.zip", "Running", 0)];
        assert!(w.update(c.iter()).is_empty());
        let d = [row(3, "c.zip", "Failed", 0)];
        assert_eq!(w.update(d.iter()), vec![Event::Failed(3)]);
    }

    #[test]
    fn many_at_once_become_one_notification() {
        let name = |id: u64| Some(format!("file-{id}.zip"));
        let why = |_| Some("HTTP 404".to_string());
        let one = messages(&[Event::Finished(1), Event::Failed(2)], name, why);
        assert_eq!(one.len(), 2);
        assert_eq!((one[0].title.as_str(), one[0].body.as_str()), ("Download complete", "file-1.zip"));
        assert_eq!(one[1].body, "file-2.zip\nHTTP 404");
        assert!(one[1].failed);

        let events: Vec<Event> = (1..=12).map(Event::Finished).collect();
        let many = messages(&events, name, why);
        assert_eq!(many.len(), 1);
        assert_eq!(many[0].title, "12 downloads finished");
        assert_eq!(many[0].body, "file-1.zip\nfile-2.zip\nfile-3.zip\nand 9 more");
        assert_eq!(many[0].ids.len(), 12, "all of them fall back to the window if it can't be shown");
    }
}
