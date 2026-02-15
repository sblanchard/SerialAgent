//! `UserFactsBuilder` — queries SerialMemory for user-specific context
//! (persona attributes + relevant memories) and formats the result as a
//! compact Markdown string suitable for injection into the system prompt.
//!
//! Gracefully degrades: if SerialMemory is unreachable or returns errors,
//! the builder returns an empty string rather than propagating the failure.

use sa_domain::trace::TraceEvent;
use tracing::warn;

use crate::provider::SerialMemoryProvider;
use crate::types::RagSearchRequest;

/// Builds the `USER_FACTS` section injected into the context pack.
pub struct UserFactsBuilder<'a> {
    provider: &'a dyn SerialMemoryProvider,
    user_id: String,
    max_chars: usize,
    search_queries: Vec<String>,
}

impl<'a> UserFactsBuilder<'a> {
    /// Create a new builder.
    ///
    /// * `provider`       — any implementation of `SerialMemoryProvider`
    /// * `user_id`        — user identifier for trace events
    /// * `max_chars`      — hard cap on the resulting string length
    pub fn new(
        provider: &'a dyn SerialMemoryProvider,
        user_id: impl Into<String>,
        max_chars: usize,
    ) -> Self {
        Self {
            provider,
            user_id: user_id.into(),
            max_chars,
            search_queries: Vec::new(),
        }
    }

    /// Add a contextual search query that will be used to retrieve relevant
    /// memories beyond the static persona attributes.
    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.search_queries.push(query.into());
        self
    }

    /// Add multiple contextual search queries.
    pub fn with_queries(mut self, queries: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.search_queries.extend(queries.into_iter().map(|q| q.into()));
        self
    }

    /// Fetch persona + search results and assemble the USER_FACTS string.
    ///
    /// Never fails — returns an empty string on error.
    pub async fn build(&self) -> String {
        let mut sections: Vec<(&str, String)> = Vec::new();
        let mut pinned_count: usize = 0;
        let mut search_count: usize = 0;

        // ── 1. Fetch persona ─────────────────────────────────────────
        match self.provider.get_persona().await {
            Ok(persona) => {
                let persona_parts = self.extract_persona_sections(&persona);
                pinned_count = persona_parts.iter().map(|(_, v)| v.lines().count()).sum();
                sections.extend(persona_parts);
            }
            Err(e) => {
                warn!(user_id = %self.user_id, error = %e, "failed to fetch persona from SerialMemory");
            }
        }

        // ── 2. Search for relevant facts ─────────────────────────────
        let mut retrieved_facts = Vec::new();
        for query in &self.search_queries {
            match self
                .provider
                .search(RagSearchRequest {
                    query: query.clone(),
                    limit: Some(5),
                })
                .await
            {
                Ok(resp) => {
                    for mem in &resp.memories {
                        let content = mem.content.trim();
                        if !content.is_empty() {
                            retrieved_facts.push(content.to_owned());
                        }
                    }
                    search_count += resp.memories.len();
                }
                Err(e) => {
                    warn!(
                        user_id = %self.user_id,
                        query = %query,
                        error = %e,
                        "SerialMemory search failed for user facts"
                    );
                }
            }
        }

        if !retrieved_facts.is_empty() {
            // De-duplicate (stable order)
            let mut seen = std::collections::HashSet::new();
            let mut unique = Vec::new();
            for fact in &retrieved_facts {
                if seen.insert(fact.clone()) {
                    unique.push(fact.clone());
                }
            }
            let body = unique
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(("Retrieved Facts", body));
        }

        // ── 3. Assemble markdown ─────────────────────────────────────
        let assembled = self.assemble_markdown(&sections);

        // ── 4. Emit trace event ──────────────────────────────────────
        TraceEvent::UserFactsFetched {
            user_id: self.user_id.clone(),
            facts_chars: assembled.len(),
            pinned_count,
            search_count,
        }
        .emit();

        assembled
    }

    // ── internal helpers ─────────────────────────────────────────────

    /// Extract structured persona sections from the raw JSON returned by
    /// GET /api/persona.
    ///
    /// The server may return either:
    ///   - An object with top-level keys like `preferences`, `skills`, etc.
    ///   - An array of attribute objects `[{ attributeType, attributeKey, attributeValue }]`.
    ///
    /// We handle both and produce titled sub-sections.
    fn extract_persona_sections(
        &self,
        persona: &serde_json::Value,
    ) -> Vec<(&'static str, String)> {
        let mut result = Vec::new();

        // Section mapping: JSON key -> display heading
        let section_keys: &[(&str, &str)] = &[
            ("preferences", "Preferences"),
            ("skills", "Skills"),
            ("goals", "Goals"),
            ("background", "Background"),
        ];

        if let Some(obj) = persona.as_object() {
            for &(json_key, heading) in section_keys {
                if let Some(val) = obj.get(json_key) {
                    let body = self.value_to_bullet_list(val);
                    if !body.is_empty() {
                        result.push((heading, body));
                    }
                }
            }

            // Catch-all for any extra top-level keys not in the canonical set
            for (key, val) in obj {
                let canonical = section_keys.iter().any(|(k, _)| *k == key.as_str());
                if !canonical && !val.is_null() {
                    let body = self.value_to_bullet_list(val);
                    if !body.is_empty() {
                        // Title-case the key
                        let heading_owned = title_case(key);
                        // We can't return a reference to a local, so we push
                        // it below after the loop.  For now collect them.
                        result.push(("Other", format!("**{heading_owned}**\n{body}")));
                    }
                }
            }
        } else if let Some(arr) = persona.as_array() {
            // Array-of-attribute form: group by attributeType
            let mut groups: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            for item in arr {
                let attr_type = item
                    .get("attributeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("other")
                    .to_owned();
                let key = item
                    .get("attributeKey")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let value = item
                    .get("attributeValue")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !value.is_empty() {
                    if key.is_empty() {
                        groups.entry(attr_type).or_default().push(format!("- {value}"));
                    } else {
                        groups
                            .entry(attr_type)
                            .or_default()
                            .push(format!("- **{key}**: {value}"));
                    }
                }
            }

            for &(json_key, heading) in section_keys {
                if let Some(lines) = groups.remove(json_key) {
                    result.push((heading, lines.join("\n")));
                }
            }
            // Remaining groups
            for (group_key, lines) in groups {
                let heading_owned = title_case(&group_key);
                result.push(("Other", format!("**{heading_owned}**\n{}", lines.join("\n"))));
            }
        }

        result
    }

    /// Render a JSON value as a bullet list.
    fn value_to_bullet_list(&self, val: &serde_json::Value) -> String {
        match val {
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|v| {
                    let s = match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Object(obj) => {
                            // { key: ..., value: ... } or { attributeKey, attributeValue }
                            let key = obj
                                .get("key")
                                .or_else(|| obj.get("attributeKey"))
                                .and_then(|v| v.as_str());
                            let value = obj
                                .get("value")
                                .or_else(|| obj.get("attributeValue"))
                                .and_then(|v| v.as_str());
                            match (key, value) {
                                (Some(k), Some(v)) => format!("**{k}**: {v}"),
                                (None, Some(v)) => v.to_owned(),
                                _ => return None,
                            }
                        }
                        other => {
                            let s = other.to_string();
                            if s == "null" {
                                return None;
                            }
                            s
                        }
                    };
                    if s.is_empty() {
                        None
                    } else {
                        Some(format!("- {s}"))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            serde_json::Value::Object(obj) => obj
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| {
                    let display = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    format!("- **{k}**: {display}")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            serde_json::Value::String(s) if !s.is_empty() => format!("- {s}"),
            _ => String::new(),
        }
    }

    /// Assemble titled sections into final markdown, respecting `max_chars`.
    fn assemble_markdown(&self, sections: &[(&str, String)]) -> String {
        if sections.is_empty() {
            return String::new();
        }

        let mut output = String::new();

        for (heading, body) in sections {
            let section_block = format!("### {heading}\n{body}\n\n");

            if output.len() + section_block.len() > self.max_chars {
                // Try to fit a partial section
                let remaining = self.max_chars.saturating_sub(output.len());
                if remaining > 30 {
                    // Enough room for at least a heading + truncation marker
                    let truncated = &section_block[..section_block
                        .char_indices()
                        .take_while(|(i, _)| *i < remaining.saturating_sub(25))
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(0)];
                    output.push_str(truncated);
                    output.push_str("\n[USER_FACTS_TRUNCATED]\n");
                } else {
                    output.push_str("[USER_FACTS_TRUNCATED]\n");
                }
                return output;
            }

            output.push_str(&section_block);
        }

        // Final length check (defensive)
        if output.len() > self.max_chars {
            let cut = output
                .char_indices()
                .take_while(|(i, _)| *i < self.max_chars.saturating_sub(25))
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            output.truncate(cut);
            output.push_str("\n[USER_FACTS_TRUNCATED]\n");
        }

        output
    }
}

/// Simple title-case helper: `"some_key"` -> `"Some Key"`.
fn title_case(s: &str) -> String {
    s.replace('_', " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{upper}{}", chars.as_str())
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_title_case() {
        assert_eq!(title_case("hello_world"), "Hello World");
        assert_eq!(title_case("preferences"), "Preferences");
        assert_eq!(title_case(""), "");
    }
}
