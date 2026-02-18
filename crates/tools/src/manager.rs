//! In-memory process manager.
//!
//! Tracks background process sessions, their output buffers, and lifecycle.
//! The manager owns no child processes directly — each spawn creates a
//! background tokio task that writes into the shared `ProcessSession`.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::Serialize;
use tokio::sync::mpsc;

use sa_domain::config::ExecConfig;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessStatus {
    Running,
    Finished,
    Killed,
    TimedOut,
    Failed,
}

/// Shared mutable state for a single background process.
pub struct ProcessSession {
    pub id: String,
    pub command: String,
    pub workdir: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub output: OutputBuffer,
    /// Send data to the child's stdin.
    pub stdin_tx: Option<mpsc::Sender<StdinMessage>>,
    /// Send a kill signal to the background task.
    pub kill_tx: Option<mpsc::Sender<()>>,
    pub name: Option<String>,
}

pub struct OutputBuffer {
    pub combined: String,
    pub max_chars: usize,
}

impl OutputBuffer {
    pub fn new(max_chars: usize) -> Self {
        Self {
            combined: String::new(),
            max_chars,
        }
    }

    pub fn push(&mut self, text: &str) {
        self.combined.push_str(text);
        if self.combined.len() > self.max_chars {
            let keep = self.max_chars * 3 / 4;
            let drain_count = self.combined.len() - keep;
            // Find a char boundary to avoid splitting a multi-byte character.
            let mut boundary = drain_count;
            while boundary < self.combined.len() && !self.combined.is_char_boundary(boundary) {
                boundary += 1;
            }
            self.combined.drain(..boundary);
        }
    }

    pub fn len(&self) -> usize {
        self.combined.len()
    }

    pub fn is_empty(&self) -> bool {
        self.combined.is_empty()
    }

    pub fn tail(&self, lines: usize) -> String {
        let all_lines: Vec<&str> = self.combined.lines().collect();
        if all_lines.len() <= lines {
            self.combined.clone()
        } else {
            all_lines[all_lines.len() - lines..].join("\n")
        }
    }

    pub fn read_from(&self, offset: usize, limit: Option<usize>) -> &str {
        let start = offset.min(self.combined.len());
        let end = match limit {
            Some(l) => (start + l).min(self.combined.len()),
            None => self.combined.len(),
        };
        &self.combined[start..end]
    }
}

/// Messages that can be sent to a process's stdin.
pub enum StdinMessage {
    Data(Vec<u8>),
    Eof,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ProcessManager
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// In-memory registry of all active and recently-finished processes.
pub struct ProcessManager {
    sessions: RwLock<HashMap<String, Arc<RwLock<ProcessSession>>>>,
    config: ExecConfig,
}

impl ProcessManager {
    pub fn new(config: ExecConfig) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            config,
        }
    }

    pub fn config(&self) -> &ExecConfig {
        &self.config
    }

    /// Register a new process session.
    pub fn register(&self, session: ProcessSession) -> Arc<RwLock<ProcessSession>> {
        let id = session.id.clone();
        let arc = Arc::new(RwLock::new(session));
        self.sessions.write().insert(id, arc.clone());
        arc
    }

    /// Get a process session by ID.
    pub fn get(&self, id: &str) -> Option<Arc<RwLock<ProcessSession>>> {
        self.sessions.read().get(id).cloned()
    }

    /// List all process sessions with their current status.
    pub fn list(&self) -> Vec<ProcessInfo> {
        self.sessions
            .read()
            .values()
            .map(|s| {
                let s = s.read();
                ProcessInfo {
                    id: s.id.clone(),
                    command: s.command.clone(),
                    status: s.status,
                    exit_code: s.exit_code,
                    started_at: s.started_at,
                    finished_at: s.finished_at,
                    output_chars: s.output.len(),
                    name: s.name.clone(),
                }
            })
            .collect()
    }

    /// Poll a process: return incremental output since `offset` + current status.
    pub fn poll(&self, id: &str, offset: usize) -> Option<PollResult> {
        let sessions = self.sessions.read();
        let arc = sessions.get(id)?;
        let s = arc.read();
        Some(PollResult {
            status: s.status,
            exit_code: s.exit_code,
            new_output: s.output.read_from(offset, None).to_owned(),
            next_offset: s.output.len(),
        })
    }

    /// Read the log of a process (offset + limit, default tail 200 lines).
    pub fn log(&self, id: &str, offset: Option<usize>, limit: Option<usize>, tail_lines: Option<usize>) -> Option<String> {
        let sessions = self.sessions.read();
        let arc = sessions.get(id)?;
        let s = arc.read();
        if let Some(off) = offset {
            Some(s.output.read_from(off, limit).to_owned())
        } else {
            Some(s.output.tail(tail_lines.unwrap_or(200)))
        }
    }

    /// Kill a running process.
    pub fn kill(&self, id: &str) -> bool {
        let sessions = self.sessions.read();
        if let Some(arc) = sessions.get(id) {
            let s = arc.read();
            if s.status == ProcessStatus::Running {
                if let Some(ref tx) = s.kill_tx {
                    let _ = tx.try_send(());
                    return true;
                }
            }
        }
        false
    }

    /// Write data to a process's stdin.
    pub async fn write_stdin(&self, id: &str, data: Vec<u8>, eof: bool) -> bool {
        let tx = {
            let sessions = self.sessions.read();
            let arc = sessions.get(id);
            arc.and_then(|a| {
                let s = a.read();
                s.stdin_tx.clone()
            })
        };

        if let Some(tx) = tx {
            if !data.is_empty() {
                let _ = tx.send(StdinMessage::Data(data)).await;
            }
            if eof {
                let _ = tx.send(StdinMessage::Eof).await;
            }
            true
        } else {
            false
        }
    }

    /// Remove all finished sessions.
    pub fn clear_finished(&self) -> usize {
        let mut sessions = self.sessions.write();
        let before = sessions.len();
        sessions.retain(|_, v| {
            let s = v.read();
            s.status == ProcessStatus::Running
        });
        before - sessions.len()
    }

    /// Remove a specific session (kill if running, then remove).
    pub fn remove(&self, id: &str) -> bool {
        // Kill first if running.
        self.kill(id);
        self.sessions.write().remove(id).is_some()
    }

    /// Cleanup sessions older than cleanup_ms.
    pub fn cleanup_stale(&self) {
        let cutoff_ms = self.config.cleanup_ms as i64;
        let now = Utc::now();
        let mut sessions = self.sessions.write();
        sessions.retain(|_, v| {
            let s = v.read();
            match s.finished_at {
                Some(finished) => {
                    let age_ms = now.signed_duration_since(finished).num_milliseconds();
                    age_ms < cutoff_ms
                }
                None => true, // still running
            }
        });
    }
}

/// Summary info for a process (returned by list).
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub id: String,
    pub command: String,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub output_chars: usize,
    pub name: Option<String>,
}

/// Result of polling a process.
#[derive(Debug, Clone, Serialize)]
pub struct PollResult {
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub new_output: String,
    pub next_offset: usize,
}
