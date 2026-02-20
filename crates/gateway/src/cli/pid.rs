//! PID file management for daemon-style operation.
//!
//! On startup the server writes its PID to the configured path and acquires an
//! `fs2` exclusive lock on the file.  If another instance already holds the
//! lock, startup fails immediately.  The lock (and file) are released on
//! shutdown via [`remove_pid_file`].

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use fs2::FileExt;

/// Write the current process PID to `path` and acquire an exclusive lock.
///
/// Returns the open [`File`] handle — the caller **must** keep it alive for
/// the lifetime of the server so the advisory lock is held.
///
/// # Errors
///
/// * Another process already holds the lock (stale or running).
/// * Filesystem I/O failure.
pub fn write_pid_file(path: &Path) -> anyhow::Result<File> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .read(true)
        .open(path)
        .map_err(|e| anyhow::anyhow!("opening PID file {}: {e}", path.display()))?;

    file.try_lock_exclusive().map_err(|_| {
        anyhow::anyhow!(
            "another SerialAgent instance is running (PID file {} is locked)",
            path.display()
        )
    })?;

    let pid = std::process::id();
    // Re-open for write after lock (file was opened read+write, just write content).
    {
        let mut f = &file;
        writeln!(f, "{pid}")?;
        f.flush()?;
    }

    tracing::info!(path = %path.display(), pid, "PID file written");
    Ok(file)
}

/// Remove the PID file at `path`.  The exclusive lock is released when the
/// `_handle` is dropped (happens automatically, but calling this makes the
/// cleanup explicit and removes the stale file from disk).
pub fn remove_pid_file(path: &Path, _handle: File) {
    if let Err(e) = fs::remove_file(path) {
        tracing::warn!(path = %path.display(), error = %e, "failed to remove PID file");
    } else {
        tracing::info!(path = %path.display(), "PID file removed");
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_remove_pid_file() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("test.pid");

        let handle = write_pid_file(&pid_path).unwrap();

        // File exists and contains our PID.
        let content = fs::read_to_string(&pid_path).unwrap();
        let stored_pid: u32 = content.trim().parse().unwrap();
        assert_eq!(stored_pid, std::process::id());

        // A second lock attempt should fail.
        let second = write_pid_file(&pid_path);
        assert!(second.is_err(), "expected lock conflict");

        // Cleanup.
        remove_pid_file(&pid_path, handle);
        assert!(!pid_path.exists());
    }

    #[test]
    fn creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("nested").join("dirs").join("sa.pid");

        let handle = write_pid_file(&pid_path).unwrap();
        assert!(pid_path.exists());

        remove_pid_file(&pid_path, handle);
    }
}
