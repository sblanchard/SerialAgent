//! Exec tool — spawn a command foreground or background.
//!
//! OpenClaw semantics:
//! - Foreground: run command, wait up to `yield_ms`, return output.
//! - Background: spawn command, return immediately with session ID + initial tail.
//! - If foreground exceeds `yield_ms`, auto-background and return session ID.

use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Notify};

use crate::manager::{
    OutputBuffer, ProcessManager, ProcessSession, ProcessStatus, StdinMessage,
};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request / Response
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Deserialize)]
pub struct ExecRequest {
    pub command: String,
    #[serde(default)]
    pub background: bool,
    /// Override yield time (ms).  0 = wait forever (foreground).
    pub yield_ms: Option<u64>,
    /// Override hard timeout (seconds).
    pub timeout_sec: Option<u64>,
    /// Working directory.
    #[serde(default)]
    pub workdir: Option<String>,
    /// Extra environment variables.
    #[serde(default)]
    pub env: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecResponse {
    pub status: ProcessStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Exec logic
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Check if an environment variable name is dangerous to override.
fn is_dangerous_env_var(name: &str) -> bool {
    const BLOCKED: &[&str] = &[
        "LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT",
        "DYLD_INSERT_LIBRARIES", "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH",
        "PATH", "HOME", "USER", "SHELL",
        "SSH_AUTH_SOCK", "SSH_AGENT_PID",
        "PYTHONPATH", "PYTHONSTARTUP", "PYTHONHOME",
        "NODE_PATH", "NODE_OPTIONS",
        "RUBYLIB", "RUBYOPT",
        "PERL5LIB", "PERL5OPT",
        "CLASSPATH",
        "BASH_ENV", "ENV", "CDPATH",
        "IFS",
    ];
    let upper = name.to_ascii_uppercase();
    BLOCKED.contains(&upper.as_str())
}

/// Execute a command, returning either the completed output (foreground)
/// or a session ID (background / auto-backgrounded).
pub async fn exec(
    manager: &ProcessManager,
    req: ExecRequest,
) -> ExecResponse {
    let cfg = manager.config();
    let yield_ms = if req.background {
        0 // immediate background
    } else {
        req.yield_ms.unwrap_or(cfg.background_ms)
    };
    let timeout_sec = req.timeout_sec.unwrap_or(cfg.timeout_sec);

    // Spawn the child process.
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(&req.command);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::piped());

    if let Some(ref wd) = req.workdir {
        cmd.current_dir(wd);
    }
    if let Some(ref env) = req.env {
        for (k, v) in env {
            if is_dangerous_env_var(k) {
                return ExecResponse {
                    status: ProcessStatus::Failed,
                    exit_code: None,
                    output: Some(format!("environment variable '{k}' is blocked by security policy")),
                    session_id: None,
                    tail: None,
                };
            }
            cmd.env(k, v);
        }
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ExecResponse {
                status: ProcessStatus::Failed,
                exit_code: None,
                output: Some(format!("failed to spawn: {e}")),
                session_id: None,
                tail: None,
            };
        }
    };

    // Create the session.
    let (stdin_tx, stdin_rx) = mpsc::channel::<StdinMessage>(32);
    let (kill_tx, kill_rx) = mpsc::channel::<()>(1);

    let session = ProcessSession {
        id: session_id.clone(),
        command: req.command.clone(),
        workdir: req.workdir.clone(),
        started_at: Utc::now(),
        finished_at: None,
        status: ProcessStatus::Running,
        exit_code: None,
        output: OutputBuffer::new(cfg.max_output_chars),
        stdin_tx: Some(stdin_tx),
        kill_tx: Some(kill_tx),
        name: None,
    };

    let session_arc = manager.register(session);

    // Notify used to wake the foreground waiter when the process finishes,
    // eliminating the need for a 50ms polling loop.
    let done_notify = Arc::new(Notify::new());

    // Spawn the background monitoring task.
    spawn_monitor(child, session_arc.clone(), stdin_rx, kill_rx, timeout_sec, done_notify.clone());

    // If background: return immediately.
    if req.background {
        return ExecResponse {
            status: ProcessStatus::Running,
            exit_code: None,
            output: None,
            session_id: Some(session_id),
            tail: Some(String::new()),
        };
    }

    // Foreground: wait for completion or yield deadline, whichever comes first.
    let yield_dur = if yield_ms > 0 {
        std::time::Duration::from_millis(yield_ms)
    } else {
        // "0 yield" with foreground: wait up to timeout_sec
        std::time::Duration::from_secs(timeout_sec)
    };

    tokio::select! {
        _ = done_notify.notified() => {
            let s = session_arc.read();
            ExecResponse {
                status: s.status,
                exit_code: s.exit_code,
                output: Some(s.output.combined.clone()),
                session_id: None,
                tail: None,
            }
        }
        _ = tokio::time::sleep(yield_dur) => {
            // Auto-background: process is still running.
            let tail = session_arc.read().output.tail(20);
            ExecResponse {
                status: ProcessStatus::Running,
                exit_code: None,
                output: None,
                session_id: Some(session_id),
                tail: Some(tail),
            }
        }
    }
}

/// Spawn the background task that monitors the child process.
fn spawn_monitor(
    mut child: tokio::process::Child,
    session: Arc<parking_lot::RwLock<ProcessSession>>,
    mut stdin_rx: mpsc::Receiver<StdinMessage>,
    mut kill_rx: mpsc::Receiver<()>,
    timeout_sec: u64,
    done_notify: Arc<Notify>,
) {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    tokio::spawn(async move {
        // Stdout reader.
        let session_out = session.clone();
        let stdout_task = tokio::spawn(async move {
            if let Some(stdout) = stdout {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let mut s = session_out.write();
                    s.output.push(&line);
                    s.output.push("\n");
                }
            }
        });

        // Stderr reader (merged into combined output).
        let session_err = session.clone();
        let stderr_task = tokio::spawn(async move {
            if let Some(stderr) = stderr {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let mut s = session_err.write();
                    s.output.push(&line);
                    s.output.push("\n");
                }
            }
        });

        // Stdin writer.
        let stdin_task = tokio::spawn(async move {
            if let Some(mut stdin) = stdin {
                while let Some(msg) = stdin_rx.recv().await {
                    match msg {
                        StdinMessage::Data(data) => {
                            let _ = stdin.write_all(&data).await;
                            let _ = stdin.flush().await;
                        }
                        StdinMessage::Eof => {
                            drop(stdin);
                            return;
                        }
                    }
                }
            }
        });

        // Wait for process to exit, kill signal, or timeout.
        let timeout_dur = std::time::Duration::from_secs(timeout_sec);
        let status;

        tokio::select! {
            result = child.wait() => {
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                stdin_task.abort();

                match result {
                    Ok(exit) => {
                        let mut s = session.write();
                        s.exit_code = exit.code();
                        s.status = ProcessStatus::Finished;
                        s.finished_at = Some(Utc::now());
                        s.stdin_tx = None;
                        s.kill_tx = None;
                        status = ProcessStatus::Finished;
                    }
                    Err(e) => {
                        let mut s = session.write();
                        s.output.push(&format!("\n[process error: {e}]"));
                        s.status = ProcessStatus::Failed;
                        s.finished_at = Some(Utc::now());
                        s.stdin_tx = None;
                        s.kill_tx = None;
                        status = ProcessStatus::Failed;
                    }
                }
            }
            _ = kill_rx.recv() => {
                let _ = child.kill().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                stdin_task.abort();

                let mut s = session.write();
                s.output.push("\n[killed]");
                s.status = ProcessStatus::Killed;
                s.finished_at = Some(Utc::now());
                s.stdin_tx = None;
                s.kill_tx = None;
                status = ProcessStatus::Killed;
            }
            _ = tokio::time::sleep(timeout_dur) => {
                let _ = child.kill().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                stdin_task.abort();

                let mut s = session.write();
                s.output.push("\n[timed out]");
                s.status = ProcessStatus::TimedOut;
                s.finished_at = Some(Utc::now());
                s.stdin_tx = None;
                s.kill_tx = None;
                status = ProcessStatus::TimedOut;
            }
        }

        // Wake any foreground waiter.
        done_notify.notify_waiters();

        tracing::debug!(
            session_id = %session.read().id,
            status = ?status,
            "process monitor completed"
        );
    });
}
