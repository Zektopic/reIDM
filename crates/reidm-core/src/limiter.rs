//! Speed limiting: a token bucket that every connection draws from.
//!
//! One bucket is shared by every download for the global limit, and each download can have
//! its own as well; a chunk is paced by both. The bucket is allowed to go into debt, which is
//! what keeps concurrent connections honest: if eight connections each take 64 KiB at the same
//! moment, the eighth waits for all eight chunks' worth of time, not one.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How often a long wait looks at the cancel flag, so Pause still takes effect under a very low
/// limit.
const TICK: Duration = Duration::from_millis(250);

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

    /// Account for `n` bytes and wait as long as the limit requires. Returns early once `cancel`
    /// is set, so a paused download does not sit out a long wait first.
    pub async fn take(&self, n: u64, cancel: &AtomicBool) {
        let wait = self.reserve(n);
        if wait.is_zero() {
            return;
        }
        let deadline = Instant::now() + wait;
        while !cancel.load(Ordering::Relaxed) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            tokio::time::sleep(left.min(TICK)).await;
        }
    }

    /// The bookkeeping half of [`Limiter::take`]: charge `n` bytes and say how long to wait.
    fn reserve(&self, n: u64) -> Duration {
        let rate = self.rate();
        if rate == 0 {
            return Duration::ZERO;
        }
        let mut b = self.bucket.lock().unwrap();
        let now = Instant::now();
        let refill = now.duration_since(b.at).as_secs_f64() * rate as f64;
        b.tokens = (b.tokens + refill).min(capacity(rate));
        b.at = now;
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
}
