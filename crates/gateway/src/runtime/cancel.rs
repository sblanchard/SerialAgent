//! Per-session cancellation tokens.
//!
//! Each running turn gets a `CancelToken`. Calling `cancel()` on it signals
//! the runtime to stop the current turn cleanly.

use std::collections::HashMap;
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

/// Tracks active cancellation tokens per session key.
pub struct CancelMap {
    tokens: Mutex<HashMap<String, CancelToken>>,
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

    /// Cancel a running turn for a session. Returns true if a token was found.
    pub fn cancel(&self, session_key: &str) -> bool {
        if let Some(token) = self.tokens.lock().get(session_key) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// Remove the token for a session (called when a turn completes).
    pub fn remove(&self, session_key: &str) {
        self.tokens.lock().remove(session_key);
    }

    /// Check if a session has an active (running) turn.
    pub fn is_running(&self, session_key: &str) -> bool {
        self.tokens.lock().contains_key(session_key)
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
}
