//! Concurrent task queue — per-session semaphore-controlled execution.
//!
//! Tasks bypass the existing `SessionLockMap` (which enforces 1-turn-at-a-time
//! for direct `/v1/chat` calls) and instead use their own per-session semaphores
//! to allow multiple concurrent turns within a session.
//!
//! Tasks are ephemeral — runs are the durable record.  The `TaskStore` is
//! in-memory only (no JSONL persistence).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Semaphore};
use uuid::Uuid;

use super::turn::{TurnEvent, TurnInput};
use crate::state::AppState;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Task status
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Task record
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: Uuid,
    pub session_key: String,
    pub session_id: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Task {
    pub fn new(session_key: String, session_id: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_key,
            session_id,
            status: TaskStatus::Queued,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            run_id: None,
            result: None,
            error: None,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Task events (for SSE broadcast)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum TaskEvent {
    #[serde(rename = "task.status")]
    StatusChanged { task_id: Uuid, status: TaskStatus },
    #[serde(rename = "task.turn_event")]
    TurnEvent {
        task_id: Uuid,
        #[serde(flatten)]
        event: super::turn::TurnEvent,
    },
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Task store (in-memory, ephemeral)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct TaskStore {
    tasks: RwLock<HashMap<Uuid, Task>>,
    /// Per-task broadcast channels for SSE event streaming.
    event_channels: RwLock<HashMap<Uuid, broadcast::Sender<TaskEvent>>>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            event_channels: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a new task. Returns the task ID.
    pub fn insert(&self, task: Task) -> Uuid {
        let task_id = task.id;
        self.tasks.write().insert(task_id, task);
        task_id
    }

    /// Get a task by ID.
    pub fn get(&self, task_id: &Uuid) -> Option<Task> {
        self.tasks.read().get(task_id).cloned()
    }

    /// List tasks with optional filters and pagination.
    ///
    /// Returns (page, total_matching).  Results are ordered newest-first.
    pub fn list(
        &self,
        session_key: Option<&str>,
        status: Option<TaskStatus>,
        limit: usize,
        offset: usize,
    ) -> (Vec<Task>, usize) {
        let tasks = self.tasks.read();

        let filter = |t: &&Task| -> bool {
            if let Some(sk) = session_key {
                if t.session_key != sk {
                    return false;
                }
            }
            if let Some(s) = status {
                if t.status != s {
                    return false;
                }
            }
            true
        };

        // Collect matching tasks sorted by created_at descending (newest first).
        let mut matching: Vec<&Task> = tasks.values().filter(filter).collect();
        matching.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let total = matching.len();
        let page: Vec<Task> = matching
            .into_iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();

        (page, total)
    }

    /// Update a task in-place by ID. Returns true if found.
    pub fn update<F>(&self, task_id: &Uuid, f: F) -> bool
    where
        F: FnOnce(&mut Task),
    {
        let mut tasks = self.tasks.write();
        if let Some(task) = tasks.get_mut(task_id) {
            f(task);
            return true;
        }
        false
    }

    /// Cancel a task. Returns true if the task was found and was in a
    /// non-terminal state.
    pub fn cancel(&self, task_id: &Uuid) -> bool {
        let mut tasks = self.tasks.write();
        if let Some(task) = tasks.get_mut(task_id) {
            if task.status.is_terminal() {
                return false;
            }
            task.status = TaskStatus::Cancelled;
            task.completed_at = Some(Utc::now());
            return true;
        }
        false
    }

    /// Get or create a broadcast channel for a task (for SSE).
    pub fn subscribe(&self, task_id: &Uuid) -> broadcast::Receiver<TaskEvent> {
        let mut channels = self.event_channels.write();
        let tx = channels
            .entry(*task_id)
            .or_insert_with(|| broadcast::channel(128).0);
        tx.subscribe()
    }

    /// Emit an event for a task (broadcast to all subscribers).
    pub fn emit(&self, task_id: &Uuid, event: TaskEvent) {
        let channels = self.event_channels.read();
        if let Some(tx) = channels.get(task_id) {
            let _ = tx.send(event);
        }
    }

    /// Clean up the broadcast channel for a completed task.
    pub fn cleanup_channel(&self, task_id: &Uuid) {
        let mut channels = self.event_channels.write();
        channels.remove(task_id);
    }

    /// Remove terminal tasks older than the given duration.
    /// Called periodically to prevent unbounded memory growth.
    pub fn evict_terminal(&self, older_than: chrono::Duration) {
        let cutoff = Utc::now() - older_than;
        let mut tasks = self.tasks.write();
        tasks.retain(|_, t| {
            !t.status.is_terminal()
                || t.completed_at.map_or(true, |ts| ts > cutoff)
        });
    }
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Task runner (per-session semaphore concurrency)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub struct TaskRunner {
    /// Per-session semaphores controlling concurrency.
    semaphores: RwLock<HashMap<String, Arc<Semaphore>>>,
    /// Maximum concurrent tasks per session (clamped to 1..=20).
    max_concurrent: usize,
}

impl TaskRunner {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphores: RwLock::new(HashMap::new()),
            max_concurrent: max_concurrent.clamp(1, 20),
        }
    }

    /// Get the max concurrency setting.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Get or create the semaphore for a session.
    fn session_semaphore(&self, session_key: &str) -> Arc<Semaphore> {
        // Fast path: read lock.
        {
            let semaphores = self.semaphores.read();
            if let Some(sem) = semaphores.get(session_key) {
                return sem.clone();
            }
        }
        // Slow path: write lock to insert.
        let mut semaphores = self.semaphores.write();
        semaphores
            .entry(session_key.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(self.max_concurrent)))
            .clone()
    }

    /// Enqueue a task: spawns a tokio task that waits for a semaphore
    /// permit, then executes the turn.
    pub fn enqueue(
        &self,
        state: AppState,
        task_store: Arc<TaskStore>,
        task_id: Uuid,
        input: TurnInput,
    ) {
        let semaphore = self.session_semaphore(&input.session_key);
        let cancel_key = format!("task:{task_id}");

        let span = tracing::info_span!(
            "task_runner",
            %task_id,
            session_key = %input.session_key,
        );

        tokio::spawn(tracing::Instrument::instrument(
            async move {
                // 1. Acquire semaphore permit.
                let _permit = match semaphore.acquire().await {
                    Ok(permit) => permit,
                    Err(_) => {
                        // Semaphore closed (should not happen).
                        task_store.update(&task_id, |t| {
                            t.status = TaskStatus::Failed;
                            t.error = Some("semaphore closed".into());
                            t.completed_at = Some(Utc::now());
                        });
                        task_store.emit(
                            &task_id,
                            TaskEvent::StatusChanged {
                                task_id,
                                status: TaskStatus::Failed,
                            },
                        );
                        task_store.cleanup_channel(&task_id);
                        return;
                    }
                };

                // Check if the task was cancelled while queued.
                if let Some(task) = task_store.get(&task_id) {
                    if task.status == TaskStatus::Cancelled {
                        task_store.cleanup_channel(&task_id);
                        return;
                    }
                } else {
                    return;
                }

                // 2. Update task status to Running.
                task_store.update(&task_id, |t| {
                    t.status = TaskStatus::Running;
                    t.started_at = Some(Utc::now());
                });
                task_store.emit(
                    &task_id,
                    TaskEvent::StatusChanged {
                        task_id,
                        status: TaskStatus::Running,
                    },
                );

                // Register a cancel token for the task.
                let _cancel_token = state.cancel_map.register(&cancel_key);

                // 3. Call run_turn reusing the existing turn machinery.
                let (run_id, mut rx) = super::turn::run_turn(state.clone(), input);

                // Link the run to the task.
                task_store.update(&task_id, |t| {
                    t.run_id = Some(run_id);
                });

                // 4. Forward TurnEvents through the task's broadcast channel.
                let mut final_content = String::new();
                let mut had_error = false;
                let mut error_msg = None;

                while let Some(event) = rx.recv().await {
                    match &event {
                        TurnEvent::Final { content } => {
                            final_content = content.clone();
                        }
                        TurnEvent::Error { message } => {
                            had_error = true;
                            error_msg = Some(message.clone());
                        }
                        TurnEvent::Stopped { content } => {
                            final_content = content.clone();
                        }
                        _ => {}
                    }

                    task_store.emit(
                        &task_id,
                        TaskEvent::TurnEvent {
                            task_id,
                            event,
                        },
                    );
                }

                // Cleanup cancel token.
                state.cancel_map.remove(&cancel_key);

                // 5. Update task status on completion/failure.
                // Guard: if the task was already cancelled externally
                // (via the cancel API), do not overwrite its terminal status.
                let final_status = if had_error {
                    TaskStatus::Failed
                } else {
                    TaskStatus::Completed
                };

                let did_update = task_store.update(&task_id, |t| {
                    if t.status.is_terminal() {
                        return; // Already cancelled — do not overwrite.
                    }
                    t.status = final_status;
                    t.completed_at = Some(Utc::now());
                    if !final_content.is_empty() {
                        t.result = Some(final_content.clone());
                    }
                    if let Some(ref err) = error_msg {
                        t.error = Some(err.clone());
                    }
                });

                // Emit terminal status event for SSE subscribers.
                // If the task was already cancelled externally, emit the
                // Cancelled status; otherwise emit Completed/Failed.
                if did_update {
                    if let Some(task) = task_store.get(&task_id) {
                        task_store.emit(
                            &task_id,
                            TaskEvent::StatusChanged {
                                task_id,
                                status: task.status,
                            },
                        );
                    }
                }

                // 6. Clean up event channel.
                task_store.cleanup_channel(&task_id);

                // Permit is dropped here, releasing the semaphore slot.
            },
            span,
        ));
    }

    /// Cancel a running task by signalling its cancel token.
    pub fn cancel_task(&self, state: &AppState, task_id: &Uuid) {
        let cancel_key = format!("task:{task_id}");
        state.cancel_map.cancel(&cancel_key);
    }

    /// Remove semaphores for sessions with no active tasks.
    /// Called periodically to prevent unbounded growth.
    pub fn prune_idle(&self) {
        let mut semaphores = self.semaphores.write();
        semaphores.retain(|_, sem| {
            // Keep if someone still holds a permit (available < max).
            // Since we can't easily query max, retain if strong_count > 1
            // (meaning a spawned task holds a reference).
            Arc::strong_count(sem) > 1
        });
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    // ── TaskStatus ──────────────────────────────────────────────────

    #[test]
    fn task_status_is_terminal() {
        assert!(!TaskStatus::Queued.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
    }

    #[test]
    fn task_status_serde_roundtrip() {
        let statuses = [
            TaskStatus::Queued,
            TaskStatus::Running,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).unwrap();
            let deserialized: TaskStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, deserialized);
        }
    }

    #[test]
    fn task_status_snake_case_serialization() {
        assert_eq!(serde_json::to_string(&TaskStatus::Queued).unwrap(), "\"queued\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Completed).unwrap(), "\"completed\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Failed).unwrap(), "\"failed\"");
        assert_eq!(serde_json::to_string(&TaskStatus::Cancelled).unwrap(), "\"cancelled\"");
    }

    // ── Task ────────────────────────────────────────────────────────

    #[test]
    fn task_new_defaults() {
        let task = Task::new("sk1".into(), "sid1".into());
        assert_eq!(task.session_key, "sk1");
        assert_eq!(task.session_id, "sid1");
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.started_at.is_none());
        assert!(task.completed_at.is_none());
        assert!(task.run_id.is_none());
        assert!(task.result.is_none());
        assert!(task.error.is_none());
    }

    // ── TaskStore ───────────────────────────────────────────────────

    #[test]
    fn store_insert_and_get() {
        let store = TaskStore::new();
        let task = Task::new("sk".into(), "sid".into());
        let task_id = task.id;
        store.insert(task);

        let fetched = store.get(&task_id).unwrap();
        assert_eq!(fetched.session_key, "sk");
        assert_eq!(fetched.status, TaskStatus::Queued);
    }

    #[test]
    fn store_get_nonexistent() {
        let store = TaskStore::new();
        assert!(store.get(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn store_update() {
        let store = TaskStore::new();
        let task = Task::new("sk".into(), "sid".into());
        let task_id = task.id;
        store.insert(task);

        let found = store.update(&task_id, |t| {
            t.status = TaskStatus::Running;
            t.started_at = Some(Utc::now());
        });
        assert!(found);

        let fetched = store.get(&task_id).unwrap();
        assert_eq!(fetched.status, TaskStatus::Running);
        assert!(fetched.started_at.is_some());
    }

    #[test]
    fn store_update_nonexistent() {
        let store = TaskStore::new();
        let found = store.update(&Uuid::new_v4(), |t| {
            t.status = TaskStatus::Running;
        });
        assert!(!found);
    }

    #[test]
    fn store_cancel_queued_task() {
        let store = TaskStore::new();
        let task = Task::new("sk".into(), "sid".into());
        let task_id = task.id;
        store.insert(task);

        assert!(store.cancel(&task_id));
        let fetched = store.get(&task_id).unwrap();
        assert_eq!(fetched.status, TaskStatus::Cancelled);
        assert!(fetched.completed_at.is_some());
    }

    #[test]
    fn store_cancel_running_task() {
        let store = TaskStore::new();
        let task = Task::new("sk".into(), "sid".into());
        let task_id = task.id;
        store.insert(task);
        store.update(&task_id, |t| {
            t.status = TaskStatus::Running;
        });

        assert!(store.cancel(&task_id));
        let fetched = store.get(&task_id).unwrap();
        assert_eq!(fetched.status, TaskStatus::Cancelled);
    }

    #[test]
    fn store_cancel_terminal_task_returns_false() {
        let store = TaskStore::new();
        let task = Task::new("sk".into(), "sid".into());
        let task_id = task.id;
        store.insert(task);
        store.update(&task_id, |t| {
            t.status = TaskStatus::Completed;
        });

        assert!(!store.cancel(&task_id));
        let fetched = store.get(&task_id).unwrap();
        assert_eq!(fetched.status, TaskStatus::Completed);
    }

    #[test]
    fn store_cancel_nonexistent_returns_false() {
        let store = TaskStore::new();
        assert!(!store.cancel(&Uuid::new_v4()));
    }

    #[test]
    fn store_list_all() {
        let store = TaskStore::new();
        for _ in 0..5 {
            store.insert(Task::new("sk".into(), "sid".into()));
        }

        let (tasks, total) = store.list(None, None, 50, 0);
        assert_eq!(total, 5);
        assert_eq!(tasks.len(), 5);
    }

    #[test]
    fn store_list_filter_by_session_key() {
        let store = TaskStore::new();
        store.insert(Task::new("alpha".into(), "sid".into()));
        store.insert(Task::new("beta".into(), "sid".into()));
        store.insert(Task::new("alpha".into(), "sid".into()));

        let (tasks, total) = store.list(Some("alpha"), None, 50, 0);
        assert_eq!(total, 2);
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().all(|t| t.session_key == "alpha"));
    }

    #[test]
    fn store_list_filter_by_status() {
        let store = TaskStore::new();
        let t1 = Task::new("sk".into(), "sid".into());
        let t1_id = t1.id;
        store.insert(t1);
        store.insert(Task::new("sk".into(), "sid".into()));

        store.update(&t1_id, |t| {
            t.status = TaskStatus::Completed;
        });

        let (completed, total) = store.list(None, Some(TaskStatus::Completed), 50, 0);
        assert_eq!(total, 1);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].status, TaskStatus::Completed);
    }

    #[test]
    fn store_list_pagination() {
        let store = TaskStore::new();
        for _ in 0..5 {
            store.insert(Task::new("sk".into(), "sid".into()));
        }

        let (page1, total) = store.list(None, None, 2, 0);
        assert_eq!(total, 5);
        assert_eq!(page1.len(), 2);

        let (page2, _) = store.list(None, None, 2, 2);
        assert_eq!(page2.len(), 2);

        let (page3, _) = store.list(None, None, 2, 4);
        assert_eq!(page3.len(), 1);

        // No overlap between pages.
        let all_ids: std::collections::HashSet<_> = page1
            .iter()
            .chain(page2.iter())
            .chain(page3.iter())
            .map(|t| t.id)
            .collect();
        assert_eq!(all_ids.len(), 5);
    }

    #[test]
    fn store_list_empty() {
        let store = TaskStore::new();
        let (tasks, total) = store.list(None, None, 50, 0);
        assert_eq!(total, 0);
        assert!(tasks.is_empty());
    }

    #[test]
    fn store_list_combined_filters() {
        let store = TaskStore::new();

        let t1 = Task::new("alpha".into(), "sid".into());
        let t1_id = t1.id;
        store.insert(t1);

        let t2 = Task::new("alpha".into(), "sid".into());
        store.insert(t2);

        store.insert(Task::new("beta".into(), "sid".into()));

        store.update(&t1_id, |t| {
            t.status = TaskStatus::Completed;
        });

        // Filter: session_key=alpha AND status=completed
        let (tasks, total) = store.list(Some("alpha"), Some(TaskStatus::Completed), 50, 0);
        assert_eq!(total, 1);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, t1_id);
    }

    // ── TaskRunner ──────────────────────────────────────────────────

    #[test]
    fn runner_clamps_max_concurrent() {
        assert_eq!(TaskRunner::new(0).max_concurrent(), 1);
        assert_eq!(TaskRunner::new(5).max_concurrent(), 5);
        assert_eq!(TaskRunner::new(20).max_concurrent(), 20);
        assert_eq!(TaskRunner::new(100).max_concurrent(), 20);
    }

    #[test]
    fn runner_creates_semaphore_per_session() {
        let runner = TaskRunner::new(5);
        let s1 = runner.session_semaphore("session-a");
        let s2 = runner.session_semaphore("session-b");

        // Different sessions get different semaphores.
        assert!(!Arc::ptr_eq(&s1, &s2));

        // Same session gets the same semaphore.
        let s1_again = runner.session_semaphore("session-a");
        assert!(Arc::ptr_eq(&s1, &s1_again));
    }

    #[test]
    fn runner_semaphore_has_correct_permits() {
        let runner = TaskRunner::new(3);
        let sem = runner.session_semaphore("test");
        assert_eq!(sem.available_permits(), 3);
    }

    // ── TaskEvent ───────────────────────────────────────────────────

    #[test]
    fn task_event_status_changed_serialization() {
        let event = TaskEvent::StatusChanged {
            task_id: Uuid::nil(),
            status: TaskStatus::Running,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"task.status\""));
        assert!(json.contains("\"status\":\"running\""));
    }
}
