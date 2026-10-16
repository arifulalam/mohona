//! Speed limits.
//!
//! A [`RateLimit`] paces bytes: every connection that receives data asks it
//! how long to wait first. One limit can be shared by any number of
//! downloads (a global limit, a queue's limit) and its rate can be changed
//! while they run. A download may have several limits at once (global, queue,
//! download); it then goes at the slowest of them.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How much unused time a limit remembers. This lets a connection that was
/// idle for a moment catch up a little instead of losing that time.
const CREDIT: Duration = Duration::from_millis(50);

/// A shared speed limit in bytes per second. `0` means unlimited.
#[derive(Debug)]
pub struct RateLimit {
    rate: AtomicU64,
    /// When the next byte may be sent, as if the limit were a single pipe.
    next: Mutex<Instant>,
}

impl RateLimit {
    /// A limit of `bytes_per_second` (`0` for none).
    pub fn new(bytes_per_second: u64) -> Arc<Self> {
        Arc::new(Self {
            rate: AtomicU64::new(bytes_per_second),
            next: Mutex::new(Instant::now()),
        })
    }

    /// Change the limit. Running downloads follow at once.
    pub fn set(&self, bytes_per_second: u64) {
        self.rate.store(bytes_per_second, Ordering::Relaxed);
    }

    /// The current limit in bytes per second (`0` is unlimited).
    pub fn get(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Book `bytes` and say how long the caller must wait before using them.
    fn book(&self, bytes: u64) -> Duration {
        let rate = self.get();
        let now = Instant::now();
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        if rate == 0 {
            *next = now;
            return Duration::ZERO;
        }
        let start = (*next).max(now.checked_sub(CREDIT).unwrap_or(now));
        // The bytes count as delivered once the pipe has carried them, so the
        // wait runs to the end of their slot, not its start.
        *next = start + Duration::from_secs_f64(bytes as f64 / rate as f64);
        (*next).saturating_duration_since(now)
    }
}

/// All the limits that apply to one download.
#[derive(Clone, Debug, Default)]
pub(crate) struct Limits(pub Vec<Arc<RateLimit>>);

impl Limits {
    /// Wait until `bytes` may be used under every limit.
    pub async fn acquire(&self, bytes: usize) {
        let wait = self
            .0
            .iter()
            .map(|limit| limit.book(bytes as u64))
            .max()
            .unwrap_or_default();
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let limit = RateLimit::new(0);
        assert_eq!(limit.book(10_000_000), Duration::ZERO);
    }

    #[test]
    fn waits_add_up_to_the_rate() {
        let limit = RateLimit::new(1000);
        // The first 50 ms worth are covered by the credit, the rest queues.
        let waits: Vec<Duration> = (0..10).map(|_| limit.book(100)).collect();
        let last = *waits.last().unwrap();
        // 10 x 100 bytes at 1000 B/s is a second, minus the 50 ms credit.
        assert!(
            last > Duration::from_millis(850) && last < Duration::from_millis(1000),
            "{last:?}"
        );
        assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn the_rate_can_change_while_in_use() {
        let limit = RateLimit::new(1000);
        limit.book(1000);
        limit.set(0);
        assert_eq!(limit.book(1_000_000), Duration::ZERO);
        assert_eq!(limit.get(), 0);
    }

    #[tokio::test]
    async fn the_slowest_of_several_limits_wins() {
        let fast = RateLimit::new(1_000_000);
        let slow = RateLimit::new(1000);
        let limits = Limits(vec![fast, slow]);
        let start = Instant::now();
        // 300 bytes at 1000 B/s: 300 ms, minus 50 ms of credit.
        limits.acquire(300).await;
        limits.acquire(1).await;
        assert!(
            start.elapsed() >= Duration::from_millis(150),
            "{:?}",
            start.elapsed()
        );
    }
}
