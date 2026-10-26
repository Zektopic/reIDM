//! Speed limiting: a token bucket that every connection draws from.
//!
//! One bucket is shared by every download for the global limit, and each download can have
//! its own as well; a chunk is paced by both. The bucket is allowed to go into debt, which is
//! what keeps concurrent connections honest: if eight connections each take 64 KiB at the same
//! moment, the eighth waits for all eight chunks' worth of time, not one.
//!
//! Bytes that are paced elsewhere — yt-dlp's own `--limit-rate`, librqbit's own bucket — are
//! [charged](Limiter::charge) to the global bucket after the fact, so the HTTP downloads give
//! way to them and the global limit holds for everything together.
//!
//! Connections get bandwidth in proportion to how often they ask, so a download with eight
//! connections gets more of a shared limit than one with two.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The most that [`Limiter::charge`] may put the bucket in debt, in seconds of the rate: enough
/// to make the HTTP downloads give way, not so much that they stall for long after a burst.
const MAX_CHARGED_DEBT: f64 = 1.0;

pub struct Limiter {
    /// Bytes per second; 0 means unlimited.
    rate: AtomicU64,
    bucket: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

/// Half a second of burst: enough to absorb the uneven sizes of network reads, small enough
/// that a limit is visibly a limit within a second of being set.
fn capacity(rate: u64) -> f64 {
    (rate as f64 / 2.0).max(8192.0)
}

impl Limiter {
    pub fn new(bytes_per_sec: u64) -> Self {
        Self { rate: AtomicU64::new(bytes_per_sec), bucket: Mutex::new(Bucket { tokens: capacity(bytes_per_sec), at: Instant::now() }) }
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
        let mut b = self.bucket.lock().unwrap();
        // A lower limit must bite now, not after the old, larger burst has been spent.
        b.tokens = b.tokens.min(capacity(bytes_per_sec));
    }

    /// Account for `n` bytes and wait as long as the limit requires. Returns early once `stop`
    /// completes, so a paused download does not sit out a long wait first — and gives back the
    /// time it did not wait, or the debt would hold up every other download sharing the bucket.
    pub async fn take(&self, n: u64, stop: impl Future<Output = ()>) {
        let wait = self.reserve(n);
        if wait.is_zero() {
            return;
        }
        let deadline = tokio::time::Instant::now() + wait;
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {}
            _ = stop => self.refund(deadline.saturating_duration_since(tokio::time::Instant::now())),
        }
    }

    /// Count `n` bytes that were transferred without waiting here — paced by another program's
    /// own limit — so that whatever does wait here gives way to them.
    pub fn charge(&self, n: u64) {
        let rate = self.rate();
        if rate == 0 {
            return;
        }
        let mut b = self.refill(rate);
        b.tokens = (b.tokens - n as f64).max(-(rate as f64) * MAX_CHARGED_DEBT);
    }

    fn refund(&self, unused: Duration) {
        let rate = self.rate();
        if rate == 0 || unused.is_zero() {
            return;
        }
        let mut b = self.bucket.lock().unwrap();
        b.tokens = (b.tokens + unused.as_secs_f64() * rate as f64).min(capacity(rate));
    }

    /// The bucket, topped up for the time since it was last used.
    fn refill(&self, rate: u64) -> std::sync::MutexGuard<'_, Bucket> {
        let mut b = self.bucket.lock().unwrap();
        let now = Instant::now();
        let refill = now.duration_since(b.at).as_secs_f64() * rate as f64;
        b.tokens = (b.tokens + refill).min(capacity(rate));
        b.at = now;
        b
    }

    /// The bookkeeping half of [`Limiter::take`]: charge `n` bytes and say how long to wait.
    fn reserve(&self, n: u64) -> Duration {
        let rate = self.rate();
        if rate == 0 {
            return Duration::ZERO;
        }
        let mut b = self.refill(rate);
        b.tokens -= n as f64;
        if b.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-b.tokens / rate as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let l = Limiter::new(0);
        assert_eq!(l.reserve(u64::MAX / 2), Duration::ZERO);
    }

    #[test]
    fn debt_accumulates_across_callers() {
        let l = Limiter::new(100_000);
        // The first half-second is burst.
        assert_eq!(l.reserve(50_000), Duration::ZERO);
        // Two callers taking 100 kB each now owe one and two seconds respectively.
        let a = l.reserve(100_000);
        let b = l.reserve(100_000);
        assert!(a > Duration::from_millis(900) && a < Duration::from_millis(1100), "{a:?}");
        assert!(b > Duration::from_millis(1900) && b < Duration::from_millis(2100), "{b:?}");
    }

    #[test]
    fn lowering_the_rate_drops_the_old_burst() {
        let l = Limiter::new(10_000_000);
        l.set_rate(10_000);
        // Had the 5 MB burst survived, this would not wait at all.
        assert!(l.reserve(100_000) > Duration::from_secs(8));
    }

    #[tokio::test]
    async fn a_wait_cut_short_gives_its_time_back() {
        let l = Limiter::new(100_000);
        l.reserve(50_000); // spend the burst
        // Ten seconds' worth, abandoned at once (a pause): the debt must not stay behind.
        l.take(1_000_000, async {}).await;
        assert!(l.reserve(10_000) < Duration::from_millis(300), "the abandoned wait still holds up the next caller");
    }

    #[test]
    fn charged_bytes_make_others_wait_but_not_for_ever() {
        let l = Limiter::new(100_000);
        l.charge(50_000);
        assert!(l.reserve(50_000) > Duration::from_millis(400), "another program's bytes count against the limit");
        // However much is charged, the debt is capped.
        l.charge(100_000_000);
        assert!(l.reserve(0) < Duration::from_millis(1100));
        Limiter::new(0).charge(u64::MAX); // unlimited: nothing to account
    }
}
