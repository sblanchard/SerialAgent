use std::sync::Arc;

use crate::config::Config;
use crate::error::Result;
use crate::memory::client::SerialMemoryClient;
use crate::memory::types::{SearchQuery, UserProfile};
use crate::trace::TraceEvent;

/// Builds the USER_FACTS section by querying SerialMemory.
///
/// USER_FACTS is separate from USER.md:
/// - USER.md = declared preferences/constraints (static, in workspace)
/// - USER_FACTS = learned facts (vector-retrieved + pinned facts from SerialMemory)
pub struct UserFactsBuilder {
    client: Arc<SerialMemoryClient>,
    config: Arc<Config>,
}

impl UserFactsBuilder {
    pub fn new(client: Arc<SerialMemoryClient>, config: Arc<Config>) -> Self {
        Self { client, config }
    }

    /// Fetch user profile and recent relevant facts, format as a capped string.
    pub async fn build(&self, user_id: &str) -> Result<String> {
        let max_chars = self.config.context.user_facts_max_chars;

        // 1. Get user profile (preferences, skills, goals, background)
        let profile = self.client.memory_about_user(user_id).await;

        // 2. Search for pinned / high-confidence user facts
        let facts = self
            .client
            .memory_search(SearchQuery {
                query: format!("user preferences and facts about {user_id}"),
                mode: "hybrid".into(),
                limit: 20,
                threshold: 0.5,
                include_entities: false,
                memory_type: Some("knowledge".into()),
            })
            .await;

        // Build output even if one or both calls failed (graceful degradation)
        let mut output = String::new();

        // Format profile section
        if let Ok(ref profile) = profile {
            append_profile_section(&mut output, profile);
        }

        // Format retrieved facts
        let search_count;
        if let Ok(ref results) = facts {
            search_count = results.len();
            if !results.is_empty() {
                output.push_str("\n## Retrieved Facts\n");
                for result in results {
                    let line = format!("- {} (similarity: {:.2})\n", result.content, result.similarity);
                    output.push_str(&line);
                }
            }
        } else {
            search_count = 0;
        }

        // Apply cap
        if output.len() > max_chars {
            let boundary = output.floor_char_boundary(max_chars);
            output.truncate(boundary);
            output.push_str("\n\n[USER_FACTS_TRUNCATED]\n");
        }

        // Emit trace
        TraceEvent::UserFactsFetched {
            user_id: user_id.to_string(),
            facts_chars: output.len(),
            pinned_count: 0, // profile-based, not individually tracked
            search_count,
        }
        .emit();

        Ok(output)
    }
}

fn append_profile_section(output: &mut String, profile: &UserProfile) {
    if !profile.preferences.is_empty() {
        output.push_str("## Preferences\n");
        for (k, v) in &profile.preferences {
            output.push_str(&format!("- {k}: {v}\n"));
        }
    }

    if !profile.skills.is_empty() {
        output.push_str("\n## Skills\n");
        for (k, v) in &profile.skills {
            output.push_str(&format!("- {k}: {v}\n"));
        }
    }

    if !profile.goals.is_empty() {
        output.push_str("\n## Goals\n");
        for (k, v) in &profile.goals {
            output.push_str(&format!("- {k}: {v}\n"));
        }
    }

    if !profile.background.is_empty() {
        output.push_str("\n## Background\n");
        for (k, v) in &profile.background {
            output.push_str(&format!("- {k}: {v}\n"));
        }
    }
}
