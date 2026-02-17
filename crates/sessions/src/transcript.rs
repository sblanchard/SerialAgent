//! Append-only JSONL transcripts.
//!
//! Each session gets a `<sessionId>.jsonl` file under the sessions directory.
//! Every inbound/outbound message is appended as a single JSON line.
//!
//! Includes an in-memory write-through cache to avoid re-reading from disk
//! every turn, and async I/O wrappers to avoid blocking the tokio runtime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use parking_lot::RwLock;
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

/// Writes append-only JSONL transcript files with an in-memory write-through
/// cache so reads never hit disk after the first load.
pub struct TranscriptWriter {
    base_dir: PathBuf,
    cache: RwLock<HashMap<String, Vec<TranscriptLine>>>,
}

impl TranscriptWriter {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Append one or more lines to a session's transcript (sync).
    ///
    /// Writes through to both the in-memory cache and disk.
    pub fn append(
        &self,
        session_id: &str,
        lines: &[TranscriptLine],
    ) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }

        // Write to disk first — only update cache if I/O succeeds.
        self.write_to_disk(session_id, lines)?;

        {
            let mut cache = self.cache.write();
            cache
                .entry(session_id.to_owned())
                .or_default()
                .extend(lines.iter().cloned());
        }

        TraceEvent::TranscriptAppend {
            session_id: session_id.to_owned(),
            lines: lines.len(),
        }
        .emit();

        Ok(())
    }

    /// Append one or more lines to a session's transcript (async).
    ///
    /// Uses `spawn_blocking` to avoid blocking the tokio runtime during file I/O.
    pub async fn append_async(
        &self,
        session_id: &str,
        lines: &[TranscriptLine],
    ) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }

        // Serialize lines for the blocking task.
        let buf = serialize_lines(lines)?;
        let path = self.base_dir.join(format!("{session_id}.jsonl"));
        let line_count = lines.len();
        let sid = session_id.to_owned();

        // Write to disk first — only update cache if I/O succeeds.
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(Error::Io)?;
            file.write_all(buf.as_bytes()).map_err(Error::Io)?;
            Ok::<(), Error>(())
        })
        .await
        .map_err(|e| Error::Other(format!("spawn_blocking join: {e}")))??;

        {
            let mut cache = self.cache.write();
            cache
                .entry(session_id.to_owned())
                .or_default()
                .extend(lines.iter().cloned());
        }

        TraceEvent::TranscriptAppend {
            session_id: sid,
            lines: line_count,
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

    /// Read back a transcript. Returns cached lines if available, otherwise
    /// loads from disk and populates the cache.
    pub fn read(&self, session_id: &str) -> Result<Vec<TranscriptLine>> {
        // Fast path: return from cache.
        {
            let cache = self.cache.read();
            if let Some(lines) = cache.get(session_id) {
                return Ok(lines.clone());
            }
        }

        // Slow path: load from disk and populate cache.
        let lines = self.read_from_disk(session_id)?;
        {
            let mut cache = self.cache.write();
            cache.insert(session_id.to_owned(), lines.clone());
        }
        Ok(lines)
    }

    /// Read back a transcript (async). Returns cached lines if available,
    /// otherwise loads from disk via `spawn_blocking` and populates the cache.
    pub async fn read_async(&self, session_id: &str) -> Result<Vec<TranscriptLine>> {
        // Fast path: return from cache.
        {
            let cache = self.cache.read();
            if let Some(lines) = cache.get(session_id) {
                return Ok(lines.clone());
            }
        }

        // Slow path: load from disk on a blocking thread.
        let path = self.base_dir.join(format!("{session_id}.jsonl"));
        let sid = session_id.to_owned();

        let lines = tokio::task::spawn_blocking(move || {
            read_jsonl_file(&path, &sid)
        })
        .await
        .map_err(|e| Error::Other(format!("spawn_blocking join: {e}")))??;

        // Populate cache.
        {
            let mut cache = self.cache.write();
            cache.insert(session_id.to_owned(), lines.clone());
        }
        Ok(lines)
    }

    /// Invalidate the cache for a session (e.g. after compaction rewrites
    /// the transcript on disk outside normal append flow).
    pub fn invalidate_cache(&self, session_id: &str) {
        let mut cache = self.cache.write();
        cache.remove(session_id);
    }

    // ── Private helpers ───────────────────────────────────────────────

    fn write_to_disk(&self, session_id: &str, lines: &[TranscriptLine]) -> Result<()> {
        let path = self.base_dir.join(format!("{session_id}.jsonl"));
        let buf = serialize_lines(lines)?;

        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(Error::Io)?;
        file.write_all(buf.as_bytes()).map_err(Error::Io)?;
        Ok(())
    }

    fn read_from_disk(&self, session_id: &str) -> Result<Vec<TranscriptLine>> {
        let path = self.base_dir.join(format!("{session_id}.jsonl"));
        read_jsonl_file(&path, session_id)
    }
}

/// Serialize transcript lines to a JSONL string.
fn serialize_lines(lines: &[TranscriptLine]) -> Result<String> {
    let mut buf = String::new();
    for line in lines {
        let json = serde_json::to_string(line)
            .map_err(|e| Error::Other(format!("serializing transcript line: {e}")))?;
        buf.push_str(&json);
        buf.push('\n');
    }
    Ok(buf)
}

/// Read and parse a JSONL transcript file.
fn read_jsonl_file(path: &Path, session_id: &str) -> Result<Vec<TranscriptLine>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw = std::fs::read_to_string(path).map_err(Error::Io)?;
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
