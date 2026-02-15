//! Process tool — manage background process sessions.
//!
//! Actions: list, poll, log, write, kill, clear, remove.

use serde::{Deserialize, Serialize};

use crate::manager::ProcessManager;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request / Response
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessRequest {
    pub action: ProcessAction,
    /// Session ID (required for all actions except `list` and `clear`).
    #[serde(default)]
    pub session_id: Option<String>,
    /// For `poll`: byte offset to read from.
    #[serde(default)]
    pub offset: Option<usize>,
    /// For `log`: byte limit.
    #[serde(default)]
    pub limit: Option<usize>,
    /// For `log`: number of tail lines (default 200).
    #[serde(default)]
    pub tail_lines: Option<usize>,
    /// For `write`: data to send to stdin.
    #[serde(default)]
    pub data: Option<String>,
    /// For `write`: close stdin after sending.
    #[serde(default)]
    pub eof: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessAction {
    List,
    Poll,
    Log,
    Write,
    Kill,
    Clear,
    Remove,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Handler
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Execute a process management action.
pub async fn handle_process(
    manager: &ProcessManager,
    req: ProcessRequest,
) -> ProcessResponse {
    match req.action {
        ProcessAction::List => {
            let sessions = manager.list();
            ProcessResponse {
                success: true,
                error: None,
                data: Some(serde_json::json!({
                    "sessions": sessions,
                    "count": sessions.len(),
                })),
            }
        }

        ProcessAction::Poll => {
            let sid = match &req.session_id {
                Some(s) => s.as_str(),
                None => {
                    return ProcessResponse {
                        success: false,
                        error: Some("session_id required for poll".into()),
                        data: None,
                    }
                }
            };
            match manager.poll(sid, req.offset.unwrap_or(0)) {
                Some(result) => ProcessResponse {
                    success: true,
                    error: None,
                    data: Some(serde_json::to_value(result).unwrap_or_default()),
                },
                None => ProcessResponse {
                    success: false,
                    error: Some("session not found".into()),
                    data: None,
                },
            }
        }

        ProcessAction::Log => {
            let sid = match &req.session_id {
                Some(s) => s.as_str(),
                None => {
                    return ProcessResponse {
                        success: false,
                        error: Some("session_id required for log".into()),
                        data: None,
                    }
                }
            };
            match manager.log(sid, req.offset, req.limit, req.tail_lines) {
                Some(log) => ProcessResponse {
                    success: true,
                    error: None,
                    data: Some(serde_json::json!({ "log": log })),
                },
                None => ProcessResponse {
                    success: false,
                    error: Some("session not found".into()),
                    data: None,
                },
            }
        }

        ProcessAction::Write => {
            let sid = match &req.session_id {
                Some(s) => s.as_str(),
                None => {
                    return ProcessResponse {
                        success: false,
                        error: Some("session_id required for write".into()),
                        data: None,
                    }
                }
            };
            let data = req.data.unwrap_or_default().into_bytes();
            let ok = manager.write_stdin(sid, data, req.eof).await;
            ProcessResponse {
                success: ok,
                error: if ok { None } else { Some("session not found or stdin closed".into()) },
                data: None,
            }
        }

        ProcessAction::Kill => {
            let sid = match &req.session_id {
                Some(s) => s.as_str(),
                None => {
                    return ProcessResponse {
                        success: false,
                        error: Some("session_id required for kill".into()),
                        data: None,
                    }
                }
            };
            let ok = manager.kill(sid);
            ProcessResponse {
                success: ok,
                error: if ok { None } else { Some("session not found or not running".into()) },
                data: None,
            }
        }

        ProcessAction::Clear => {
            let cleared = manager.clear_finished();
            ProcessResponse {
                success: true,
                error: None,
                data: Some(serde_json::json!({ "cleared": cleared })),
            }
        }

        ProcessAction::Remove => {
            let sid = match &req.session_id {
                Some(s) => s.as_str(),
                None => {
                    return ProcessResponse {
                        success: false,
                        error: Some("session_id required for remove".into()),
                        data: None,
                    }
                }
            };
            let ok = manager.remove(sid);
            ProcessResponse {
                success: ok,
                error: if ok { None } else { Some("session not found".into()) },
                data: None,
            }
        }
    }
}
