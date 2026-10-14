//! The scheduler: start the scheduled queue at a time of day, and optionally stop it again.
//!
//! IDM's scheduler works on queues; reIDM has one scheduled queue, which is every download added
//! with *Download Later* (or moved into it). The clock is local time, because "start at 2am" means
//! the user's 2am.
//!
//! The decision is a pure function of two instants — the previous check and this one — so it
//! is testable without a clock, and a machine that slept through a start time still acts on
//! wake-up instead of waiting a day.

use anyhow::{bail, Result};
use chrono::{Datelike, NaiveDateTime, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub enabled: bool,
    /// `HH:MM`, local time, to start the scheduled queue.
    #[serde(default)]
    pub start: Option<String>,
    /// `HH:MM`, local time, to stop it. Earlier than `start` means the next morning.
    #[serde(default)]
    pub stop: Option<String>,
    /// Days the schedule runs, Monday = 0 … Sunday = 6. Empty means every day.
    #[serde(default)]
    pub days: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
}

/// `"7:05"`, `"07:05"` and `"23:59"` parse; anything else is `None`.
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.trim().split_once(':')?;
    NaiveTime::from_hms_opt(h.trim().parse().ok()?, m.trim().parse().ok()?, 0)
}

fn day_index(d: Weekday) -> u8 {
    d.num_days_from_monday() as u8
}

impl Schedule {
    /// Refuse a schedule that could never fire, rather than saving it and silently doing nothing.
    pub fn validate(&self) -> Result<()> {
        for (what, t) in [("start", &self.start), ("stop", &self.stop)] {
            if let Some(t) = t {
                if parse_hhmm(t).is_none() {
                    bail!("the {what} time {t:?} is not a time of day (use HH:MM)");
                }
            }
        }
        if let Some(d) = self.days.iter().find(|d| **d > 6) {
            bail!("day {d} is not a weekday (0 = Monday … 6 = Sunday)");
        }
        if self.enabled && self.start.is_none() && self.stop.is_none() {
            bail!("a schedule needs a start time, a stop time, or both");
        }
        Ok(())
    }

    fn runs_on(&self, d: Weekday) -> bool {
        self.days.is_empty() || self.days.contains(&day_index(d))
    }

    /// What the clock moving from `prev` to `now` asks for, if anything. When both a start and
    /// a stop fall inside the interval (the machine slept through both), the later one wins:
    /// it is the state that should hold *now*.
    pub fn due(&self, prev: NaiveDateTime, now: NaiveDateTime) -> Option<Action> {
        if !self.enabled || now <= prev {
            return None;
        }
        let start = self.start.as_deref().and_then(parse_hhmm);
        let stop = self.stop.as_deref().and_then(parse_hhmm);
        // An overnight window (start 23:00, stop 06:00) stops on the morning after a start day.
        let overnight = matches!((start, stop), (Some(a), Some(b)) if b <= a);

        let mut last: Option<(NaiveDateTime, Action)> = None;
        let mut consider = |at: NaiveDateTime, a: Action| {
            if prev < at && at <= now && last.is_none_or(|(l, _)| at >= l) {
                last = Some((at, a));
            }
        };
        // Look back one extra day for an overnight stop, and no further than a week overall:
        // after a longer sleep the right answer is still whichever boundary came last.
        let first = prev.date().pred_opt().unwrap_or(prev.date()).max(now.date() - chrono::Days::new(8));
        let mut d = first;
        while d <= now.date() {
            if let Some(t) = start {
                if self.runs_on(d.weekday()) {
                    consider(d.and_time(t), Action::Start);
                }
            }
            if let Some(t) = stop {
                let owner = if overnight { d.pred_opt().unwrap_or(d) } else { d };
                if self.runs_on(owner.weekday()) {
                    consider(d.and_time(t), Action::Stop);
                }
            }
            match d.succ_opt() {
                Some(n) => d = n,
                None => break,
            }
        }
        last.map(|(_, a)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    fn sched(start: Option<&str>, stop: Option<&str>, days: &[u8]) -> Schedule {
        Schedule { enabled: true, start: start.map(Into::into), stop: stop.map(Into::into), days: days.to_vec() }
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_hhmm("7:05"), NaiveTime::from_hms_opt(7, 5, 0));
        assert_eq!(parse_hhmm(" 23:59 "), NaiveTime::from_hms_opt(23, 59, 0));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("noon"), None);
    }

    #[test]
    fn fires_once_when_the_start_time_is_crossed() {
        let s = sched(Some("02:00"), None, &[]);
        // 2026-10-15 is a Thursday.
        assert_eq!(s.due(at(2026, 10, 15, 1, 59), at(2026, 10, 15, 2, 0)), Some(Action::Start));
        // The next check, a moment later, must not fire again.
        assert_eq!(s.due(at(2026, 10, 15, 2, 0), at(2026, 10, 15, 2, 1)), None);
    }

    #[test]
    fn disabled_or_backwards_clock_does_nothing() {
        let mut s = sched(Some("02:00"), None, &[]);
        s.enabled = false;
        assert_eq!(s.due(at(2026, 10, 15, 1, 0), at(2026, 10, 15, 3, 0)), None);
        let s = sched(Some("02:00"), None, &[]);
        assert_eq!(s.due(at(2026, 10, 15, 3, 0), at(2026, 10, 15, 1, 0)), None);
    }

    #[test]
    fn respects_the_chosen_days() {
        // Thursday is 3; only Mondays are enabled.
        let s = sched(Some("02:00"), None, &[0]);
        assert_eq!(s.due(at(2026, 10, 15, 1, 0), at(2026, 10, 15, 3, 0)), None);
        // 2026-10-19 is a Monday.
        assert_eq!(s.due(at(2026, 10, 19, 1, 0), at(2026, 10, 19, 3, 0)), Some(Action::Start));
    }

    #[test]
    fn a_long_sleep_acts_on_the_latest_boundary() {
        let s = sched(Some("01:00"), Some("06:00"), &[]);
        // Slept from before the start until after the stop: the queue should end up stopped.
        assert_eq!(s.due(at(2026, 10, 15, 0, 30), at(2026, 10, 15, 7, 0)), Some(Action::Stop));
        // Woke between them: it should be running.
        assert_eq!(s.due(at(2026, 10, 15, 0, 30), at(2026, 10, 15, 3, 0)), Some(Action::Start));
    }

    #[test]
    fn overnight_window_stops_the_next_morning() {
        // Fridays only, 23:00 to 06:00. 2026-10-16 is a Friday.
        let s = sched(Some("23:00"), Some("06:00"), &[4]);
        assert_eq!(s.due(at(2026, 10, 16, 22, 59), at(2026, 10, 16, 23, 0)), Some(Action::Start));
        // The stop lands on Saturday, which is not itself enabled, but belongs to Friday's run.
        assert_eq!(s.due(at(2026, 10, 17, 5, 59), at(2026, 10, 17, 6, 0)), Some(Action::Stop));
        // Friday morning's 06:00 belongs to Thursday, which is not enabled.
        assert_eq!(s.due(at(2026, 10, 16, 5, 59), at(2026, 10, 16, 6, 0)), None);
    }

    #[test]
    fn validation_catches_unusable_schedules() {
        assert!(sched(Some("25:00"), None, &[]).validate().is_err());
        assert!(sched(Some("01:00"), None, &[7]).validate().is_err());
        assert!(sched(None, None, &[]).validate().is_err());
        assert!(sched(Some("01:00"), Some("02:00"), &[0, 6]).validate().is_ok());
    }
}
