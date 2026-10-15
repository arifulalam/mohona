//! When and how long to wait before trying again.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use mohona_net::{Failure, HttpClient, NetError, Probe};

/// How patiently and gently a download treats servers: retries, waits, and
/// how fast it opens connections.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// The first wait. Each further failure in a row doubles it.
    pub base_delay: Duration,
    /// No wait is longer than this, even if the server asks for more.
    pub max_delay: Duration,
    /// How many failures in a row (without receiving any new bytes) one
    /// segment survives before the download gives up.
    pub max_attempts: u32,
    /// How many times in a row the server may refuse us (429, 503, refused
    /// connections) without any progress in between before the download
    /// gives up.
    pub max_throttles: u32,
    /// Find the right number of connections by measuring. The download
    /// starts with one connection and doubles them while the total speed
    /// really rises (see the `tuner` module). When `false`, all requested
    /// connections start at once.
    pub adaptive: bool,
    /// How often the speed is measured while tuning.
    pub sample_interval: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            max_attempts: 6,
            max_throttles: 10,
            adaptive: true,
            sample_interval: Duration::from_millis(250),
        }
    }
}

impl RetryPolicy {
    /// How long to wait before attempt number `attempt` (1 for the first
    /// retry). Grows exponentially up to `max_delay` and is randomized, so
    /// many connections do not all retry at the same moment. A `Retry-After`
    /// from the server replaces the exponential part.
    pub fn backoff(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        let ceiling = match retry_after {
            Some(asked) => asked,
            None => {
                let doublings = attempt.saturating_sub(1).min(20);
                self.base_delay.saturating_mul(1 << doublings)
            }
        }
        .min(self.max_delay);
        // Between half and all of the ceiling.
        let half = ceiling / 2;
        let spread = ceiling - half;
        half + spread.mul_f64(unit_random())
    }
}

/// A random number in `[0, 1)`, from the standard library's per-process
/// random hashing, so no extra dependency is needed.
fn unit_random() -> f64 {
    let bits = RandomState::new().build_hasher().finish() >> 11;
    bits as f64 / (1u64 << 53) as f64
}

/// Probe a URL, trying again a few times when the failure may be temporary.
pub(crate) async fn probe_with_retry(
    client: &HttpClient,
    url: &str,
    policy: &RetryPolicy,
) -> Result<Probe, NetError> {
    const ATTEMPTS: u32 = 3;
    let mut attempt = 1;
    loop {
        match client.probe(url).await {
            Ok(probe) => return Ok(probe),
            Err(error) if attempt < ATTEMPTS && error.failure() != Failure::Fatal => {
                tokio::time::sleep(policy.backoff(attempt, error.retry_after())).await;
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RetryPolicy;
    use std::time::Duration;

    fn policy() -> RetryPolicy {
        RetryPolicy {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
            ..RetryPolicy::default()
        }
    }

    #[test]
    fn waits_grow_and_stay_within_bounds() {
        let policy = policy();
        for _ in 0..200 {
            for (attempt, ceiling_ms) in [(1, 100), (2, 200), (3, 400), (4, 800), (5, 1600)] {
                let wait = policy.backoff(attempt, None);
                assert!(wait >= Duration::from_millis(ceiling_ms / 2), "{wait:?}");
                assert!(wait <= Duration::from_millis(ceiling_ms), "{wait:?}");
            }
            // Capped, even for absurd attempt numbers.
            assert!(policy.backoff(60, None) <= Duration::from_secs(2));
        }
    }

    #[test]
    fn retry_after_is_respected_up_to_the_cap() {
        let policy = policy();
        let wait = policy.backoff(1, Some(Duration::from_secs(1)));
        assert!(wait >= Duration::from_millis(500) && wait <= Duration::from_secs(1));
        assert!(policy.backoff(1, Some(Duration::from_secs(3600))) <= Duration::from_secs(2));
    }

    #[test]
    fn waits_are_randomized() {
        let policy = policy();
        let waits: std::collections::HashSet<_> = (0..50)
            .map(|_| policy.backoff(4, None).as_micros())
            .collect();
        assert!(waits.len() > 10);
    }
}
