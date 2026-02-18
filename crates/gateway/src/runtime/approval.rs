//! Exec approval workflow — gates dangerous commands behind human approval.
//!
//! When a command matches one of the configured `approval_patterns`, execution
//! is paused until a human approves or denies the request via the REST API.
//! A timeout ensures the system never blocks indefinitely.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::Serialize;
use tokio::sync::oneshot;
use uuid::Uuid;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The decision made by a human reviewer.
#[derive(Debug)]
pub enum ApprovalDecision {
    Approved,
    Denied { reason: Option<String> },
}

/// A pending approval waiting for human review.
pub struct PendingApproval {
    pub id: Uuid,
    pub command: String,
    pub session_key: String,
    pub created_at: DateTime<Utc>,
    pub respond: oneshot::Sender<ApprovalDecision>,
}

/// Serializable snapshot of a pending approval (for API responses / SSE events).
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalInfo {
    pub id: Uuid,
    pub command: String,
    pub session_key: String,
    pub created_at: DateTime<Utc>,
}

impl From<&PendingApproval> for ApprovalInfo {
    fn from(p: &PendingApproval) -> Self {
        Self {
            id: p.id,
            command: p.command.clone(),
            session_key: p.session_key.clone(),
            created_at: p.created_at,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Store
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Thread-safe store for pending exec approvals.
///
/// Each approval is associated with a `oneshot::Sender` that unblocks the
/// waiting `dispatch_exec` call when resolved.
pub struct ApprovalStore {
    pending: RwLock<HashMap<Uuid, PendingApproval>>,
    timeout: Duration,
}

impl ApprovalStore {
    /// Create a new store with the given approval timeout.
    pub fn new(timeout: Duration) -> Self {
        Self {
            pending: RwLock::new(HashMap::new()),
            timeout,
        }
    }

    /// The configured approval timeout duration.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Insert a pending approval. Returns the serializable info snapshot.
    pub fn insert(&self, approval: PendingApproval) -> ApprovalInfo {
        let info = ApprovalInfo::from(&approval);
        self.pending.write().insert(approval.id, approval);
        info
    }

    /// Resolve a pending approval as approved. Returns `true` if found.
    pub fn approve(&self, id: &Uuid) -> bool {
        if let Some(pending) = self.pending.write().remove(id) {
            let _ = pending.respond.send(ApprovalDecision::Approved);
            return true;
        }
        false
    }

    /// Resolve a pending approval as denied. Returns `true` if found.
    pub fn deny(&self, id: &Uuid, reason: Option<String>) -> bool {
        if let Some(pending) = self.pending.write().remove(id) {
            let _ = pending.respond.send(ApprovalDecision::Denied { reason });
            return true;
        }
        false
    }

    /// Remove a timed-out approval (called when the receiver times out).
    pub fn remove_expired(&self, id: &Uuid) {
        self.pending.write().remove(id);
    }

    /// List all currently pending approvals (for dashboard introspection).
    pub fn list_pending(&self) -> Vec<ApprovalInfo> {
        self.pending
            .read()
            .values()
            .map(ApprovalInfo::from)
            .collect()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> ApprovalStore {
        ApprovalStore::new(Duration::from_secs(300))
    }

    fn make_pending() -> (PendingApproval, oneshot::Receiver<ApprovalDecision>) {
        let (tx, rx) = oneshot::channel();
        let pending = PendingApproval {
            id: Uuid::new_v4(),
            command: "rm -rf /tmp/test".into(),
            session_key: "sk_test".into(),
            created_at: Utc::now(),
            respond: tx,
        };
        (pending, rx)
    }

    #[test]
    fn insert_and_list() {
        let store = make_store();
        let (pending, _rx) = make_pending();
        let id = pending.id;
        store.insert(pending);

        let list = store.list_pending();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
    }

    #[tokio::test]
    async fn approve_resolves_channel() {
        let store = make_store();
        let (pending, rx) = make_pending();
        let id = pending.id;
        store.insert(pending);

        assert!(store.approve(&id));
        let decision = rx.await.unwrap();
        assert!(matches!(decision, ApprovalDecision::Approved));
        assert!(store.list_pending().is_empty());
    }

    #[tokio::test]
    async fn deny_resolves_channel() {
        let store = make_store();
        let (pending, rx) = make_pending();
        let id = pending.id;
        store.insert(pending);

        assert!(store.deny(&id, Some("too dangerous".into())));
        let decision = rx.await.unwrap();
        match decision {
            ApprovalDecision::Denied { reason } => {
                assert_eq!(reason.as_deref(), Some("too dangerous"));
            }
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn approve_nonexistent_returns_false() {
        let store = make_store();
        assert!(!store.approve(&Uuid::new_v4()));
    }

    #[test]
    fn deny_nonexistent_returns_false() {
        let store = make_store();
        assert!(!store.deny(&Uuid::new_v4(), None));
    }

    #[test]
    fn remove_expired() {
        let store = make_store();
        let (pending, _rx) = make_pending();
        let id = pending.id;
        store.insert(pending);

        store.remove_expired(&id);
        assert!(store.list_pending().is_empty());
    }

    #[test]
    fn timeout_returns_configured_duration() {
        let store = ApprovalStore::new(Duration::from_secs(60));
        assert_eq!(store.timeout(), Duration::from_secs(60));
    }
}
