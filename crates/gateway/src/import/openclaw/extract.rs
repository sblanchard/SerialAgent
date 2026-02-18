//! Hardened tar extraction + validation for OpenClaw imports.
//!
//! All tar paths pass through [`normalize_tar_path()`] which is the **single source
//! of truth** for both the dedup key (validation) and the filesystem target (extraction).

use std::io;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;

use super::OpenClawImportError;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Max path depth to prevent zip-bomb-style deeply nested directories.
const MAX_PATH_DEPTH: usize = 64;

/// Max total tar entries (including metadata like PAX headers) to prevent
/// entry-count DoS even without materializing files.
const MAX_ENTRIES_TOTAL: u64 = 100_000;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Limits (configurable via env, sensible defaults)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Max total extracted size in bytes (default 500MB).
fn max_extracted_bytes() -> u64 {
    std::env::var("SA_IMPORT_MAX_EXTRACTED_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500 * 1024 * 1024)
}

/// Max number of files in archive (default 50_000).
fn max_file_count() -> u64 {
    std::env::var("SA_IMPORT_MAX_FILE_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50_000)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Safe extraction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub(super) async fn safe_extract_tgz(
    tgz_path: &Path,
    dest_dir: &Path,
) -> Result<(), OpenClawImportError> {
    // Phase 1: Stream validation — check all entries before extracting.
    // This catches path traversal, symlinks, duplicates, size limits, etc.
    validate_tgz_entries(tgz_path)?;

    // Phase 2: Manual extraction with hardened file creation.
    // We do NOT use `unpack_in()` — instead we control every file open.
    let file = std::fs::File::open(tgz_path)?;
    let gz = GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = Archive::new(gz);

    for entry in archive.entries().map_err(|e| {
        OpenClawImportError::ArchiveInvalid(format!("tar entries failed: {e}"))
    })? {
        let mut entry = entry.map_err(|e| {
            OpenClawImportError::ArchiveInvalid(format!("tar entry read failed: {e}"))
        })?;

        let entry_type = entry.header().entry_type();

        // Skip metadata-only entries (PAX headers, GNU longname)
        match entry_type {
            tar::EntryType::XHeader
            | tar::EntryType::XGlobalHeader
            | tar::EntryType::GNULongName
            | tar::EntryType::GNULongLink => continue,
            tar::EntryType::Regular
            | tar::EntryType::GNUSparse
            | tar::EntryType::Directory => {}
            _ => {
                // Already validated in phase 1, but defense-in-depth
                let path = entry.path().unwrap_or_default();
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "unexpected entry type {:?} at: {}",
                    entry_type,
                    path.display()
                )));
            }
        }

        let raw_path = entry
            .path()
            .map_err(|e| {
                OpenClawImportError::ArchiveInvalid(format!("tar path read failed: {e}"))
            })?
            .into_owned();

        // Defense-in-depth: re-validate path even though phase 1 already did
        validate_relative_path(&raw_path)?;

        // Use the same normalized path as validation — ensures the filesystem path
        // matches the dedup key (a/./b → a/b, a//b → a/b, etc.)
        let (_, normalized_path) = normalize_tar_path(&raw_path)?;
        let full_path = dest_dir.join(&normalized_path);

        match entry_type {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&full_path)?;
                // Safe permissions: rwxr-xr-x, no setuid/setgid/sticky
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        &full_path,
                        std::fs::Permissions::from_mode(0o755),
                    )?;
                }
            }
            _ => {
                // Regular file (or GNUSparse)
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                // create_new(true): never overwrite, never follow pre-existing symlinks.
                // This prevents tar tricks with repeated paths and TOCTOU races.
                let mut out_file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&full_path)
                    .map_err(|e| {
                        if e.kind() == io::ErrorKind::AlreadyExists {
                            OpenClawImportError::ArchiveInvalid(format!(
                                "file collision (duplicate or pre-existing): {}",
                                normalized_path.display()
                            ))
                        } else {
                            OpenClawImportError::Io(e)
                        }
                    })?;

                std::io::copy(&mut entry, &mut out_file)?;

                // Safe permissions: strip setuid(04000)/setgid(02000)/sticky(01000)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = entry.header().mode().unwrap_or(0o644) & 0o777;
                    std::fs::set_permissions(
                        &full_path,
                        std::fs::Permissions::from_mode(mode),
                    )?;
                }
            }
        }
    }

    Ok(())
}

/// Validate tar entries without extracting: check paths, types, cumulative sizes,
/// and duplicate file paths. Uses streaming (BufReader) — NOT tokio::fs::read.
fn validate_tgz_entries(tgz_path: &Path) -> Result<(), OpenClawImportError> {
    let file = std::fs::File::open(tgz_path)?;
    let gz = GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = Archive::new(gz);

    let max_bytes = max_extracted_bytes();
    let max_files = max_file_count();
    let mut total_bytes: u64 = 0;
    let mut total_files: u64 = 0;
    let mut total_entries: u64 = 0;
    let mut seen_file_paths = std::collections::HashSet::new();

    for entry in archive.entries().map_err(|e| {
        OpenClawImportError::ArchiveInvalid(format!("tar entries failed: {e}"))
    })? {
        let entry = entry.map_err(|e| {
            OpenClawImportError::ArchiveInvalid(format!("tar entry read failed: {e}"))
        })?;

        // ── Global entry counter (caps total tar records, including metadata) ──
        total_entries += 1;
        if total_entries > MAX_ENTRIES_TOTAL {
            return Err(OpenClawImportError::SizeLimitExceeded(format!(
                "archive contains more than {} total entries (including metadata)",
                MAX_ENTRIES_TOTAL
            )));
        }

        // ── Type check ──
        let entry_type = entry.header().entry_type();
        match entry_type {
            // PAX / GNU longname metadata: normally consumed transparently by the
            // tar crate, but handle defensively. Count bytes toward the limit
            // (PAX records can be arbitrarily large → decompression DoS).
            tar::EntryType::XHeader
            | tar::EntryType::XGlobalHeader
            | tar::EntryType::GNULongName
            | tar::EntryType::GNULongLink => {
                let meta_size = entry.header().size().unwrap_or(0);
                total_bytes += meta_size;
                if total_bytes > max_bytes {
                    return Err(OpenClawImportError::SizeLimitExceeded(format!(
                        "archive metadata exceeds extracted-bytes limit of {} bytes \
                         (at {} bytes after {} entries)",
                        max_bytes, total_bytes, total_entries
                    )));
                }
                continue;
            }
            // Allowed content types
            tar::EntryType::Regular
            | tar::EntryType::GNUSparse
            | tar::EntryType::Directory => {}
            // Reject everything else
            tar::EntryType::Symlink | tar::EntryType::Link => {
                let path = entry.path().unwrap_or_default();
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "symlink/hardlink in archive: {}",
                    path.display()
                )));
            }
            other => {
                let path = entry.path().unwrap_or_default();
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "unsupported entry type {:?}: {}",
                    other,
                    path.display()
                )));
            }
        }

        // ── Path check: no traversal, no empty, depth limit, no non-UTF8 ──
        let path = entry.path().map_err(|e| {
            OpenClawImportError::ArchiveInvalid(format!("tar path read failed: {e}"))
        })?;
        validate_relative_path(&path)?;

        // ── Normalize path and check for collisions ──
        let (normalized_key, _) = normalize_tar_path(&path)?;

        // Duplicate file detection (dirs may repeat, that's OK)
        if !matches!(entry_type, tar::EntryType::Directory) {
            if !seen_file_paths.insert(normalized_key.clone()) {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "duplicate file path in archive (after normalization): {}",
                    path.display()
                )));
            }
        }

        // ── Size limits ──
        let entry_size = entry.header().size().unwrap_or(0);
        total_bytes += entry_size;
        total_files += 1;

        if total_bytes > max_bytes {
            return Err(OpenClawImportError::SizeLimitExceeded(format!(
                "extracted content exceeds limit of {} bytes (at {} bytes after {} files)",
                max_bytes, total_bytes, total_files
            )));
        }
        if total_files > max_files {
            return Err(OpenClawImportError::SizeLimitExceeded(format!(
                "archive contains more than {} files",
                max_files
            )));
        }
    }
    Ok(())
}

/// Normalize a tar path to a canonical form for dedup and filesystem use.
///
/// This is the **single source of truth** for path normalization. Both validation
/// (dedup key) and extraction (filesystem target) must use this function so the
/// model matches.
///
/// Invariants enforced:
/// - Rejects non-UTF8 paths and components (encoding bypass prevention)
/// - Rejects `..` (ParentDir), absolute (`/`, RootDir), and platform prefixes (`C:\`)
/// - Strips `.` (CurDir) components and collapses repeated separators
/// - Rejects empty Normal components (e.g. from pathological inputs)
/// - Rejects paths that normalize to empty
/// - Returns `(String key, PathBuf fs_path)` — both always identical in meaning
fn normalize_tar_path(path: &Path) -> Result<(String, PathBuf), OpenClawImportError> {
    // Reject non-UTF8 paths explicitly
    let raw = path.to_str().ok_or_else(|| {
        OpenClawImportError::ArchiveInvalid(format!(
            "non-UTF8 path in archive: {}",
            path.display()
        ))
    })?;

    // Rebuild from components: this strips `.`, collapses `//`, and normalizes.
    // Dangerous components are hard-rejected here (not just in validate_relative_path)
    // so this function is safe to call standalone.
    let mut parts = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => {
                let s_str = s.to_str().ok_or_else(|| {
                    OpenClawImportError::ArchiveInvalid(format!(
                        "non-UTF8 component in archive path: {}",
                        raw
                    ))
                })?;
                // Reject empty normal components (shouldn't happen, but be explicit)
                if s_str.is_empty() {
                    return Err(OpenClawImportError::ArchiveInvalid(format!(
                        "empty component in archive path: {}",
                        raw
                    )));
                }
                parts.push(s_str);
            }
            Component::CurDir => {} // strip "."
            Component::ParentDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "parent dir traversal in path: {}",
                    raw
                )));
            }
            Component::RootDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "absolute path (root dir): {}",
                    raw
                )));
            }
            Component::Prefix(_) => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "platform prefix in path: {}",
                    raw
                )));
            }
        }
    }

    // Reject paths that normalize to empty (e.g. "." or "./")
    if parts.is_empty() {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "path normalizes to empty: {}",
            raw
        )));
    }

    let normalized: PathBuf = parts.iter().collect();
    let key = normalized.to_string_lossy().to_string();
    Ok((key, normalized))
}

fn validate_relative_path(path: &Path) -> Result<(), OpenClawImportError> {
    // Reject empty paths
    if path.as_os_str().is_empty() {
        return Err(OpenClawImportError::ArchiveInvalid(
            "empty path in archive".to_string(),
        ));
    }
    if path.is_absolute() {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "absolute path in archive: {}",
            path.display()
        )));
    }
    let mut depth = 0usize;
    for comp in path.components() {
        match comp {
            Component::Normal(_) => {
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "parent dir traversal in archive: {}",
                    path.display()
                )));
            }
            Component::Prefix(_) => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "platform prefix in archive path: {}",
                    path.display()
                )));
            }
            Component::RootDir => {
                return Err(OpenClawImportError::ArchiveInvalid(format!(
                    "root dir in archive path: {}",
                    path.display()
                )));
            }
        }
    }
    // Reject paths like "." or "./" that have no real components
    if depth == 0 {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "path resolves to empty: {}",
            path.display()
        )));
    }
    if depth > MAX_PATH_DEPTH {
        return Err(OpenClawImportError::ArchiveInvalid(format!(
            "path depth {} exceeds limit of {MAX_PATH_DEPTH}: {}",
            depth,
            path.display()
        )));
    }
    Ok(())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ─────────────────────────────────────────────

    fn create_test_tgz(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append_data(&mut header, path, &data[..]).unwrap();
        }

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();
        tmp
    }

    /// Create a test .tgz with path-traversal entries by writing raw tar bytes.
    /// The tar crate blocks `..` in both `append_data` and `set_path`, so we
    /// construct the malicious archive at the byte level.
    fn create_test_tgz_with_traversal(
        entries: &[(&str, &[u8])],
    ) -> tempfile::NamedTempFile {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut out = std::io::BufWriter::new(gz);

        for (path, data) in entries {
            // Build a raw 512-byte POSIX/GNU tar header
            let mut header_bytes = [0u8; 512];

            // Name field: offset 0, 100 bytes
            let name_bytes = path.as_bytes();
            let name_len = name_bytes.len().min(100);
            header_bytes[..name_len].copy_from_slice(&name_bytes[..name_len]);

            // Mode: offset 100, 8 bytes ("0000644\0")
            header_bytes[100..108].copy_from_slice(b"0000644\0");

            // UID: offset 108, 8 bytes
            header_bytes[108..116].copy_from_slice(b"0001000\0");

            // GID: offset 116, 8 bytes
            header_bytes[116..124].copy_from_slice(b"0001000\0");

            // Size: offset 124, 12 bytes (octal, zero-terminated)
            let size_str = format!("{:011o}\0", data.len());
            header_bytes[124..136].copy_from_slice(size_str.as_bytes());

            // Mtime: offset 136, 12 bytes
            header_bytes[136..148].copy_from_slice(b"00000000000\0");

            // Typeflag: offset 156, 1 byte ('0' = regular file)
            header_bytes[156] = b'0';

            // Magic: offset 257, 6 bytes ("ustar\0")
            header_bytes[257..263].copy_from_slice(b"ustar\0");

            // Version: offset 263, 2 bytes ("00")
            header_bytes[263..265].copy_from_slice(b"00");

            // Checksum: offset 148, 8 bytes — compute over header with
            // checksum field treated as spaces
            header_bytes[148..156].copy_from_slice(b"        ");
            let cksum: u32 = header_bytes.iter().map(|&b| b as u32).sum();
            let cksum_str = format!("{:06o}\0 ", cksum);
            header_bytes[148..156].copy_from_slice(&cksum_str.as_bytes()[..8]);

            out.write_all(&header_bytes).unwrap();
            out.write_all(data).unwrap();

            // Pad to 512-byte boundary
            let remainder = data.len() % 512;
            if remainder != 0 {
                let padding = 512 - remainder;
                out.write_all(&vec![0u8; padding]).unwrap();
            }
        }

        // Two 512-byte zero blocks mark end-of-archive
        out.write_all(&[0u8; 1024]).unwrap();
        let gz = out.into_inner().unwrap();
        gz.finish().unwrap();
        tmp
    }

    // ── Path validation ─────────────────────────────────────────

    #[test]
    fn test_relative_path_ok() {
        assert!(validate_relative_path(Path::new("agents/main/sessions/foo.jsonl")).is_ok());
        assert!(validate_relative_path(Path::new("workspace/MEMORY.md")).is_ok());
        assert!(validate_relative_path(Path::new("workspace-kimi/file.txt")).is_ok());
    }

    #[test]
    fn test_relative_path_traversal_rejected() {
        assert!(validate_relative_path(Path::new("../../../etc/passwd")).is_err());
        assert!(validate_relative_path(Path::new("agents/../../../etc/shadow")).is_err());
        assert!(validate_relative_path(Path::new("agents/main/../../..")).is_err());
    }

    #[test]
    fn test_absolute_path_rejected() {
        assert!(validate_relative_path(Path::new("/etc/passwd")).is_err());
        assert!(validate_relative_path(Path::new("/tmp/evil")).is_err());
    }

    #[test]
    fn test_empty_path_rejected() {
        assert!(validate_relative_path(Path::new("")).is_err());
    }

    #[test]
    fn test_curdir_only_rejected() {
        // "." and "./" resolve to zero Normal components → rejected
        assert!(validate_relative_path(Path::new(".")).is_err());
        assert!(validate_relative_path(Path::new("./")).is_err());
    }

    #[test]
    fn test_deep_nesting_rejected() {
        let deep = (0..MAX_PATH_DEPTH + 1)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/");
        assert!(validate_relative_path(Path::new(&deep)).is_err());

        // Just at the limit should be OK
        let at_limit = (0..MAX_PATH_DEPTH)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/");
        assert!(validate_relative_path(Path::new(&at_limit)).is_ok());
    }

    // ── Tar entry validation with real archives ─────────────────

    #[test]
    fn test_validate_clean_archive() {
        let tgz = create_test_tgz(&[
            ("workspace/MEMORY.md", b"# Memory"),
            ("agents/main/sessions/s1.jsonl", b"{}"),
        ]);
        assert!(validate_tgz_entries(tgz.path()).is_ok());
    }

    #[test]
    fn test_validate_archive_with_traversal() {
        let tgz = create_test_tgz_with_traversal(&[("../../../etc/passwd", b"root:x:0:0")]);
        assert!(validate_tgz_entries(tgz.path()).is_err());
    }

    #[test]
    fn test_validate_archive_size_limit() {
        // Create archive with 2 small files — should pass
        let tgz = create_test_tgz(&[("a", b"x"), ("b", b"y")]);
        assert!(validate_tgz_entries(tgz.path()).is_ok());
    }

    #[test]
    fn test_validate_archive_absolute_path() {
        // Create archive with absolute path via raw bytes
        let tgz = create_test_tgz_with_traversal(&[("/tmp/evil", b"pwned")]);
        let result = validate_tgz_entries(tgz.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("absolute") || err.contains("root dir"),
            "should reject absolute path: {err}"
        );
    }

    #[test]
    fn test_validate_archive_duplicate_file_paths() {
        // The tar crate's Builder doesn't check for duplicates,
        // so we can create a valid tgz with the same file path twice.
        let tgz = create_test_tgz(&[
            ("agents/main/sessions/s1.jsonl", b"first"),
            ("agents/main/sessions/s1.jsonl", b"second"),
        ]);
        let result = validate_tgz_entries(tgz.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("duplicate"),
            "should reject duplicate file path: {err}"
        );
    }

    #[test]
    fn test_validate_archive_deep_nesting() {
        let deep = (0..MAX_PATH_DEPTH + 1)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/")
            + "/file.txt";
        let tgz = create_test_tgz(&[(&deep, b"deep")]);
        let result = validate_tgz_entries(tgz.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("depth"),
            "should reject deep nesting: {err}"
        );
    }

    #[test]
    fn test_validate_archive_normalization_collision() {
        // "a/b" and "a/./b" should normalize to the same key → duplicate detected.
        // The tar crate's Builder strips "." from paths, so we use the raw
        // byte-level builder to craft the a/./b entry.
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut out = std::io::BufWriter::new(gz);

        // Helper to write a raw tar entry
        let write_raw_entry = |out: &mut std::io::BufWriter<GzEncoder<&std::fs::File>>,
                                path: &str,
                                data: &[u8]| {
            let mut hdr = [0u8; 512];
            let name_bytes = path.as_bytes();
            let name_len = name_bytes.len().min(100);
            hdr[..name_len].copy_from_slice(&name_bytes[..name_len]);
            hdr[100..108].copy_from_slice(b"0000644\0");
            hdr[108..116].copy_from_slice(b"0001000\0");
            hdr[116..124].copy_from_slice(b"0001000\0");
            let size_str = format!("{:011o}\0", data.len());
            hdr[124..136].copy_from_slice(size_str.as_bytes());
            hdr[136..148].copy_from_slice(b"00000000000\0");
            hdr[156] = b'0';
            hdr[257..263].copy_from_slice(b"ustar\0");
            hdr[263..265].copy_from_slice(b"00");
            hdr[148..156].copy_from_slice(b"        ");
            let cksum: u32 = hdr.iter().map(|&b| b as u32).sum();
            let cksum_str = format!("{:06o}\0 ", cksum);
            hdr[148..156].copy_from_slice(&cksum_str.as_bytes()[..8]);
            out.write_all(&hdr).unwrap();
            out.write_all(data).unwrap();
            let rem = data.len() % 512;
            if rem != 0 {
                out.write_all(&vec![0u8; 512 - rem]).unwrap();
            }
        };

        write_raw_entry(&mut out, "agents/main/s.jsonl", b"first");
        write_raw_entry(&mut out, "agents/./main/s.jsonl", b"second");
        out.write_all(&[0u8; 1024]).unwrap();
        let gz = out.into_inner().unwrap();
        gz.finish().unwrap();

        let result = validate_tgz_entries(tmp.path());
        assert!(result.is_err(), "should detect normalization collision");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("duplicate"),
            "should report as duplicate: {err}"
        );
    }

    #[test]
    fn test_validate_archive_rejects_symlink() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        // Add a symlink entry: agents/evil -> /etc
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder
            .append_link(&mut header, "agents/evil", "/etc")
            .unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();

        let result = validate_tgz_entries(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("symlink") || err.contains("hardlink"),
            "error should mention symlink: {err}"
        );
    }

    // ── Normalize tar path ──────────────────────────────────────

    #[test]
    fn test_normalize_tar_path_strips_curdir() {
        let (key, pb) = normalize_tar_path(Path::new("a/./b/./c")).unwrap();
        assert_eq!(key, "a/b/c");
        assert_eq!(pb, PathBuf::from("a/b/c"));
    }

    #[test]
    fn test_normalize_tar_path_rejects_parent_dir() {
        // normalize_tar_path must reject .. independently of validate_relative_path
        assert!(normalize_tar_path(Path::new("a/../b")).is_err());
        assert!(normalize_tar_path(Path::new("../x")).is_err());
    }

    #[test]
    fn test_normalize_tar_path_rejects_empty_result() {
        assert!(normalize_tar_path(Path::new(".")).is_err());
        assert!(normalize_tar_path(Path::new("./")).is_err());
    }

    // ── Safe extract ────────────────────────────────────────────

    #[tokio::test]
    async fn test_safe_extract_clean_archive() {
        let tgz = create_test_tgz(&[
            ("workspace/MEMORY.md", b"# Memory file"),
            ("agents/main/sessions/s1.jsonl", b"{\"role\":\"user\"}"),
        ]);

        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tgz.path(), dest.path()).await;
        assert!(result.is_ok(), "extract should succeed: {:?}", result);

        // Verify files exist
        assert!(dest.path().join("workspace/MEMORY.md").exists());
        assert!(dest.path().join("agents/main/sessions/s1.jsonl").exists());
    }

    #[tokio::test]
    async fn test_safe_extract_rejects_traversal() {
        let tgz = create_test_tgz_with_traversal(&[("../../../etc/shadow", b"bad")]);
        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tgz.path(), dest.path()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_safe_extract_create_new_prevents_overwrite() {
        let tgz = create_test_tgz(&[("workspace/MEMORY.md", b"# Memory file")]);
        let dest = tempfile::tempdir().unwrap();

        // First extraction should succeed
        let r1 = safe_extract_tgz(tgz.path(), dest.path()).await;
        assert!(r1.is_ok(), "first extract should succeed: {:?}", r1);

        // Second extraction into same dir should fail due to create_new(true)
        let r2 = safe_extract_tgz(tgz.path(), dest.path()).await;
        assert!(r2.is_err(), "second extract should fail (file collision)");
        let err = r2.unwrap_err().to_string();
        assert!(
            err.contains("collision") || err.contains("AlreadyExists") || err.contains("duplicate"),
            "should report file collision: {err}"
        );
    }

    #[tokio::test]
    async fn test_safe_extract_permission_masking() {
        // Create archive with setuid bit in header
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        let data = b"#!/bin/sh\necho pwned";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o4755); // setuid!
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder
            .append_data(&mut header, "workspace/evil.sh", &data[..])
            .unwrap();
        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();

        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tmp.path(), dest.path()).await;
        assert!(result.is_ok(), "extract should succeed: {:?}", result);

        // Verify setuid bit was stripped
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(dest.path().join("workspace/evil.sh")).unwrap();
            let mode = meta.permissions().mode();
            assert_eq!(
                mode & 0o7777,
                0o755,
                "setuid bit should be stripped, got {:o}",
                mode & 0o7777
            );
        }
    }

    #[tokio::test]
    async fn test_safe_extract_dir_then_file_collision() {
        // Archive has a dir entry "workspace" then a file entry "workspace"
        // Extraction should fail because you can't create_new a file where a dir exists.
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        // Add directory entry
        let mut dir_hdr = tar::Header::new_gnu();
        dir_hdr.set_entry_type(tar::EntryType::Directory);
        dir_hdr.set_size(0);
        dir_hdr.set_mode(0o755);
        dir_hdr.set_cksum();
        builder
            .append_data(&mut dir_hdr, "workspace", &[] as &[u8])
            .unwrap();

        // Add file entry with same name
        let data = b"conflict";
        let mut file_hdr = tar::Header::new_gnu();
        file_hdr.set_entry_type(tar::EntryType::Regular);
        file_hdr.set_size(data.len() as u64);
        file_hdr.set_mode(0o644);
        file_hdr.set_cksum();
        builder
            .append_data(&mut file_hdr, "workspace", &data[..])
            .unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();

        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tmp.path(), dest.path()).await;
        // Should fail: can't create a file where a directory exists
        assert!(result.is_err(), "dir-then-file collision should fail: {:?}", result);
    }

    #[tokio::test]
    async fn test_safe_extract_file_then_dir_collision() {
        // Archive has a file entry "agents" then a dir entry "agents"
        // Extraction should fail because the file already occupies the path.
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = GzEncoder::new(tmp.as_file(), Compression::fast());
        let mut builder = tar::Builder::new(gz);

        // Add file entry
        let data = b"I'm a file, not a dir";
        let mut file_hdr = tar::Header::new_gnu();
        file_hdr.set_entry_type(tar::EntryType::Regular);
        file_hdr.set_size(data.len() as u64);
        file_hdr.set_mode(0o644);
        file_hdr.set_cksum();
        builder
            .append_data(&mut file_hdr, "agents", &data[..])
            .unwrap();

        // Add directory entry with same name
        let mut dir_hdr = tar::Header::new_gnu();
        dir_hdr.set_entry_type(tar::EntryType::Directory);
        dir_hdr.set_size(0);
        dir_hdr.set_mode(0o755);
        dir_hdr.set_cksum();
        builder
            .append_data(&mut dir_hdr, "agents", &[] as &[u8])
            .unwrap();

        let gz = builder.into_inner().unwrap();
        gz.finish().unwrap();

        let dest = tempfile::tempdir().unwrap();
        let result = safe_extract_tgz(tmp.path(), dest.path()).await;
        // Should fail: create_dir_all on a path that's already a file
        assert!(result.is_err(), "file-then-dir collision should fail: {:?}", result);
    }
}
