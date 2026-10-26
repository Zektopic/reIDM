//! *When everything is done*: exit, sleep, hibernate or shut down once nothing is downloading.
//!
//! The service owns it (it has to work with the window closed) and counts down 60 seconds
//! before acting; the window chooses the action, shows the countdown with a Cancel button, and
//! closes itself when the service exits. Not kept between sessions, as in qBittorrent, so nobody's
//! computer shuts down unexpectedly next week.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    #[default]
    None,
    Exit,
    Sleep,
    Hibernate,
    Shutdown,
    /// Something a newer service does that this window has no name for.
    #[serde(other)]
    Other,
}

impl Action {
    /// The choices, in menu order.
    pub const ALL: [Action; 5] = [Action::None, Action::Exit, Action::Sleep, Action::Hibernate, Action::Shutdown];

    pub fn wire(self) -> &'static str {
        match self {
            Action::None | Action::Other => "none",
            Action::Exit => "exit",
            Action::Sleep => "sleep",
            Action::Hibernate => "hibernate",
            Action::Shutdown => "shutdown",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::None | Action::Other => "Do nothing",
            Action::Exit => "Exit reIDM",
            Action::Sleep => "Sleep",
            Action::Hibernate => "Hibernate",
            Action::Shutdown => "Shut down the computer",
        }
    }

    /// What a toast says once it is chosen.
    pub fn chosen(self) -> &'static str {
        match self {
            Action::None | Action::Other => "Nothing happens when the downloads are done",
            Action::Exit => "reIDM exits when the downloads are done",
            Action::Sleep => "The computer goes to sleep when the downloads are done",
            Action::Hibernate => "The computer hibernates when the downloads are done",
            Action::Shutdown => "The computer shuts down when the downloads are done",
        }
    }
}

/// `GET /api/when-done`, and `when_done` in `/api/status`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct WhenDone {
    #[serde(default)]
    pub action: Action,
    /// When the countdown ends, in unix seconds; `None` while nothing is counting down.
    #[serde(default)]
    pub fires_at: Option<u64>,
}

impl WhenDone {
    /// The countdown's text and the seconds left, while one runs.
    pub fn countdown(&self, now: u64) -> Option<(u64, String)> {
        let at = self.fires_at?;
        if self.action == Action::None {
            return None;
        }
        let left = at.saturating_sub(now);
        let secs = if left == 1 { "1 second".to_string() } else { format!("{left} seconds") };
        let text = match self.action {
            Action::Exit => format!("reIDM exits in {secs}."),
            Action::Sleep => format!("The computer goes to sleep in {secs}."),
            Action::Hibernate => format!("The computer hibernates in {secs}."),
            Action::Shutdown => format!("The computer shuts down in {secs}."),
            Action::None | Action::Other => format!("reIDM acts in {secs}."),
        };
        Some((left, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_countdown_says_what_happens_and_when() {
        let w = WhenDone { action: Action::Shutdown, fires_at: Some(1_000) };
        assert_eq!(w.countdown(942), Some((58, "The computer shuts down in 58 seconds.".into())));
        assert_eq!(w.countdown(999).unwrap().1, "The computer shuts down in 1 second.");
        assert_eq!(w.countdown(1_005).unwrap().0, 0, "late, not negative");
        assert_eq!(WhenDone { action: Action::Sleep, fires_at: None }.countdown(0), None, "no countdown yet");
        assert_eq!(WhenDone { action: Action::None, fires_at: Some(5) }.countdown(0), None);
    }

    #[test]
    fn the_service_s_answer_reads_as_the_spec_says() {
        let w: WhenDone = serde_json::from_str(r#"{"action":"hibernate","fires_at":1790000060}"#).unwrap();
        assert_eq!(w, WhenDone { action: Action::Hibernate, fires_at: Some(1_790_000_060) });
        let w: WhenDone = serde_json::from_str(r#"{"action":"reboot","fires_at":null}"#).unwrap();
        assert_eq!(w.action, Action::Other, "a newer action still loads");
        assert_eq!(w.action.wire(), "none");
        for a in Action::ALL {
            let back: Action = serde_json::from_value(serde_json::json!(a.wire())).unwrap();
            assert_eq!(back, a);
        }
    }
}
