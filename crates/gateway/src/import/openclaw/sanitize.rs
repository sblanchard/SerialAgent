//! Identifier sanitization for safe filesystem use.
//!
//! Shared across the codebase: used by both the OpenClaw import pipeline
//! and admin API endpoints for validating agent IDs, workspace names, etc.

use super::OpenClawImportError;

/// Max length for identifiers (agent IDs, workspace names).
pub(crate) const MAX_IDENT_LEN: usize = 128;

/// Validate an identifier (agent ID, workspace folder name) for safe filesystem use.
/// Allows [a-zA-Z0-9._-] only, rejects empty / "." / ".." and caps length.
pub(crate) fn sanitize_ident(name: &str) -> Result<(), OpenClawImportError> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(OpenClawImportError::InvalidPath(format!(
            "invalid identifier: {name:?}"
        )));
    }
    if name.len() > MAX_IDENT_LEN {
        return Err(OpenClawImportError::InvalidPath(format!(
            "identifier too long ({} > {MAX_IDENT_LEN}): {name:?}",
            name.len()
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(OpenClawImportError::InvalidPath(format!(
            "identifier contains invalid characters: {name:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_ident_valid() {
        assert!(sanitize_ident("main").is_ok());
        assert!(sanitize_ident("kimi-agent").is_ok());
        assert!(sanitize_ident("workspace-claude").is_ok());
        assert!(sanitize_ident("agent_v2.1").is_ok());
    }

    #[test]
    fn test_sanitize_ident_rejects_traversal() {
        assert!(sanitize_ident("..").is_err());
        assert!(sanitize_ident(".").is_err());
        assert!(sanitize_ident("").is_err());
    }

    #[test]
    fn test_sanitize_ident_rejects_slashes() {
        assert!(sanitize_ident("foo/bar").is_err());
        assert!(sanitize_ident("../etc").is_err());
        assert!(sanitize_ident("foo\\bar").is_err());
    }

    #[test]
    fn test_sanitize_ident_rejects_special_chars() {
        assert!(sanitize_ident("foo bar").is_err());
        assert!(sanitize_ident("foo\0bar").is_err());
        assert!(sanitize_ident("agent;rm -rf").is_err());
    }

    #[test]
    fn test_sanitize_ident_length_limit() {
        let long = "a".repeat(MAX_IDENT_LEN);
        assert!(sanitize_ident(&long).is_ok());
        let too_long = "a".repeat(MAX_IDENT_LEN + 1);
        assert!(sanitize_ident(&too_long).is_err());
    }
}
