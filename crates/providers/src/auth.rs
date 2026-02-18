//! Auth key rotation with round-robin selection and failure cooldown.
//!
//! [`AuthRotator`] holds one or more resolved API keys and hands them out
//! via [`AuthRotator::next_key`] in round-robin order. When a key causes a
//! failure, callers invoke [`AuthRotator::mark_failed`] to put that key into
//! a cooldown window. Keys in cooldown are skipped during rotation; if every
//! key is cooling down, the least-recently-failed key is returned instead.
//!
//! The rotator is thread-safe (`Send + Sync`) and designed to be shared
//! across async tasks behind an `Arc`.

use sa_domain::config::AuthConfig;
use sa_domain::error::{Error, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default cooldown period after a key failure (seconds).
const DEFAULT_COOLDOWN_SECS: u64 = 60;

/// A single resolved API key with its cooldown state.
struct KeySlot {
    /// The resolved API key value.
    key: String,
    /// When the key last failed. `None` means it is healthy.
    failed_at: Option<Instant>,
}

/// Thread-safe round-robin key rotator with failure cooldown.
///
/// # Construction
///
/// Use [`AuthRotator::from_auth_config`] to resolve env vars and build
/// the rotator. If `AuthConfig.keys` is non-empty each entry is treated
/// as an environment variable name and resolved eagerly. Otherwise the
/// single `env`/`key` field is used (backward compatible).
pub struct AuthRotator {
    /// Resolved key slots. At least one is always present after construction.
    slots: Mutex<Vec<KeySlot>>,
    /// Atomic counter for round-robin indexing.
    index: AtomicUsize,
    /// How long a failed key is kept in cooldown.
    cooldown: Duration,
}

impl AuthRotator {
    /// Build a rotator from resolved keys.
    ///
    /// # Errors
    ///
    /// Returns an error if `keys` is empty.
    fn new(keys: Vec<String>, cooldown: Duration) -> Result<Self> {
        if keys.is_empty() {
            return Err(Error::Auth(
                "AuthRotator requires at least one resolved API key".into(),
            ));
        }
        let slots = keys
            .into_iter()
            .map(|key| KeySlot {
                key,
                failed_at: None,
            })
            .collect();
        Ok(Self {
            slots: Mutex::new(slots),
            index: AtomicUsize::new(0),
            cooldown,
        })
    }

    /// Build a rotator from an [`AuthConfig`].
    ///
    /// Resolution order:
    /// 1. If `auth.keys` is non-empty, resolve each env var name and use those.
    /// 2. Else fall back to single `auth.key` (direct) or `auth.env` (env var).
    pub fn from_auth_config(auth: &AuthConfig) -> Result<Self> {
        let resolved = if !auth.keys.is_empty() {
            let mut resolved_keys = Vec::with_capacity(auth.keys.len());
            for env_name in &auth.keys {
                match std::env::var(env_name) {
                    Ok(val) if !val.is_empty() => resolved_keys.push(val),
                    _ => {
                        return Err(Error::Auth(format!(
                            "environment variable '{}' not set or empty \
                             (from auth.keys list)",
                            env_name
                        )));
                    }
                }
            }
            resolved_keys
        } else {
            // Fall back to single key resolution.
            let key = crate::util::resolve_api_key(auth)?;
            vec![key]
        };

        Self::new(resolved, Duration::from_secs(DEFAULT_COOLDOWN_SECS))
    }

    /// Return the next healthy key using round-robin.
    ///
    /// Keys that are within their cooldown window are skipped. If all keys
    /// are in cooldown, the one whose cooldown expires soonest (i.e. was
    /// marked failed longest ago) is returned.
    pub fn next_key(&self) -> KeyEntry {
        let slots = self.slots.lock().expect("AuthRotator lock poisoned");
        let len = slots.len();
        let now = Instant::now();

        // Fast path: single key, no rotation needed.
        if len == 1 {
            return KeyEntry {
                index: 0,
                key: slots[0].key.clone(),
            };
        }

        let start = self.index.fetch_add(1, Ordering::Relaxed) % len;

        // First pass: find the next healthy key.
        for offset in 0..len {
            let idx = (start + offset) % len;
            let slot = &slots[idx];
            if let Some(failed_at) = slot.failed_at {
                if now.duration_since(failed_at) < self.cooldown {
                    continue; // still in cooldown
                }
            }
            return KeyEntry {
                index: idx,
                key: slot.key.clone(),
            };
        }

        // All keys are in cooldown. Pick the one that failed longest ago
        // (its cooldown expires soonest).
        let best = slots
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| s.failed_at.unwrap_or(now))
            .map(|(i, s)| KeyEntry {
                index: i,
                key: s.key.clone(),
            })
            .expect("slots is non-empty");
        best
    }

    /// Mark a key at the given index as failed, starting its cooldown timer.
    pub fn mark_failed(&self, index: usize) {
        let mut slots = self.slots.lock().expect("AuthRotator lock poisoned");
        if let Some(slot) = slots.get_mut(index) {
            slot.failed_at = Some(Instant::now());
            tracing::warn!(
                key_index = index,
                cooldown_secs = self.cooldown.as_secs(),
                "API key marked as failed, entering cooldown"
            );
        }
    }

    /// Number of keys in the rotator.
    pub fn len(&self) -> usize {
        self.slots.lock().expect("AuthRotator lock poisoned").len()
    }

    /// Whether the rotator has exactly zero keys (should never be true after
    /// successful construction).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Manual Debug impl to avoid leaking key values.
impl std::fmt::Debug for AuthRotator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.slots.lock().map(|s| s.len()).unwrap_or(0);
        f.debug_struct("AuthRotator")
            .field("key_count", &len)
            .field("cooldown", &self.cooldown)
            .finish()
    }
}

/// A key entry returned by [`AuthRotator::next_key`].
///
/// Callers should hold onto the `index` so they can call
/// [`AuthRotator::mark_failed`] if the request fails.
#[derive(Debug, Clone)]
pub struct KeyEntry {
    /// Index into the rotator's key list.
    pub index: usize,
    /// The resolved API key value.
    pub key: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_key_always_returns_same() {
        let rotator =
            AuthRotator::new(vec!["key-a".into()], Duration::from_secs(60)).unwrap();
        let e1 = rotator.next_key();
        let e2 = rotator.next_key();
        assert_eq!(e1.key, "key-a");
        assert_eq!(e2.key, "key-a");
        assert_eq!(e1.index, 0);
    }

    #[test]
    fn round_robin_cycles_through_keys() {
        let rotator = AuthRotator::new(
            vec!["a".into(), "b".into(), "c".into()],
            Duration::from_secs(60),
        )
        .unwrap();

        let mut seen = Vec::new();
        for _ in 0..6 {
            seen.push(rotator.next_key().key);
        }
        // Should cycle through a, b, c, a, b, c
        assert_eq!(seen, vec!["a", "b", "c", "a", "b", "c"]);
    }

    #[test]
    fn mark_failed_skips_key() {
        let rotator = AuthRotator::new(
            vec!["a".into(), "b".into(), "c".into()],
            Duration::from_secs(60),
        )
        .unwrap();

        // First call: counter=0, start=0, returns "a".
        let e = rotator.next_key();
        assert_eq!(e.key, "a");

        // Mark "b" (index 1) as failed.
        rotator.mark_failed(1);

        // Second call: counter=1, start=1 ("b" in cooldown), skip to "c".
        let e = rotator.next_key();
        assert_eq!(e.key, "c");

        // Third call: counter=2, start=2, "c" is healthy.
        let e = rotator.next_key();
        assert_eq!(e.key, "c");

        // Fourth call: counter=3, start=0, "a" is healthy.
        let e = rotator.next_key();
        assert_eq!(e.key, "a");

        // Fifth call: counter=4, start=1, "b" still in cooldown, skip to "c".
        let e = rotator.next_key();
        assert_eq!(e.key, "c");
    }

    #[test]
    fn all_failed_returns_least_recently_failed() {
        let rotator = AuthRotator::new(
            vec!["a".into(), "b".into()],
            Duration::from_secs(60),
        )
        .unwrap();

        // Mark "a" as failed first.
        rotator.mark_failed(0);
        // Small delay to ensure different Instant values.
        std::thread::sleep(Duration::from_millis(10));
        // Mark "b" as failed second.
        rotator.mark_failed(1);

        // Both in cooldown: should return "a" (failed longest ago).
        let e = rotator.next_key();
        assert_eq!(e.key, "a");
    }

    #[test]
    fn expired_cooldown_key_is_available() {
        let rotator = AuthRotator::new(
            vec!["a".into(), "b".into()],
            Duration::from_millis(50), // very short cooldown
        )
        .unwrap();

        rotator.mark_failed(0);
        // Wait for cooldown to expire.
        std::thread::sleep(Duration::from_millis(100));

        // "a" should now be available again.
        let e = rotator.next_key();
        assert_eq!(e.key, "a");
    }

    #[test]
    fn empty_keys_returns_error() {
        let result = AuthRotator::new(vec![], Duration::from_secs(60));
        assert!(result.is_err());
    }

    #[test]
    fn from_auth_config_single_key() {
        let auth = AuthConfig {
            key: Some("direct-key".into()),
            ..AuthConfig::default()
        };
        let rotator = AuthRotator::from_auth_config(&auth).unwrap();
        let e = rotator.next_key();
        assert_eq!(e.key, "direct-key");
        assert_eq!(rotator.len(), 1);
    }

    #[test]
    fn from_auth_config_keys_env_missing() {
        let auth = AuthConfig {
            keys: vec!["NONEXISTENT_VAR_12345".into()],
            ..AuthConfig::default()
        };
        let result = AuthRotator::from_auth_config(&auth);
        assert!(result.is_err());
    }

    #[test]
    fn debug_does_not_leak_keys() {
        let rotator =
            AuthRotator::new(vec!["secret-key".into()], Duration::from_secs(60)).unwrap();
        let debug_str = format!("{:?}", rotator);
        assert!(!debug_str.contains("secret-key"));
        assert!(debug_str.contains("key_count: 1"));
    }
}
