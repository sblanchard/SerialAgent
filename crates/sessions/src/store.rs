//! Gateway-owned session store.
//!
//! Persists session state in `sessions.json` under the configured state path.
//! Each session key maps to a `SessionEntry` tracking the session ID, token
//! counters, origin metadata, and the SerialMemory session ID.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use sa_domain::error::{Error, Result};
use sa_domain::trace::TraceEvent;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Session entry
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A single session tracked by the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub session_key: String,
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The model used for this session (e.g. `"openai/gpt-4o"`).
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub context_tokens: u64,
    /// SerialMemory session ID (from `init_session`).
    #[serde(default)]
    pub sm_session_id: Option<String>,
    #[serde(default)]
    pub origin: SessionOrigin,
}

/// Origin metadata describing where the session came from.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionOrigin {
    pub channel: Option<String>,
    pub account: Option<String>,
    pub peer: Option<String>,
    pub group: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Session store
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Gateway-owned session store backed by a JSON file.
pub struct SessionStore {
    sessions_path: PathBuf,
    sessions: RwLock<HashMap<String, SessionEntry>>,
}

impl SessionStore {
    /// Load or create the session store at `state_path/sessions/sessions.json`.
    pub fn new(state_path: &Path) -> Result<Self> {
        let dir = state_path.join("sessions");
        std::fs::create_dir_all(&dir)
            .map_err(Error::Io)?;

        let sessions_path = dir.join("sessions.json");
        let sessions = if sessions_path.exists() {
            let raw = std::fs::read_to_string(&sessions_path)
                .map_err(Error::Io)?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            HashMap::new()
        };

        tracing::info!(
            sessions = sessions.len(),
            path = %sessions_path.display(),
            "session store loaded"
        );

        Ok(Self {
            sessions_path,
            sessions: RwLock::new(sessions),
        })
    }

    /// Look up a session by its key.
    pub fn get(&self, session_key: &str) -> Option<SessionEntry> {
        self.sessions.read().get(session_key).cloned()
    }

    /// Resolve or create a session for the given key.  Returns `(entry, is_new)`.
    pub fn resolve_or_create(
        &self,
        session_key: &str,
        origin: SessionOrigin,
    ) -> (SessionEntry, bool) {
        // Fast path: session already exists.
        {
            let sessions = self.sessions.read();
            if let Some(entry) = sessions.get(session_key) {
                return (entry.clone(), false);
            }
        }

        // Slow path: create new session.
        let now = Utc::now();
        let session_id = uuid::Uuid::new_v4().to_string();
        let entry = SessionEntry {
            session_key: session_key.to_owned(),
            session_id: session_id.clone(),
            created_at: now,
            updated_at: now,
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            context_tokens: 0,
            sm_session_id: None,
            origin,
        };

        let mut sessions = self.sessions.write();
        sessions.insert(session_key.to_owned(), entry.clone());

        TraceEvent::SessionResolved {
            session_key: session_key.to_owned(),
            session_id,
            is_new: true,
        }
        .emit();

        (entry, true)
    }

    /// Record a session reset: mint a new session ID for the same key.
    pub fn reset_session(
        &self,
        session_key: &str,
        reason: &str,
    ) -> Option<SessionEntry> {
        let mut sessions = self.sessions.write();
        let entry = sessions.get_mut(session_key)?;

        let old_id = entry.session_id.clone();
        let new_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        entry.session_id = new_id.clone();
        entry.created_at = now;
        entry.updated_at = now;
        entry.input_tokens = 0;
        entry.output_tokens = 0;
        entry.total_tokens = 0;
        entry.context_tokens = 0;
        entry.sm_session_id = None;

        TraceEvent::SessionReset {
            session_key: session_key.to_owned(),
            old_session_id: old_id,
            new_session_id: new_id,
            reason: reason.to_owned(),
        }
        .emit();

        Some(entry.clone())
    }

    /// Update token counters for a session.
    pub fn record_usage(
        &self,
        session_key: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) {
        let mut sessions = self.sessions.write();
        if let Some(entry) = sessions.get_mut(session_key) {
            entry.input_tokens += input_tokens;
            entry.output_tokens += output_tokens;
            entry.total_tokens += input_tokens + output_tokens;
            entry.updated_at = Utc::now();
        }
    }

    /// Store the SerialMemory session ID for a session.
    pub fn set_sm_session_id(
        &self,
        session_key: &str,
        sm_session_id: String,
    ) {
        let mut sessions = self.sessions.write();
        if let Some(entry) = sessions.get_mut(session_key) {
            entry.sm_session_id = Some(sm_session_id);
        }
    }

    /// Touch the updated_at timestamp.
    pub fn touch(&self, session_key: &str) {
        let mut sessions = self.sessions.write();
        if let Some(entry) = sessions.get_mut(session_key) {
            entry.updated_at = Utc::now();
        }
    }

    /// List all session entries.
    pub fn list(&self) -> Vec<SessionEntry> {
        self.sessions.read().values().cloned().collect()
    }

    /// Persist the current session state to disk.
    pub fn flush(&self) -> Result<()> {
        let sessions = self.sessions.read();
        let json = serde_json::to_string_pretty(&*sessions)
            .map_err(|e| Error::Other(format!("serializing sessions: {e}")))?;
        std::fs::write(&self.sessions_path, json)
            .map_err(Error::Io)?;
        Ok(())
    }

    /// Return the transcript directory for a given session ID.
    pub fn transcript_dir(&self) -> PathBuf {
        self.sessions_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf()
    }
}
