//! Run tracking — persistent execution records for every agent turn.
//!
//! Each call to [`run_turn`] produces a `Run` with a unique UUID. The run
//! contains a list of `RunNode`s representing each step (LLM calls, tool
//! invocations). Runs are persisted to a JSONL file and kept in a bounded
//! in-memory ring for fast queries.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Run status
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Stopped,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Stopped)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Run node
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    LlmRequest,
    ToolCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunNode {
    pub node_id: u32,
    pub kind: NodeKind,
    pub name: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Summary of input (truncated for display).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_preview: Option<String>,
    /// Summary of output (truncated for display).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Run record
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub run_id: Uuid,
    pub session_key: String,
    pub session_id: String,
    pub status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    /// First ~200 chars of the user message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_preview: Option<String>,
    /// First ~200 chars of the final assistant response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub nodes: Vec<RunNode>,
    /// Number of tool-call loop iterations.
    pub loop_count: u32,
}

impl Run {
    pub fn new(session_key: String, session_id: String, user_message: &str) -> Self {
        Self {
            run_id: Uuid::new_v4(),
            session_key,
            session_id,
            status: RunStatus::Queued,
            agent_id: None,
            model: None,
            started_at: Utc::now(),
            ended_at: None,
            duration_ms: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            input_preview: Some(truncate(user_message, 200)),
            output_preview: None,
            error: None,
            nodes: Vec::new(),
            loop_count: 0,
        }
    }

    pub fn finish(&mut self, status: RunStatus) {
        self.status = status;
        self.ended_at = Some(Utc::now());
        self.duration_ms = Some(
            (Utc::now() - self.started_at)
                .num_milliseconds()
                .max(0) as u64,
        );
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Run events (for SSE broadcast)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum RunEvent {
    #[serde(rename = "run.status")]
    RunStatus { run_id: Uuid, status: self::RunStatus },
    #[serde(rename = "node.started")]
    NodeStarted { run_id: Uuid, node: RunNode },
    #[serde(rename = "node.completed")]
    NodeCompleted { run_id: Uuid, node: RunNode },
    #[serde(rename = "node.failed")]
    NodeFailed { run_id: Uuid, node: RunNode },
    #[serde(rename = "log")]
    Log { run_id: Uuid, level: String, message: String },
    #[serde(rename = "usage")]
    Usage { run_id: Uuid, input_tokens: u32, output_tokens: u32, total_tokens: u32 },
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Run store
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const MAX_RUNS_IN_MEMORY: usize = 2000;

pub struct RunStore {
    /// Bounded ring of recent runs (newest last) + O(1) index.
    inner: RwLock<RunStoreInner>,
    /// JSONL persistence path.
    log_path: PathBuf,
    /// Per-run broadcast channels for SSE.
    event_channels: RwLock<HashMap<Uuid, broadcast::Sender<RunEvent>>>,
}

/// Interior state behind the RwLock — VecDeque plus a HashMap index
/// that maps run_id → logical sequence number. The logical offset
/// tracks how many entries have been popped from the front so the
/// HashMap values never need bulk adjustment.
struct RunStoreInner {
    runs: VecDeque<Run>,
    index: HashMap<Uuid, usize>,
    /// Logical sequence number of the front element.
    base_seq: usize,
}

impl RunStoreInner {
    fn new(runs: VecDeque<Run>) -> Self {
        let mut index = HashMap::with_capacity(runs.len());
        for (i, run) in runs.iter().enumerate() {
            index.insert(run.run_id, i);
        }
        Self {
            runs,
            index,
            base_seq: 0,
        }
    }

    /// Convert a logical sequence number to a VecDeque index.
    fn deque_idx(&self, seq: usize) -> usize {
        seq - self.base_seq
    }

    fn get_mut(&mut self, run_id: &Uuid) -> Option<&mut Run> {
        let seq = *self.index.get(run_id)?;
        let idx = self.deque_idx(seq);
        self.runs.get_mut(idx)
    }

    fn get(&self, run_id: &Uuid) -> Option<&Run> {
        let seq = *self.index.get(run_id)?;
        let idx = self.deque_idx(seq);
        self.runs.get(idx)
    }

    fn push_back(&mut self, run: Run) {
        let seq = self.base_seq + self.runs.len();
        self.index.insert(run.run_id, seq);
        self.runs.push_back(run);
    }

    fn pop_front(&mut self) -> Option<Run> {
        let run = self.runs.pop_front()?;
        self.index.remove(&run.run_id);
        self.base_seq += 1;
        Some(run)
    }
}

impl RunStore {
    /// Create a new RunStore, loading recent runs from the JSONL file.
    pub fn new(state_path: &Path) -> Self {
        let dir = state_path.join("runs");
        std::fs::create_dir_all(&dir).ok();

        let log_path = dir.join("runs.jsonl");
        let (runs, total_on_disk) = Self::load_recent(&log_path);

        // Prune the JSONL file if it contained more entries than we kept.
        if total_on_disk > runs.len() {
            tracing::info!(
                kept = runs.len(),
                pruned = total_on_disk - runs.len(),
                "pruning runs JSONL on disk"
            );
            Self::rewrite_jsonl(&log_path, &runs);
        }

        Self {
            inner: RwLock::new(RunStoreInner::new(runs)),
            log_path,
            event_channels: RwLock::new(HashMap::new()),
        }
    }

    /// Load the most recent MAX_RUNS_IN_MEMORY runs from the JSONL file.
    /// Returns (runs, total_line_count) to detect if pruning is needed.
    fn load_recent(path: &Path) -> (VecDeque<Run>, usize) {
        let mut runs = VecDeque::new();
        let mut total = 0;
        if let Ok(content) = std::fs::read_to_string(path) {
            let lines: Vec<&str> = content.lines().collect();
            total = lines.len();
            for line in lines.iter().rev().take(MAX_RUNS_IN_MEMORY) {
                if let Ok(run) = serde_json::from_str::<Run>(line) {
                    runs.push_front(run);
                }
            }
        }
        (runs, total)
    }

    /// Rewrite the JSONL file with only the given runs (disk pruning).
    fn rewrite_jsonl(path: &Path, runs: &VecDeque<Run>) {
        let tmp = path.with_extension("jsonl.tmp");
        let mut ok = false;
        if let Ok(mut f) = std::fs::File::create(&tmp) {
            use std::io::Write;
            ok = true;
            for run in runs {
                if let Ok(json) = serde_json::to_string(run) {
                    if writeln!(f, "{}", json).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if ok {
            let _ = std::fs::rename(&tmp, path);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Insert a new run. Returns the run_id.
    pub fn insert(&self, run: Run) -> Uuid {
        let run_id = run.run_id;
        let mut inner = self.inner.write();
        inner.push_back(run);
        if inner.runs.len() > MAX_RUNS_IN_MEMORY {
            inner.pop_front();
        }
        run_id
    }

    /// Update a run in-place by ID (O(1) via index). Returns true if found.
    pub fn update<F>(&self, run_id: &Uuid, f: F) -> bool
    where
        F: FnOnce(&mut Run),
    {
        let mut inner = self.inner.write();
        if let Some(run) = inner.get_mut(run_id) {
            f(run);
            return true;
        }
        false
    }

    /// Persist a run to the JSONL file (append).
    pub fn persist(&self, run: &Run) {
        if let Ok(json) = serde_json::to_string(run) {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
            {
                let _ = writeln!(file, "{json}");
            }
        }
    }

    /// Get a run by ID (O(1) via index).
    pub fn get(&self, run_id: &Uuid) -> Option<Run> {
        let inner = self.inner.read();
        inner.get(run_id).cloned()
    }

    /// List runs with optional filters and pagination.
    ///
    /// Uses a two-pass approach: first counts total matches, then collects
    /// only the requested page — avoiding an intermediate Vec allocation.
    pub fn list(
        &self,
        status: Option<RunStatus>,
        session_key: Option<&str>,
        agent_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> (Vec<Run>, usize) {
        let inner = self.inner.read();
        let filter = |r: &&Run| -> bool {
            if let Some(s) = status {
                if r.status != s {
                    return false;
                }
            }
            if let Some(sk) = session_key {
                if r.session_key != sk {
                    return false;
                }
            }
            if let Some(aid) = agent_id {
                if r.agent_id.as_deref() != Some(aid) {
                    return false;
                }
            }
            true
        };

        let total = inner.runs.iter().rev().filter(filter).count();
        let page: Vec<Run> = inner
            .runs
            .iter()
            .rev()
            .filter(filter)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();

        (page, total)
    }

    /// Get or create a broadcast channel for a run (for SSE).
    pub fn subscribe(&self, run_id: &Uuid) -> broadcast::Receiver<RunEvent> {
        let mut channels = self.event_channels.write();
        let tx = channels
            .entry(*run_id)
            .or_insert_with(|| broadcast::channel(128).0);
        tx.subscribe()
    }

    /// Emit an event for a run (broadcast to all subscribers).
    pub fn emit(&self, run_id: &Uuid, event: RunEvent) {
        let channels = self.event_channels.read();
        if let Some(tx) = channels.get(run_id) {
            let _ = tx.send(event);
        }
    }

    /// Clean up the broadcast channel for a completed run.
    pub fn cleanup_channel(&self, run_id: &Uuid) {
        let mut channels = self.event_channels.write();
        channels.remove(run_id);
    }

    /// Count runs by status (for dashboard stats).
    pub fn status_counts(&self) -> HashMap<String, usize> {
        let inner = self.inner.read();
        let mut counts = HashMap::new();
        for run in inner.runs.iter() {
            let key = serde_json::to_value(run.status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| format!("{:?}", run.status).to_lowercase());
            *counts.entry(key).or_insert(0) += 1;
        }
        counts
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Re-use the shared truncation helper from the parent module.
use super::truncate_str as truncate;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_lifecycle() {
        let mut run = Run::new("sk".into(), "sid".into(), "hello world");
        assert_eq!(run.status, RunStatus::Queued);
        assert!(run.input_preview.as_deref() == Some("hello world"));

        run.status = RunStatus::Running;
        run.finish(RunStatus::Completed);
        assert_eq!(run.status, RunStatus::Completed);
        assert!(run.ended_at.is_some());
        assert!(run.duration_ms.is_some());
    }

    #[test]
    fn store_insert_and_query() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        let run = Run::new("sk1".into(), "sid1".into(), "msg1");
        let run_id = run.run_id;
        store.insert(run);

        let fetched = store.get(&run_id).unwrap();
        assert_eq!(fetched.session_key, "sk1");

        let (list, total) = store.list(None, None, None, 10, 0);
        assert_eq!(total, 1);
        assert_eq!(list[0].run_id, run_id);
    }

    #[test]
    fn store_update() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        let run = Run::new("sk".into(), "sid".into(), "msg");
        let run_id = run.run_id;
        store.insert(run);

        store.update(&run_id, |r| {
            r.status = RunStatus::Running;
        });

        let fetched = store.get(&run_id).unwrap();
        assert_eq!(fetched.status, RunStatus::Running);
    }

    #[test]
    fn store_filter_by_status() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        let mut run1 = Run::new("sk".into(), "sid".into(), "msg1");
        run1.status = RunStatus::Completed;
        store.insert(run1);

        let mut run2 = Run::new("sk".into(), "sid".into(), "msg2");
        run2.status = RunStatus::Failed;
        store.insert(run2);

        let (completed, _) = store.list(Some(RunStatus::Completed), None, None, 10, 0);
        assert_eq!(completed.len(), 1);

        let (failed, _) = store.list(Some(RunStatus::Failed), None, None, 10, 0);
        assert_eq!(failed.len(), 1);
    }

    #[test]
    fn truncate_unicode_safe() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
        // Multi-byte: "héllo" — truncating at 2 should not split 'é'
        let s = "héllo";
        let t = truncate(s, 2);
        assert!(t.ends_with("..."));
        assert!(t.len() <= 6); // 2 bytes + "..."
    }

    #[test]
    fn persist_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        let mut run = Run::new("sk".into(), "sid".into(), "msg");
        run.status = RunStatus::Completed;
        store.insert(run.clone());
        store.persist(&run);

        // Create a new store from the same path — should reload
        let store2 = RunStore::new(dir.path());
        let fetched = store2.get(&run.run_id).unwrap();
        assert_eq!(fetched.session_key, "sk");
        assert_eq!(fetched.status, RunStatus::Completed);
    }

    #[test]
    fn bounded_ring() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        // Insert MAX_RUNS_IN_MEMORY + 10 runs
        for i in 0..(MAX_RUNS_IN_MEMORY + 10) {
            let run = Run::new(format!("sk{i}"), format!("sid{i}"), &format!("msg{i}"));
            store.insert(run);
        }

        let (list, total) = store.list(None, None, None, MAX_RUNS_IN_MEMORY + 100, 0);
        assert_eq!(total, MAX_RUNS_IN_MEMORY);
        assert_eq!(list.len(), MAX_RUNS_IN_MEMORY);
    }

    #[test]
    fn run_status_is_terminal() {
        assert!(!RunStatus::Queued.is_terminal());
        assert!(!RunStatus::Running.is_terminal());
        assert!(RunStatus::Completed.is_terminal());
        assert!(RunStatus::Failed.is_terminal());
        assert!(RunStatus::Stopped.is_terminal());
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        assert!(store.get(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn update_nonexistent_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());
        let found = store.update(&Uuid::new_v4(), |r| {
            r.status = RunStatus::Running;
        });
        assert!(!found);
    }

    #[test]
    fn list_pagination() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        for i in 0..5 {
            let run = Run::new("sk".into(), "sid".into(), &format!("msg{i}"));
            store.insert(run);
        }

        // Page 1: limit 2, offset 0 (newest first)
        let (page1, total) = store.list(None, None, None, 2, 0);
        assert_eq!(total, 5);
        assert_eq!(page1.len(), 2);

        // Page 2: limit 2, offset 2
        let (page2, _) = store.list(None, None, None, 2, 2);
        assert_eq!(page2.len(), 2);

        // Page 3: limit 2, offset 4 (only 1 remaining)
        let (page3, _) = store.list(None, None, None, 2, 4);
        assert_eq!(page3.len(), 1);

        // No overlap between pages.
        let all_ids: std::collections::HashSet<_> = page1
            .iter()
            .chain(page2.iter())
            .chain(page3.iter())
            .map(|r| r.run_id)
            .collect();
        assert_eq!(all_ids.len(), 5);
    }

    #[test]
    fn list_filter_by_session_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        store.insert(Run::new("alpha".into(), "sid".into(), "msg1"));
        store.insert(Run::new("beta".into(), "sid".into(), "msg2"));
        store.insert(Run::new("alpha".into(), "sid".into(), "msg3"));

        let (hits, total) = store.list(None, Some("alpha"), None, 10, 0);
        assert_eq!(total, 2);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|r| r.session_key == "alpha"));
    }

    #[test]
    fn list_filter_by_agent_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        let mut run1 = Run::new("sk".into(), "sid".into(), "msg1");
        run1.agent_id = Some("planner".into());
        store.insert(run1);

        let run2 = Run::new("sk".into(), "sid".into(), "msg2");
        store.insert(run2);

        let (hits, total) = store.list(None, None, Some("planner"), 10, 0);
        assert_eq!(total, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].agent_id.as_deref(), Some("planner"));
    }

    #[test]
    fn status_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = RunStore::new(dir.path());

        let mut r1 = Run::new("sk".into(), "sid".into(), "msg1");
        r1.status = RunStatus::Completed;
        store.insert(r1);

        let mut r2 = Run::new("sk".into(), "sid".into(), "msg2");
        r2.status = RunStatus::Completed;
        store.insert(r2);

        let mut r3 = Run::new("sk".into(), "sid".into(), "msg3");
        r3.status = RunStatus::Failed;
        store.insert(r3);

        let counts = store.status_counts();
        assert_eq!(counts.get("completed"), Some(&2));
        assert_eq!(counts.get("failed"), Some(&1));
    }

    #[test]
    fn run_input_preview_truncated() {
        let long_msg = "a".repeat(300);
        let run = Run::new("sk".into(), "sid".into(), &long_msg);
        let preview = run.input_preview.as_deref().unwrap();
        // truncate(msg, 200) should produce at most 200 + 3 ("...") = 203 bytes
        assert!(preview.len() <= 203);
        assert!(preview.ends_with("..."));
    }
}
