use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use sa_domain::config::{ModelTier, RoutingProfile};
use serde::Serialize;
use std::collections::VecDeque;

/// A single routing decision record.
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub timestamp: DateTime<Utc>,
    pub prompt_snippet: String,
    pub profile: RoutingProfile,
    pub tier: ModelTier,
    pub model: String,
    pub latency_ms: u64,
    pub bypassed: bool,
}

/// Thread-safe ring buffer of recent routing decisions.
///
/// Uses `parking_lot::Mutex` for low-overhead synchronisation.
/// The buffer evicts the oldest entry when it reaches capacity,
/// keeping only the most recent decisions for observability.
pub struct DecisionLog {
    inner: Mutex<VecDeque<Decision>>,
    capacity: usize,
}

impl DecisionLog {
    /// Create a new decision log with the given maximum capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Record a new decision. If the buffer is at capacity the oldest
    /// entry is evicted first.
    pub fn record(&self, decision: Decision) {
        let mut buf = self.inner.lock();
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(decision);
    }

    /// Return the `limit` most recent decisions, newest first.
    ///
    /// If fewer than `limit` decisions exist, all are returned.
    pub fn recent(&self, limit: usize) -> Vec<Decision> {
        let buf = self.inner.lock();
        buf.iter().rev().take(limit).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper — build a `Decision` with a distinguishing index baked
    /// into `prompt_snippet` so assertions can identify ordering.
    fn make_decision(index: u64) -> Decision {
        Decision {
            timestamp: Utc::now(),
            prompt_snippet: format!("prompt-{index}"),
            profile: RoutingProfile::Auto,
            tier: ModelTier::Simple,
            model: "test-model".into(),
            latency_ms: index,
            bypassed: false,
        }
    }

    #[test]
    fn ring_buffer_stores_up_to_capacity() {
        let log = DecisionLog::new(3);
        for i in 0..5 {
            log.record(make_decision(i));
        }

        let recent = log.recent(10);
        assert_eq!(recent.len(), 3, "should keep at most 3 entries");

        // Newest first: 4, 3, 2
        assert_eq!(recent[0].latency_ms, 4);
        assert_eq!(recent[1].latency_ms, 3);
        assert_eq!(recent[2].latency_ms, 2);
    }

    #[test]
    fn ring_buffer_recent_respects_limit() {
        let log = DecisionLog::new(100);
        for i in 0..50 {
            log.record(make_decision(i));
        }

        let recent = log.recent(5);
        assert_eq!(recent.len(), 5);
        // Newest first: 49, 48, 47, 46, 45
        assert_eq!(recent[0].latency_ms, 49);
        assert_eq!(recent[4].latency_ms, 45);
    }

    #[test]
    fn ring_buffer_empty() {
        let log = DecisionLog::new(10);
        let recent = log.recent(5);
        assert!(recent.is_empty());
    }
}
