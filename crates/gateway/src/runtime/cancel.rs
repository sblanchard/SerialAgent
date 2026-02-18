//! Per-session cancellation tokens with group fan-out.
//!
//! Each running turn gets a `CancelToken`. Calling `cancel()` on it signals
//! the runtime to stop the current turn cleanly.
//!
//! **Groups** support cascading cancellation: when a parent turn is cancelled,
//! all children registered in its group are cancelled too.  This is used by
//! `agent.run` — child turns register in the parent's cancel group.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

/// A cancellation token that can be checked by the runtime loop.
#[derive(Clone)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Check if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks active cancellation tokens per session key, with group support
/// for cascading parent→child cancellation.
pub struct CancelMap {
    tokens: Mutex<HashMap<String, CancelToken>>,
    /// group_key (parent session) → set of child session keys.
    groups: Mutex<HashMap<String, HashSet<String>>>,
}

impl Default for CancelMap {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelMap {
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
            groups: Mutex::new(HashMap::new()),
        }
    }

    /// Create and register a new cancel token for a session.
    pub fn register(&self, session_key: &str) -> CancelToken {
        let token = CancelToken::new();
        self.tokens
            .lock()
            .insert(session_key.to_owned(), token.clone());
        token
    }

    /// Cancel a running turn for a session. Also cancels all children in
    /// the session's cancel group.  Returns true if a token was found.
    pub fn cancel(&self, session_key: &str) -> bool {
        let found = if let Some(token) = self.tokens.lock().get(session_key) {
            token.cancel();
            true
        } else {
            false
        };

        // Cascade to children.
        if let Some(children) = self.groups.lock().get(session_key) {
            let tokens = self.tokens.lock();
            for child_key in children {
                if let Some(child_token) = tokens.get(child_key) {
                    child_token.cancel();
                }
            }
        }

        found
    }

    /// Remove the token for a session (called when a turn completes).
    pub fn remove(&self, session_key: &str) {
        self.tokens.lock().remove(session_key);
        // Also remove any group this session owned.
        self.groups.lock().remove(session_key);
    }

    /// Check if a session has an active (running) turn.
    pub fn is_running(&self, session_key: &str) -> bool {
        self.tokens.lock().contains_key(session_key)
    }

    /// Register a child session key in a parent's cancel group.
    pub fn add_to_group(&self, parent_key: &str, child_key: &str) {
        self.groups
            .lock()
            .entry(parent_key.to_owned())
            .or_default()
            .insert(child_key.to_owned());
    }

    /// Remove a child from a parent's cancel group.
    pub fn remove_from_group(&self, parent_key: &str, child_key: &str) {
        let mut groups = self.groups.lock();
        if let Some(children) = groups.get_mut(parent_key) {
            children.remove(child_key);
            if children.is_empty() {
                groups.remove(parent_key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_token_lifecycle() {
        let token = CancelToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_map_register_and_cancel() {
        let map = CancelMap::new();
        let token = map.register("s1");
        assert!(!token.is_cancelled());
        assert!(map.is_running("s1"));

        assert!(map.cancel("s1"));
        assert!(token.is_cancelled());

        map.remove("s1");
        assert!(!map.is_running("s1"));
        assert!(!map.cancel("s1")); // no longer registered
    }

    #[test]
    fn cancel_cascades_to_children() {
        let map = CancelMap::new();
        let parent = map.register("parent");
        let child1 = map.register("child1");
        let child2 = map.register("child2");

        map.add_to_group("parent", "child1");
        map.add_to_group("parent", "child2");

        assert!(!child1.is_cancelled());
        assert!(!child2.is_cancelled());

        // Cancelling parent cascades to children.
        map.cancel("parent");
        assert!(parent.is_cancelled());
        assert!(child1.is_cancelled());
        assert!(child2.is_cancelled());
    }

    #[test]
    fn remove_from_group_cleanup() {
        let map = CancelMap::new();
        let _parent = map.register("p");
        let child = map.register("c");

        map.add_to_group("p", "c");
        map.remove_from_group("p", "c");

        // Cancelling parent should NOT cascade to removed child.
        map.cancel("p");
        assert!(!child.is_cancelled());
    }

    #[test]
    fn cancel_nonexistent_session_returns_false() {
        let map = CancelMap::new();
        assert!(!map.cancel("does_not_exist"));
    }

    #[test]
    fn is_running_false_for_unregistered() {
        let map = CancelMap::new();
        assert!(!map.is_running("ghost"));
    }

    #[test]
    fn remove_is_idempotent() {
        let map = CancelMap::new();
        map.register("s1");
        map.remove("s1");
        // Second remove should not panic.
        map.remove("s1");
        assert!(!map.is_running("s1"));
    }

    #[test]
    fn register_replaces_previous_token() {
        let map = CancelMap::new();
        let old_token = map.register("s1");
        let new_token = map.register("s1");

        // Old token is not cancelled, new token is fresh.
        assert!(!old_token.is_cancelled());
        assert!(!new_token.is_cancelled());

        // Cancelling via the map affects the new token.
        map.cancel("s1");
        assert!(new_token.is_cancelled());
        // Old token is orphaned — it does not get cancelled via the map.
        // (The old Arc still exists but is no longer in the map.)
    }

    #[test]
    fn cancel_token_clone_shares_state() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled());
    }

    #[test]
    fn cancel_token_default() {
        let token = CancelToken::default();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn remove_from_group_nonexistent_parent_is_safe() {
        let map = CancelMap::new();
        // Should not panic when parent group does not exist.
        map.remove_from_group("nonexistent", "child");
    }

    #[test]
    fn group_cleaned_up_on_parent_remove() {
        let map = CancelMap::new();
        let _parent = map.register("parent");
        let child = map.register("child");

        map.add_to_group("parent", "child");
        // Removing the parent should also clean up its group.
        map.remove("parent");

        // Child should still be independently accessible but
        // the group should be gone (no cascade).
        assert!(map.is_running("child"));
        assert!(!child.is_cancelled());
    }

    #[test]
    fn cancel_map_default_trait() {
        let map = CancelMap::default();
        assert!(!map.is_running("any"));
    }
}
