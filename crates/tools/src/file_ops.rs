//! File operation tools — safe, auditable file I/O constrained to a workspace root.
//!
//! Each tool takes a `workspace_root: &Path` parameter that constrains where
//! files can be accessed.  Paths containing `..` after canonicalization or
//! resolving outside the workspace are rejected.
//!
//! All functions return `Result<Value, String>` with structured JSON results.

use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;
use tokio::io::AsyncWriteExt;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Request types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Deserialize)]
pub struct FileReadRequest {
    pub path: String,
    /// Line number to start from (0-indexed).
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum number of lines to return.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileWriteRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileAppendRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileMoveRequest {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileDeleteRequest {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileListRequest {
    #[serde(default = "default_dot")]
    pub path: String,
}

fn default_dot() -> String {
    ".".into()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Response types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub size: u64,
    pub modified: String,
    pub is_dir: bool,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Path validation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Validate and resolve a requested path within a workspace root.
///
/// 1. Rejects paths that contain `..` components in the raw input.
/// 2. Joins the requested path onto the workspace root.
/// 3. Canonicalizes the workspace root and checks the resolved path
///    is still contained within it.
///
/// Returns the validated absolute path.
pub fn validate_path(workspace_root: &Path, requested: &str) -> Result<PathBuf, String> {
    // Reject absolute paths — all paths must be relative to the workspace.
    let requested_path = Path::new(requested);
    if requested_path.is_absolute() {
        return Err(format!(
            "absolute paths are not allowed; use a path relative to the workspace root (got '{requested}')"
        ));
    }

    // Reject raw `..` components before any resolution.
    for component in requested_path.components() {
        if matches!(component, Component::ParentDir) {
            return Err("path must not contain '..' components".to_owned());
        }
    }

    // Canonicalize the workspace root (must exist).
    let canonical_root = workspace_root
        .canonicalize()
        .map_err(|e| format!("cannot resolve workspace root '{}': {e}", workspace_root.display()))?;

    // Build the candidate path.
    let candidate = canonical_root.join(requested_path);

    // If the target already exists we can canonicalize directly.
    // Otherwise we canonicalize the longest existing prefix and append
    // the remaining components, then check containment.
    let resolved = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|e| format!("cannot resolve path '{}': {e}", candidate.display()))?
    } else {
        // Walk up to the nearest existing ancestor.
        let mut existing = candidate.as_path();
        let mut tail_parts: Vec<&std::ffi::OsStr> = Vec::new();
        loop {
            if existing.exists() {
                break;
            }
            match existing.parent() {
                Some(parent) => {
                    if let Some(file_name) = existing.file_name() {
                        tail_parts.push(file_name);
                    }
                    existing = parent;
                }
                None => break,
            }
        }
        let mut resolved = existing
            .canonicalize()
            .map_err(|e| format!("cannot resolve ancestor of '{}': {e}", candidate.display()))?;
        for part in tail_parts.into_iter().rev() {
            resolved.push(part);
        }
        resolved
    };

    // Containment check.
    if !resolved.starts_with(&canonical_root) {
        return Err(format!(
            "path '{}' resolves outside workspace root '{}'",
            requested,
            canonical_root.display()
        ));
    }

    Ok(resolved)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tool implementations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Read file contents with optional line offset and limit.
pub async fn file_read(workspace_root: &Path, req: FileReadRequest) -> Result<Value, String> {
    let path = validate_path(workspace_root, &req.path)?;

    let content = fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let offset = req.offset.unwrap_or(0);
    let limit = req.limit.unwrap_or(total_lines.saturating_sub(offset));

    let selected: Vec<&str> = lines
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect();

    let result_content = selected.join("\n");

    Ok(serde_json::json!({
        "path": req.path,
        "content": result_content,
        "total_lines": total_lines,
        "offset": offset,
        "lines_returned": selected.len(),
    }))
}

/// Write/create a file atomically (write to .tmp sibling, then rename).
pub async fn file_write(workspace_root: &Path, req: FileWriteRequest) -> Result<Value, String> {
    let path = validate_path(workspace_root, &req.path)?;

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create parent directory: {e}"))?;
    }

    // Atomic write: write to uniquely-named .tmp sibling, sync, then rename.
    let tmp_name = format!(
        ".{}.{}.tmp",
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        uuid::Uuid::new_v4().as_simple()
    );
    let tmp_path = path.with_file_name(tmp_name);

    let mut file = fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("failed to create temp file '{}': {e}", tmp_path.display()))?;

    file.write_all(req.content.as_bytes())
        .await
        .map_err(|e| format!("failed to write temp file: {e}"))?;

    file.flush()
        .await
        .map_err(|e| format!("failed to flush temp file: {e}"))?;

    file.sync_data()
        .await
        .map_err(|e| format!("failed to sync temp file: {e}"))?;

    // Rename into place.
    fs::rename(&tmp_path, &path)
        .await
        .map_err(|e| {
            // Best-effort cleanup of the temp file.
            let tmp = tmp_path.clone();
            tokio::spawn(async move { let _ = fs::remove_file(&tmp).await; });
            format!("failed to rename temp file into place: {e}")
        })?;

    let bytes_written = req.content.len();

    Ok(serde_json::json!({
        "path": req.path,
        "bytes_written": bytes_written,
        "success": true,
    }))
}

/// Append content to an existing file.
pub async fn file_append(workspace_root: &Path, req: FileAppendRequest) -> Result<Value, String> {
    let path = validate_path(workspace_root, &req.path)?;

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create parent directory: {e}"))?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| format!("failed to open '{}' for append: {e}", path.display()))?;

    file.write_all(req.content.as_bytes())
        .await
        .map_err(|e| format!("failed to append to '{}': {e}", path.display()))?;

    file.flush()
        .await
        .map_err(|e| format!("failed to flush '{}': {e}", path.display()))?;

    Ok(serde_json::json!({
        "path": req.path,
        "bytes_appended": req.content.len(),
        "success": true,
    }))
}

/// Move/rename a file or directory.
pub async fn file_move(workspace_root: &Path, req: FileMoveRequest) -> Result<Value, String> {
    let source = validate_path(workspace_root, &req.source)?;
    let destination = validate_path(workspace_root, &req.destination)?;

    if !source.exists() {
        return Err(format!("source '{}' does not exist", req.source));
    }

    // Ensure destination parent exists.
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create destination parent directory: {e}"))?;
    }

    fs::rename(&source, &destination)
        .await
        .map_err(|e| format!("failed to move '{}' to '{}': {e}", req.source, req.destination))?;

    Ok(serde_json::json!({
        "source": req.source,
        "destination": req.destination,
        "success": true,
    }))
}

/// Delete a file or empty directory.
pub async fn file_delete(workspace_root: &Path, req: FileDeleteRequest) -> Result<Value, String> {
    let path = validate_path(workspace_root, &req.path)?;

    let metadata = fs::metadata(&path)
        .await
        .map_err(|e| format!("failed to stat '{}': {e}", req.path))?;

    if metadata.is_dir() {
        fs::remove_dir(&path)
            .await
            .map_err(|e| format!("failed to remove directory '{}' (must be empty): {e}", req.path))?;
    } else {
        fs::remove_file(&path)
            .await
            .map_err(|e| format!("failed to remove file '{}': {e}", req.path))?;
    }

    Ok(serde_json::json!({
        "path": req.path,
        "success": true,
    }))
}

/// List directory contents with metadata.
pub async fn file_list(workspace_root: &Path, req: FileListRequest) -> Result<Value, String> {
    let path = validate_path(workspace_root, &req.path)?;

    let mut read_dir = fs::read_dir(&path)
        .await
        .map_err(|e| format!("failed to read directory '{}': {e}", req.path))?;

    let mut entries: Vec<DirEntry> = Vec::new();

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| format!("failed to read directory entry: {e}"))?
    {
        let metadata = entry
            .metadata()
            .await
            .map_err(|e| format!("failed to read metadata for '{}': {e}", entry.path().display()))?;

        let modified = metadata
            .modified()
            .ok()
            .map(|t| {
                let dt: DateTime<Utc> = t.into();
                dt.to_rfc3339()
            })
            .unwrap_or_default();

        let name = entry
            .file_name()
            .to_string_lossy()
            .to_string();

        entries.push(DirEntry {
            name,
            size: metadata.len(),
            modified,
            is_dir: metadata.is_dir(),
        });
    }

    // Sort by name for deterministic output.
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(serde_json::json!({
        "path": req.path,
        "entries": entries,
        "count": entries.len(),
    }))
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_workspace() -> TempDir {
        TempDir::new().expect("failed to create temp dir")
    }

    #[test]
    fn validate_path_rejects_parent_traversal() {
        let ws = tmp_workspace();
        let result = validate_path(ws.path(), "../etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains(".."));
    }

    #[test]
    fn validate_path_rejects_absolute_path() {
        let ws = tmp_workspace();
        // On Windows, "/etc/passwd" is not absolute; use a drive-letter path instead.
        let abs_path = if cfg!(windows) { "C:\\Windows\\System32" } else { "/etc/passwd" };
        let result = validate_path(ws.path(), abs_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("absolute paths are not allowed"));
    }

    #[test]
    fn validate_path_accepts_valid_path() {
        let ws = tmp_workspace();
        // Create the file first so canonicalization works.
        std::fs::write(ws.path().join("hello.txt"), "hi").unwrap();
        let result = validate_path(ws.path(), "hello.txt");
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with("hello.txt"));
    }

    #[test]
    fn validate_path_accepts_nested_new_file() {
        let ws = tmp_workspace();
        std::fs::create_dir_all(ws.path().join("subdir")).unwrap();
        let result = validate_path(ws.path(), "subdir/new_file.txt");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn file_write_and_read_roundtrip() {
        let ws = tmp_workspace();
        let content = "hello, world\nsecond line\n";

        file_write(
            ws.path(),
            FileWriteRequest {
                path: "test.txt".into(),
                content: content.into(),
            },
        )
        .await
        .expect("write failed");

        let result = file_read(
            ws.path(),
            FileReadRequest {
                path: "test.txt".into(),
                offset: None,
                limit: None,
            },
        )
        .await
        .expect("read failed");

        assert_eq!(result["content"].as_str().unwrap(), "hello, world\nsecond line");
        assert_eq!(result["total_lines"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn file_read_with_offset_and_limit() {
        let ws = tmp_workspace();
        let content = "line0\nline1\nline2\nline3\nline4\n";

        file_write(
            ws.path(),
            FileWriteRequest {
                path: "lines.txt".into(),
                content: content.into(),
            },
        )
        .await
        .unwrap();

        let result = file_read(
            ws.path(),
            FileReadRequest {
                path: "lines.txt".into(),
                offset: Some(1),
                limit: Some(2),
            },
        )
        .await
        .unwrap();

        assert_eq!(result["content"].as_str().unwrap(), "line1\nline2");
        assert_eq!(result["lines_returned"].as_u64().unwrap(), 2);
        assert_eq!(result["offset"].as_u64().unwrap(), 1);
    }

    #[tokio::test]
    async fn file_append_creates_and_appends() {
        let ws = tmp_workspace();

        file_append(
            ws.path(),
            FileAppendRequest {
                path: "log.txt".into(),
                content: "first\n".into(),
            },
        )
        .await
        .unwrap();

        file_append(
            ws.path(),
            FileAppendRequest {
                path: "log.txt".into(),
                content: "second\n".into(),
            },
        )
        .await
        .unwrap();

        let result = file_read(
            ws.path(),
            FileReadRequest {
                path: "log.txt".into(),
                offset: None,
                limit: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(result["content"].as_str().unwrap(), "first\nsecond");
    }

    #[tokio::test]
    async fn file_move_renames() {
        let ws = tmp_workspace();

        file_write(
            ws.path(),
            FileWriteRequest {
                path: "old.txt".into(),
                content: "data".into(),
            },
        )
        .await
        .unwrap();

        file_move(
            ws.path(),
            FileMoveRequest {
                source: "old.txt".into(),
                destination: "new.txt".into(),
            },
        )
        .await
        .unwrap();

        assert!(!ws.path().join("old.txt").exists());
        assert!(ws.path().join("new.txt").exists());
    }

    #[tokio::test]
    async fn file_delete_removes_file() {
        let ws = tmp_workspace();

        file_write(
            ws.path(),
            FileWriteRequest {
                path: "doomed.txt".into(),
                content: "bye".into(),
            },
        )
        .await
        .unwrap();

        file_delete(
            ws.path(),
            FileDeleteRequest {
                path: "doomed.txt".into(),
            },
        )
        .await
        .unwrap();

        assert!(!ws.path().join("doomed.txt").exists());
    }

    #[tokio::test]
    async fn file_delete_removes_empty_dir() {
        let ws = tmp_workspace();
        std::fs::create_dir(ws.path().join("empty_dir")).unwrap();

        file_delete(
            ws.path(),
            FileDeleteRequest {
                path: "empty_dir".into(),
            },
        )
        .await
        .unwrap();

        assert!(!ws.path().join("empty_dir").exists());
    }

    #[tokio::test]
    async fn file_list_returns_entries() {
        let ws = tmp_workspace();
        std::fs::write(ws.path().join("a.txt"), "aaa").unwrap();
        std::fs::write(ws.path().join("b.txt"), "bb").unwrap();
        std::fs::create_dir(ws.path().join("subdir")).unwrap();

        let result = file_list(
            ws.path(),
            FileListRequest {
                path: ".".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(result["count"].as_u64().unwrap(), 3);
        let entries = result["entries"].as_array().unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
        assert!(names.contains(&"subdir"));

        // Check that subdir is marked as a directory.
        let subdir_entry = entries.iter().find(|e| e["name"] == "subdir").unwrap();
        assert_eq!(subdir_entry["is_dir"].as_bool().unwrap(), true);
    }
}
