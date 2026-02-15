use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use sa_contextpack::builder::WorkspaceFile;
use sa_domain::trace::TraceEvent;

#[derive(Debug, Clone)]
struct CachedFile {
    content: String,
    hash: String,
    modified: SystemTime,
    size: u64,
}

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

        let content = std::fs::read_to_string(&path).ok()?;
        let hash = {
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            format!("{:x}", hasher.finalize())
        };
        let raw_chars = content.len();

        self.cache.write().insert(
            name.to_string(),
            CachedFile {
                content: content.clone(),
                hash,
                modified,
                size,
            },
        );

        TraceEvent::WorkspaceFileRead {
            filename: name.to_string(),
            raw_chars,
            cache_hit: false,
        }
        .emit();

        Some(content)
    }

    /// Read all expected workspace files as WorkspaceFile structs
    /// (with None content for missing files).
    pub fn read_all_context_files(&self) -> Vec<WorkspaceFile> {
        let all_names = [
            "AGENTS.md",
            "SOUL.md",
            "USER.md",
            "IDENTITY.md",
            "TOOLS.md",
            "HEARTBEAT.md",
            "BOOTSTRAP.md",
            "MEMORY.md",
        ];

        all_names
            .iter()
            .map(|&name| WorkspaceFile {
                name: name.to_string(),
                content: self.read_file(name),
            })
            .collect()
    }

    pub fn list_present_files(&self) -> Vec<String> {
        let names = [
            "AGENTS.md",
            "SOUL.md",
            "USER.md",
            "IDENTITY.md",
            "TOOLS.md",
            "HEARTBEAT.md",
            "BOOTSTRAP.md",
            "MEMORY.md",
        ];
        names
            .iter()
            .filter(|&&name| self.root.join(name).exists())
            .map(|&s| s.to_string())
            .collect()
    }

    pub fn file_hash(&self, name: &str) -> Option<FileHash> {
        let cache = self.cache.read();
        cache.get(name).map(|c| FileHash {
            sha256: c.hash.clone(),
            size: c.size,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}
