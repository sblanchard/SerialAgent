use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::trace::TraceEvent;

/// Cache entry for a workspace file.
#[derive(Debug, Clone)]
struct CachedFile {
    content: String,
    hash: String,
    modified: SystemTime,
    size: u64,
}

/// File hash info exposed via /v1/context.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileHash {
    pub sha256: String,
    pub size: u64,
}

/// Reads and caches workspace context files with mtime + size + sha256 invalidation.
pub struct WorkspaceReader {
    root: PathBuf,
    cache: RwLock<HashMap<String, CachedFile>>,
}

impl WorkspaceReader {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Read a workspace file by name (e.g. "SOUL.md").
    ///
    /// Returns `None` if the file doesn't exist.
    /// Uses cache when mtime + size haven't changed.
    pub fn read_file(&self, name: &str) -> Option<String> {
        let path = self.root.join(name);

        if !path.exists() {
            return None;
        }

        let metadata = std::fs::metadata(&path).ok()?;
        let modified = metadata.modified().ok()?;
        let size = metadata.len();

        // Check cache
        {
            let cache = self.cache.read();
            if let Some(cached) = cache.get(name) {
                if cached.modified == modified && cached.size == size {
                    TraceEvent::WorkspaceFileRead {
                        filename: name.to_string(),
                        raw_chars: cached.content.len(),
                        cache_hit: true,
                    }
                    .emit();
                    return Some(cached.content.clone());
                }
            }
        }

        // Cache miss — read from disk
        let content = std::fs::read_to_string(&path).ok()?;
        let hash = compute_sha256(&content);
        let raw_chars = content.len();

        let cached = CachedFile {
            content: content.clone(),
            hash,
            modified,
            size,
        };

        self.cache.write().insert(name.to_string(), cached);

        TraceEvent::WorkspaceFileRead {
            filename: name.to_string(),
            raw_chars,
            cache_hit: false,
        }
        .emit();

        Some(content)
    }

    /// Get the file hash for a workspace file (for /v1/context report).
    pub fn file_hash(&self, name: &str) -> Option<FileHash> {
        let cache = self.cache.read();
        cache.get(name).map(|c| FileHash {
            sha256: c.hash.clone(),
            size: c.size,
        })
    }

    /// List all context files that exist in the workspace.
    pub fn list_present_files(&self) -> Vec<String> {
        let names = [
            "AGENTS.md",
            "SOUL.md",
            "TOOLS.md",
            "IDENTITY.md",
            "USER.md",
            "HEARTBEAT.md",
            "BOOTSTRAP.md",
        ];

        names
            .iter()
            .filter(|&&name| self.root.join(name).exists())
            .map(|&s| s.to_string())
            .collect()
    }

    /// Invalidate the cache for a specific file (e.g. after edit via dashboard).
    pub fn invalidate(&self, name: &str) {
        self.cache.write().remove(name);
    }

    /// Invalidate the entire cache.
    pub fn invalidate_all(&self) {
        self.cache.write().clear();
    }

    /// Get the workspace root path.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn compute_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}
