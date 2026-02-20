//! In-memory reverse index for full-text transcript search.
//!
//! Maps lowercase words to session IDs with match counts. Built at startup
//! by scanning JSONL files and kept live by indexing new lines as they are
//! appended.

use std::collections::HashMap;
use std::path::Path;

use parking_lot::RwLock;

use crate::transcript::TranscriptLine;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A single search result.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub session_id: String,
    pub match_count: usize,
    /// First matching line content, truncated to a reasonable preview length.
    pub preview: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TranscriptIndex
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// In-memory reverse index: word -> { session_id -> count }.
pub struct TranscriptIndex {
    /// word -> { session_id -> count }
    index: RwLock<HashMap<String, HashMap<String, usize>>>,
    /// (session_id, word) -> first matching line content for preview
    previews: RwLock<HashMap<(String, String), String>>,
}

const MAX_PREVIEW_LEN: usize = 160;
const MAX_RESULTS: usize = 50;

impl TranscriptIndex {
    pub fn new() -> Self {
        Self {
            index: RwLock::new(HashMap::new()),
            previews: RwLock::new(HashMap::new()),
        }
    }

    /// Build the index from existing JSONL files on disk.
    ///
    /// Scans all `.jsonl` files in `dir`. Each file's stem is treated as the
    /// session ID.
    pub fn build_from_dir(dir: &Path) -> Self {
        let index = Self::new();

        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    dir = %dir.display(),
                    "failed to read transcript dir for search index"
                );
                return index;
            }
        };

        let mut file_count = 0u64;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            let session_id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_owned(),
                None => continue,
            };

            let raw = match std::fs::read_to_string(&path) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        path = %path.display(),
                        "skipping transcript file for indexing"
                    );
                    continue;
                }
            };

            for line in raw.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(tl) = serde_json::from_str::<TranscriptLine>(line) {
                    index.index_content(&session_id, &tl.content);
                }
            }

            file_count += 1;
        }

        tracing::info!(
            files = file_count,
            words = index.index.read().len(),
            "transcript search index built"
        );

        index
    }

    /// Index a single content string for a session.
    pub fn index_content(&self, session_id: &str, content: &str) {
        let words = tokenize(content);
        if words.is_empty() {
            return;
        }

        let mut idx = self.index.write();
        let mut previews = self.previews.write();

        for word in &words {
            let sessions = idx.entry(word.clone()).or_default();
            *sessions.entry(session_id.to_owned()).or_insert(0) += 1;

            // Store preview for the first match of this word in this session.
            let key = (session_id.to_owned(), word.clone());
            previews.entry(key).or_insert_with(|| truncate_preview(content));
        }
    }

    /// Search for sessions matching the query (AND semantics for multi-word).
    ///
    /// Returns up to 50 results sorted by total match count descending.
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let query_words = tokenize(query);
        if query_words.is_empty() {
            return vec![];
        }

        let idx = self.index.read();
        let previews = self.previews.read();

        // Find sessions that match ALL query words (intersection).
        let mut candidates: Option<HashMap<String, usize>> = None;

        for word in &query_words {
            let word_matches = match idx.get(word) {
                Some(m) => m,
                None => return vec![], // AND semantics: if any word has no matches, empty result
            };

            candidates = Some(match candidates {
                None => word_matches.clone(),
                Some(current) => {
                    // Intersect: keep only sessions present in both, sum counts.
                    current
                        .into_iter()
                        .filter_map(|(sid, count)| {
                            word_matches
                                .get(&sid)
                                .map(|wc| (sid, count + wc))
                        })
                        .collect()
                }
            });
        }

        let scored = match candidates {
            Some(c) => c,
            None => return vec![],
        };

        // Sort by score descending and take top results.
        let mut results: Vec<_> = scored.into_iter().collect();
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.truncate(MAX_RESULTS);

        results
            .into_iter()
            .map(|(session_id, match_count)| {
                // Find the best preview: use the first query word's preview.
                let preview = query_words
                    .iter()
                    .find_map(|w| {
                        previews
                            .get(&(session_id.clone(), w.clone()))
                            .cloned()
                    })
                    .unwrap_or_default();

                SearchHit {
                    session_id,
                    match_count,
                    preview,
                }
            })
            .collect()
    }
}

impl Default for TranscriptIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Tokenize text into lowercase alphanumeric words (minimum 2 characters).
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(String::from)
        .collect()
}

/// Truncate a string to a reasonable preview length, respecting UTF-8 boundaries.
fn truncate_preview(s: &str) -> String {
    if s.len() <= MAX_PREVIEW_LEN {
        return s.to_owned();
    }
    let mut end = MAX_PREVIEW_LEN;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        let tokens = tokenize("Hello, World! This is a test.");
        assert_eq!(tokens, vec!["hello", "world", "this", "is", "test"]);
    }

    #[test]
    fn tokenize_skips_single_chars() {
        let tokens = tokenize("I am a bot");
        // "i" and "a" are single chars, skipped
        assert_eq!(tokens, vec!["am", "bot"]);
    }

    #[test]
    fn tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn index_and_search_single_word() {
        let idx = TranscriptIndex::new();
        idx.index_content("s1", "Hello world");
        idx.index_content("s2", "Goodbye world");

        let hits = idx.search("world");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_and_semantics() {
        let idx = TranscriptIndex::new();
        idx.index_content("s1", "Hello world from Rust");
        idx.index_content("s2", "Hello from Python");

        // AND: both "hello" and "rust" must match
        let hits = idx.search("hello rust");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "s1");
    }

    #[test]
    fn search_no_match() {
        let idx = TranscriptIndex::new();
        idx.index_content("s1", "Hello world");

        let hits = idx.search("nonexistent");
        assert!(hits.is_empty());
    }

    #[test]
    fn search_empty_query() {
        let idx = TranscriptIndex::new();
        idx.index_content("s1", "Hello world");

        let hits = idx.search("");
        assert!(hits.is_empty());
    }

    #[test]
    fn search_sorted_by_count() {
        let idx = TranscriptIndex::new();
        idx.index_content("s1", "rust rust rust");
        idx.index_content("s2", "rust");

        let hits = idx.search("rust");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].session_id, "s1");
        assert_eq!(hits[0].match_count, 3);
        assert_eq!(hits[1].session_id, "s2");
        assert_eq!(hits[1].match_count, 1);
    }

    #[test]
    fn preview_is_stored() {
        let idx = TranscriptIndex::new();
        idx.index_content("s1", "This is a test message for preview");

        let hits = idx.search("test");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].preview.contains("test message"));
    }

    #[test]
    fn truncate_preview_short() {
        let result = truncate_preview("short");
        assert_eq!(result, "short");
    }

    #[test]
    fn truncate_preview_long() {
        let long = "a".repeat(200);
        let result = truncate_preview(&long);
        assert!(result.ends_with("..."));
        assert!(result.len() <= MAX_PREVIEW_LEN + 3);
    }
}
