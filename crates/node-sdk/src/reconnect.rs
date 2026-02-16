//! Reconnect policy with jittered exponential back-off.

use std::time::Duration;

/// Controls how the node client reconnects after a connection drop.
#[derive(Debug, Clone)]
pub struct ReconnectBackoff {
    /// Initial delay before the first reconnect attempt.
    pub initial_delay: Duration,
    /// Maximum delay between attempts (cap).
    pub max_delay: Duration,
    /// Multiplier applied after each failed attempt.
    pub backoff_factor: f64,
    /// Maximum number of consecutive failures before giving up.
    /// `0` means unlimited retries.
    pub max_attempts: u32,
}

impl Default for ReconnectBackoff {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_factor: 2.0,
            max_attempts: 0, // unlimited
        }
    }
}

impl ReconnectBackoff {
    /// Compute the delay for the given attempt number (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base_ms = self.initial_delay.as_millis() as f64;
        let delay_ms = base_ms * self.backoff_factor.powi(attempt as i32);
        let capped_ms = delay_ms.min(self.max_delay.as_millis() as f64);

        // Add ~25% jitter to prevent thundering herd.
        let jitter = capped_ms * 0.25 * pseudo_random_fraction(attempt);
        Duration::from_millis((capped_ms + jitter) as u64)
    }

    /// Whether the given attempt number exceeds the max.
    pub fn should_give_up(&self, attempt: u32) -> bool {
        self.max_attempts > 0 && attempt >= self.max_attempts
    }
}

/// Cheap deterministic "random" fraction [0, 1) based on attempt number.
/// Not cryptographically secure — just enough to spread reconnect storms.
fn pseudo_random_fraction(attempt: u32) -> f64 {
    let hash = attempt.wrapping_mul(2654435761); // Knuth multiplicative hash
    (hash as f64) / (u32::MAX as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_values() {
        let p = ReconnectBackoff::default();
        assert_eq!(p.initial_delay, Duration::from_secs(1));
        assert_eq!(p.max_delay, Duration::from_secs(60));
        assert_eq!(p.max_attempts, 0); // unlimited
    }

    #[test]
    fn delay_grows_with_backoff() {
        let p = ReconnectBackoff::default();
        let d0 = p.delay_for_attempt(0);
        let d1 = p.delay_for_attempt(1);
        let d2 = p.delay_for_attempt(2);
        assert!(d1 > d0);
        assert!(d2 > d1);
    }

    #[test]
    fn delay_capped_at_max() {
        let p = ReconnectBackoff {
            initial_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(30),
            backoff_factor: 10.0,
            max_attempts: 0,
        };
        let d = p.delay_for_attempt(10);
        // Should not exceed max_delay + 25% jitter.
        assert!(d <= Duration::from_millis(37_500));
    }

    #[test]
    fn should_give_up_when_limited() {
        let p = ReconnectBackoff {
            max_attempts: 5,
            ..Default::default()
        };
        assert!(!p.should_give_up(4));
        assert!(p.should_give_up(5));
        assert!(p.should_give_up(6));
    }

    #[test]
    fn unlimited_never_gives_up() {
        let p = ReconnectBackoff::default();
        assert!(!p.should_give_up(1_000_000));
    }
}
