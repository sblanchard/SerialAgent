use std::ffi::OsStr;
use std::path::Path;

use glob::glob;

use crate::api::import_openclaw::MergeStrategy;
use super::OpenClawImportError;

pub(super) async fn copy_dir_strategy(
    src: &Path,
    dst: &Path,
    strategy: MergeStrategy,
) -> Result<(), OpenClawImportError> {
    if !src.exists() {
        return Ok(());
    }
    match strategy {
        MergeStrategy::Replace => {
            if dst.exists() {
                tokio::fs::remove_dir_all(dst).await?;
            }
            copy_dir_recursive(src, dst).await?;
        }
        MergeStrategy::MergeSafe => {
            copy_dir_recursive(src, dst).await?;
        }
        MergeStrategy::SkipExisting => {
            copy_dir_recursive_skip_existing(src, dst).await?;
        }
    }
    Ok(())
}

pub(super) async fn copy_glob_strategy(
    src_dir: &Path,
    dst_dir: &Path,
    patterns: &[&str],
    strategy: MergeStrategy,
) -> Result<u32, OpenClawImportError> {
    let mut copied = 0u32;
    for pat in patterns {
        let g = src_dir.join(pat).to_string_lossy().to_string();
        let Ok(paths) = glob(&g) else { continue };

        for m in paths {
            let src = match m {
                Ok(p) => p,
                Err(_) => continue,
            };
            if src.is_file() {
                let name = src.file_name().unwrap_or_else(|| OsStr::new("file"));
                let dst = dst_dir.join(name);
                copy_file_strategy(&src, &dst, strategy).await?;
                copied += 1;
            }
        }
    }
    Ok(copied)
}

pub(super) async fn copy_file_strategy(
    src: &Path,
    dst: &Path,
    strategy: MergeStrategy,
) -> Result<(), OpenClawImportError> {
    if !src.exists() {
        return Ok(());
    }
    if dst.exists() {
        match strategy {
            MergeStrategy::Replace => { /* overwrite */ }
            MergeStrategy::SkipExisting => return Ok(()),
            MergeStrategy::MergeSafe => { /* overwrite for deterministic behavior */ }
        }
    }
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(src, dst).await?;
    Ok(())
}

fn copy_dir_recursive<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), OpenClawImportError>> + Send + 'a>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(dst).await?;
        let mut rd = tokio::fs::read_dir(src).await?;
        while let Some(e) = rd.next_entry().await? {
            let ft = e.file_type().await?;
            let from = e.path();
            let to = dst.join(e.file_name());
            if ft.is_dir() {
                copy_dir_recursive(&from, &to).await?;
            } else if ft.is_file() {
                tokio::fs::copy(&from, &to).await?;
            }
            // Skip symlinks and other special files during copy
        }
        Ok(())
    })
}

fn copy_dir_recursive_skip_existing<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), OpenClawImportError>> + Send + 'a>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(dst).await?;
        let mut rd = tokio::fs::read_dir(src).await?;
        while let Some(e) = rd.next_entry().await? {
            let ft = e.file_type().await?;
            let from = e.path();
            let to = dst.join(e.file_name());
            if ft.is_dir() {
                copy_dir_recursive_skip_existing(&from, &to).await?;
            } else if ft.is_file() {
                if !to.exists() {
                    tokio::fs::copy(&from, &to).await?;
                }
            }
            // Skip symlinks and other special files during copy
        }
        Ok(())
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_skip_existing_does_not_overwrite() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        // Create source file
        let src_file = src.path().join("test.txt");
        std::fs::write(&src_file, "new content").unwrap();

        // Create existing destination file
        let dst_file = dst.path().join("test.txt");
        std::fs::write(&dst_file, "original content").unwrap();

        copy_file_strategy(&src_file, &dst_file, MergeStrategy::SkipExisting)
            .await
            .unwrap();

        // Should NOT have overwritten
        assert_eq!(
            std::fs::read_to_string(&dst_file).unwrap(),
            "original content"
        );
    }

    #[tokio::test]
    async fn test_replace_does_overwrite() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        let src_file = src.path().join("test.txt");
        std::fs::write(&src_file, "new content").unwrap();

        let dst_file = dst.path().join("test.txt");
        std::fs::write(&dst_file, "original content").unwrap();

        copy_file_strategy(&src_file, &dst_file, MergeStrategy::Replace)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(&dst_file).unwrap(),
            "new content"
        );
    }
}
