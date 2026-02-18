use std::io;
use std::path::Path;

use uuid::Uuid;

/// Information about a single staging entry.
#[derive(Debug, serde::Serialize)]
pub struct StagingEntry {
    pub id: String,
    pub created_at: String,
    pub age_secs: u64,
    pub size_bytes: u64,
    pub has_extracted: bool,
}

/// Delete staging dirs older than `max_age` seconds.
/// Call this from a periodic background task.
pub async fn cleanup_stale_staging(
    staging_root: &Path,
    max_age_secs: u64,
) -> Result<u32, io::Error> {
    let openclaw_root = staging_root.join("openclaw");
    if !openclaw_root.exists() {
        return Ok(0);
    }

    let now = std::time::SystemTime::now();
    let mut removed = 0u32;

    let mut rd = tokio::fs::read_dir(&openclaw_root).await?;
    while let Some(entry) = rd.next_entry().await? {
        let ft = entry.file_type().await?;
        if !ft.is_dir() {
            continue;
        }

        let meta = entry.metadata().await?;
        let created = meta
            .created()
            .or_else(|_| meta.modified())
            .unwrap_or(now);

        if let Ok(age) = now.duration_since(created) {
            if age.as_secs() > max_age_secs {
                let _ = tokio::fs::remove_dir_all(entry.path()).await;
                removed += 1;
            }
        }
    }

    Ok(removed)
}

/// Delete a specific staging dir by ID.
pub async fn delete_staging(
    staging_root: &Path,
    staging_id: &Uuid,
) -> Result<bool, io::Error> {
    let dir = staging_root.join("openclaw").join(staging_id.to_string());
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// List all staging entries under `staging_root/openclaw/`.
pub async fn list_staging(staging_root: &Path) -> Result<Vec<StagingEntry>, io::Error> {
    let openclaw_root = staging_root.join("openclaw");
    if !openclaw_root.exists() {
        return Ok(Vec::new());
    }

    let now = std::time::SystemTime::now();
    let mut entries = Vec::new();

    let mut rd = tokio::fs::read_dir(&openclaw_root).await?;
    while let Some(entry) = rd.next_entry().await? {
        let ft = entry.file_type().await?;
        if !ft.is_dir() {
            continue;
        }

        // Only list UUID-named directories
        let name = entry.file_name().to_string_lossy().to_string();
        if Uuid::parse_str(&name).is_err() {
            continue;
        }

        let meta = entry.metadata().await?;
        let created = meta.created().or_else(|_| meta.modified()).unwrap_or(now);
        let age_secs = now
            .duration_since(created)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let created_at = created
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Approximate size by scanning immediate children
        let mut size_bytes: u64 = 0;
        let dir_path = entry.path();
        let has_extracted = dir_path.join("extracted").exists();

        if let Ok(mut sub) = tokio::fs::read_dir(&dir_path).await {
            while let Some(sub_entry) = sub.next_entry().await.ok().flatten() {
                if let Ok(sub_meta) = sub_entry.metadata().await {
                    if sub_meta.is_file() {
                        size_bytes += sub_meta.len();
                    }
                }
            }
        }
        // Also check raw/openclaw-export.tgz for more accurate size
        let tgz = dir_path.join("raw").join("openclaw-export.tgz");
        if let Ok(tgz_meta) = tokio::fs::metadata(&tgz).await {
            size_bytes = size_bytes.max(tgz_meta.len());
        }

        entries.push(StagingEntry {
            id: name,
            created_at: created_at.to_string(),
            age_secs,
            size_bytes,
            has_extracted,
        });
    }

    // Sort newest first
    entries.sort_by(|a, b| b.age_secs.cmp(&a.age_secs).reverse());
    Ok(entries)
}
