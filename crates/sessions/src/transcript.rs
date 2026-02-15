//! Append-only JSONL transcripts.
//!
//! Each session gets a `<sessionId>.jsonl` file under the sessions directory.
//! Every inbound/outbound message is appended as a single JSON line.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sa_domain::error::{Error, Result};
use sa_domain::trace::TraceEvent;

/// A single transcript line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptLine {
    pub timestamp: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Writes append-only JSONL transcript files.
pub struct TranscriptWriter {
    base_dir: PathBuf,
}

impl TranscriptWriter {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
        }
    }

    /// Append one or more lines to a session's transcript.
    pub fn append(
        &self,
        session_id: &str,
        lines: &[TranscriptLine],
    ) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }

        let path = self.base_dir.join(format!("{session_id}.jsonl"));
        let mut buf = String::new();
        for line in lines {
            let json = serde_json::to_string(line)
                .map_err(|e| Error::Other(format!("serializing transcript line: {e}")))?;
            buf.push_str(&json);
            buf.push('\n');
        }

        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(Error::Io)?;
        file.write_all(buf.as_bytes()).map_err(Error::Io)?;

        TraceEvent::TranscriptAppend {
            session_id: session_id.to_owned(),
            lines: lines.len(),
        }
        .emit();

        Ok(())
    }

    /// Helper to create a transcript line with the current timestamp.
    pub fn line(role: &str, content: &str) -> TranscriptLine {
        TranscriptLine {
            timestamp: Utc::now().to_rfc3339(),
            role: role.to_owned(),
            content: content.to_owned(),
            metadata: None,
        }
    }

    /// Read back a transcript (for debugging / dashboard).
    pub fn read(&self, session_id: &str) -> Result<Vec<TranscriptLine>> {
        let path = self.base_dir.join(format!("{session_id}.jsonl"));
        if !path.exists() {
            return Ok(Vec::new());
        }

        let raw = std::fs::read_to_string(&path).map_err(Error::Io)?;
        let mut lines = Vec::new();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<TranscriptLine>(line) {
                Ok(tl) => lines.push(tl),
                Err(e) => {
                    tracing::warn!(
                        session_id = session_id,
                        error = %e,
                        "skipping malformed transcript line"
                    );
                }
            }
        }
        Ok(lines)
    }
}
