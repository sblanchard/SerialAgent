use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Task queue configuration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Per-session task queue concurrency settings.
///
/// Tasks bypass the existing `SessionLockMap` and use their own
/// semaphore-based concurrency control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// Maximum number of tasks that may execute concurrently within a
    /// single session.  Clamped to the range `1..=20`.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
        }
    }
}

impl TaskConfig {
    /// Clamp `max_concurrent` to the allowed range `1..=20`.
    pub fn clamped(&self) -> Self {
        Self {
            max_concurrent: self.max_concurrent.clamp(1, 20),
        }
    }
}

fn default_max_concurrent() -> usize {
    5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_concurrent_is_five() {
        let cfg = TaskConfig::default();
        assert_eq!(cfg.max_concurrent, 5);
    }

    #[test]
    fn clamp_below_min() {
        let cfg = TaskConfig { max_concurrent: 0 };
        assert_eq!(cfg.clamped().max_concurrent, 1);
    }

    #[test]
    fn clamp_above_max() {
        let cfg = TaskConfig { max_concurrent: 100 };
        assert_eq!(cfg.clamped().max_concurrent, 20);
    }

    #[test]
    fn clamp_within_range() {
        let cfg = TaskConfig { max_concurrent: 10 };
        assert_eq!(cfg.clamped().max_concurrent, 10);
    }

    #[test]
    fn clamp_at_boundaries() {
        assert_eq!(TaskConfig { max_concurrent: 1 }.clamped().max_concurrent, 1);
        assert_eq!(TaskConfig { max_concurrent: 20 }.clamped().max_concurrent, 20);
    }

    #[test]
    fn serde_roundtrip() {
        let cfg = TaskConfig { max_concurrent: 8 };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: TaskConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.max_concurrent, 8);
    }

    #[test]
    fn deserialize_missing_field_uses_default() {
        let json = "{}";
        let cfg: TaskConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.max_concurrent, 5);
    }
}
