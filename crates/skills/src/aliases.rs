//! Tool namespace alias map for OpenClaw / ClawHub compatibility.
//!
//! Different agent frameworks use different names for similar capabilities.
//! This module provides a bidirectional alias map so that skill manifests
//! authored with OpenClaw-style tool names work seamlessly in SerialAgent.
//!
//! Semantic rules:
//!   - Aliases must be semantically equivalent to their canonical form.
//!   - `web.fetch` (HTTP GET) ≠ `web.search` (search engine results).
//!     These are separate canonicals — never alias one to the other.
//!   - Canonicals that don't exist yet locally resolve to `Unknown` at
//!     dispatch time, which is correct: a node may provide them later.

use std::collections::HashMap;

/// Bidirectional tool name alias map.
pub struct ToolAliasMap {
    /// canonical → [aliases]
    to_aliases: HashMap<String, Vec<String>>,
    /// alias → canonical
    to_canonical: HashMap<String, String>,
}

impl ToolAliasMap {
    /// Create a new alias map from pairs of `(canonical, alias)`.
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let mut to_aliases: HashMap<String, Vec<String>> = HashMap::new();
        let mut to_canonical: HashMap<String, String> = HashMap::new();

        for (canonical, alias) in pairs {
            to_aliases
                .entry(canonical.to_string())
                .or_default()
                .push(alias.to_string());
            to_canonical.insert(alias.to_string(), canonical.to_string());
        }

        Self {
            to_aliases,
            to_canonical,
        }
    }

    /// Build the default OpenClaw ↔ SerialAgent alias map.
    ///
    /// Canonical tool names used by SerialAgent:
    ///   exec          – run a shell command (foreground / auto-background)
    ///   process       – manage background process sessions
    ///   fs.read_text  – read file contents (may be provided by a node)
    ///   fs.write_text – write file contents (may be provided by a node)
    ///   fs.list       – list directory entries (may be provided by a node)
    ///   http.request  – raw HTTP fetch (GET/POST/etc, returns body)
    ///   web.search    – search engine / SERP query (returns results list)
    pub fn default_openclaw() -> Self {
        Self::from_pairs(&[
            // ── Shell execution ─────────────────────────────────
            ("exec", "bash"),
            ("exec", "shell"),
            ("exec", "run"),
            // ── File operations ─────────────────────────────────
            ("fs.read_text", "files.read"),
            ("fs.read_text", "read_file"),
            ("fs.write_text", "files.write"),
            ("fs.write_text", "write_file"),
            ("fs.list", "files.list"),
            ("fs.list", "ls"),
            // ── HTTP fetch (raw content) ────────────────────────
            // web.fetch / fetch / http.get are all "get me this URL's body".
            // This is NOT the same as web.search.
            ("http.request", "web.fetch"),
            ("http.request", "fetch"),
            ("http.request", "http.get"),
            // ── Search (SERP / knowledge) ───────────────────────
            // search / serp are "find information about X".
            // Semantically distinct from http.request.
            ("web.search", "search"),
            ("web.search", "serp"),
            // ── Process management ──────────────────────────────
            ("process", "background"),
            ("process", "proc"),
        ])
    }

    /// Resolve a tool name to its canonical form.
    /// Returns the input unchanged if no alias mapping exists.
    pub fn resolve<'a>(&'a self, name: &'a str) -> &'a str {
        self.to_canonical
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(name)
    }

    /// Get all known aliases for a canonical tool name.
    pub fn aliases_for(&self, canonical: &str) -> &[String] {
        self.to_aliases
            .get(canonical)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Check if a tool name (canonical or alias) matches a given canonical name.
    pub fn matches(&self, tool_name: &str, canonical: &str) -> bool {
        tool_name == canonical || self.resolve(tool_name) == canonical
    }

    /// List all canonical tool names known to this map.
    pub fn canonicals(&self) -> Vec<&str> {
        self.to_aliases.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ToolAliasMap {
    fn default() -> Self {
        Self::default_openclaw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_alias_to_canonical() {
        let map = ToolAliasMap::default_openclaw();
        assert_eq!(map.resolve("bash"), "exec");
        assert_eq!(map.resolve("shell"), "exec");
        assert_eq!(map.resolve("files.read"), "fs.read_text");
        assert_eq!(map.resolve("read_file"), "fs.read_text");
    }

    #[test]
    fn web_fetch_resolves_to_http_request_not_search() {
        let map = ToolAliasMap::default_openclaw();
        // web.fetch is raw HTTP — must NOT resolve to web.search.
        assert_eq!(map.resolve("web.fetch"), "http.request");
        assert_eq!(map.resolve("fetch"), "http.request");
        assert_eq!(map.resolve("http.get"), "http.request");
        // search / serp resolve to web.search.
        assert_eq!(map.resolve("search"), "web.search");
        assert_eq!(map.resolve("serp"), "web.search");
    }

    #[test]
    fn http_request_and_web_search_are_distinct() {
        let map = ToolAliasMap::default_openclaw();
        // These two canonicals must never resolve to each other.
        assert!(!map.matches("web.fetch", "web.search"));
        assert!(!map.matches("search", "http.request"));
        assert!(map.matches("web.fetch", "http.request"));
        assert!(map.matches("search", "web.search"));
    }

    #[test]
    fn resolve_canonical_is_identity() {
        let map = ToolAliasMap::default_openclaw();
        assert_eq!(map.resolve("exec"), "exec");
        assert_eq!(map.resolve("fs.read_text"), "fs.read_text");
        assert_eq!(map.resolve("http.request"), "http.request");
        assert_eq!(map.resolve("web.search"), "web.search");
    }

    #[test]
    fn resolve_unknown_is_identity() {
        let map = ToolAliasMap::default_openclaw();
        assert_eq!(map.resolve("custom.tool"), "custom.tool");
    }

    #[test]
    fn aliases_for_canonical() {
        let map = ToolAliasMap::default_openclaw();
        let exec_aliases = map.aliases_for("exec");
        assert!(exec_aliases.contains(&"bash".to_string()));
        assert!(exec_aliases.contains(&"shell".to_string()));
        assert!(exec_aliases.contains(&"run".to_string()));

        let http_aliases = map.aliases_for("http.request");
        assert!(http_aliases.contains(&"web.fetch".to_string()));
        assert!(http_aliases.contains(&"fetch".to_string()));
        assert!(!http_aliases.contains(&"search".to_string()));
    }

    #[test]
    fn matches_canonical_or_alias() {
        let map = ToolAliasMap::default_openclaw();
        assert!(map.matches("bash", "exec"));
        assert!(map.matches("exec", "exec"));
        assert!(map.matches("files.read", "fs.read_text"));
        assert!(!map.matches("bash", "fs.read_text"));
    }
}
