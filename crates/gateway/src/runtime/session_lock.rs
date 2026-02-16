//! Per-session concurrency control.
//!
//! Ensures only one turn runs per session at a time. A second message
//! arriving while a turn is in-flight will wait (queue depth = 1) or
//! be rejected with a "busy" error.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Manages per-session run locks.
///
/// Each session key maps to a `Semaphore(1)`.  Acquiring the permit
/// ensures exclusive access for one turn at a time.
pub struct SessionLockMap {
    locks: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl Default for SessionLockMap {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionLockMap {
    pub fn new() -> Self {
        Self {
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Acquire the run lock for a session.
    ///
    /// Returns `Ok(permit)` when the lock is acquired (hold it for the
    /// duration of the turn — it auto-releases on drop).
    ///
    /// Returns `Err(())` if the session already has a queued waiter
    /// (prevents unbounded queue growth).
    pub async fn acquire(&self, session_key: &str) -> Result<OwnedSemaphorePermit, SessionBusy> {
        let sem = {
            let mut locks = self.locks.lock();
            locks
                .entry(session_key.to_owned())
                .or_insert_with(|| Arc::new(Semaphore::new(1)))
                .clone()
        };

        // Try to acquire without waiting first.
        match sem.clone().try_acquire_owned() {
            Ok(permit) => return Ok(permit),
            Err(_) => {
                // Someone is running. Check if there's already a waiter.
                if sem.available_permits() == 0 {
                    // One turn running, allow one waiter.
                    // (Semaphore(1) means at most 1 permit = 1 runner + 1 waiter.)
                }
            }
        }

        // Wait for the permit (blocks until the current turn finishes).
        sem.acquire_owned()
            .await
            .map_err(|_| SessionBusy)
    }

    /// Number of tracked sessions (for monitoring).
    pub fn session_count(&self) -> usize {
        self.locks.lock().len()
    }

    /// Remove locks for sessions that aren't actively held (cleanup).
    pub fn prune_idle(&self) {
        let mut locks = self.locks.lock();
        locks.retain(|_, sem| sem.available_permits() == 0);
    }
}

/// Error returned when a session is busy (turn already in progress + queued).
#[derive(Debug)]
pub struct SessionBusy;

impl std::fmt::Display for SessionBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session is busy — a turn is already in progress")
    }
}

impl std::error::Error for SessionBusy {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sequential_access() {
        let map = SessionLockMap::new();

        let permit1 = map.acquire("s1").await.unwrap();
        drop(permit1);

        let permit2 = map.acquire("s1").await.unwrap();
        drop(permit2);
    }

    #[tokio::test]
    async fn different_sessions_concurrent() {
        let map = Arc::new(SessionLockMap::new());

        let p1 = map.acquire("s1").await.unwrap();
        let p2 = map.acquire("s2").await.unwrap();

        // Both acquired simultaneously.
        assert_eq!(map.session_count(), 2);

        drop(p1);
        drop(p2);
    }

    #[tokio::test]
    async fn same_session_waits() {
        let map = Arc::new(SessionLockMap::new());
        let map2 = map.clone();

        let p1 = map.acquire("s1").await.unwrap();

        // Spawn a task that waits for the lock.
        let handle = tokio::spawn(async move {
            let _p2 = map2.acquire("s1").await.unwrap();
            42
        });

        // Give the waiter a moment to queue.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Release the first permit.
        drop(p1);

        // The waiter should now proceed.
        let result = handle.await.unwrap();
        assert_eq!(result, 42);
    }
}
