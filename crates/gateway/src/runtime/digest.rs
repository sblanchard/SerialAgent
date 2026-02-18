//! Digest pipeline — fetch sources, detect changes, build prompts.
//!
//! Used by the schedule runner to fetch web content, compute content
//! hashes for change detection, and assemble the final prompt that
//! gets sent to the LLM.

use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};

use crate::runtime::schedules::{DigestMode, FetchConfig, Schedule, SourceState};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FetchResult
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Result of fetching a single source URL.
#[derive(Clone, Debug)]
pub struct FetchResult {
    pub url: String,
    pub content: String,
    pub content_hash: String,
    pub http_status: u16,
    pub fetched_at: DateTime<Utc>,
    pub changed: bool,
    pub error: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Fetching
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Compute SHA-256 hex digest of content.
pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Detect whether content has changed compared to previous state.
pub fn has_changed(new_hash: &str, prev_state: Option<&SourceState>) -> bool {
    match prev_state.and_then(|s| s.last_content_hash.as_deref()) {
        Some(prev_hash) => prev_hash != new_hash,
        None => true, // No previous state = treat as changed.
    }
}

/// Fetch a single URL using the schedule's fetch configuration.
pub async fn fetch_source(
    url: &str,
    config: &FetchConfig,
) -> FetchResult {
    let now = Utc::now();

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(config.timeout_ms))
        .user_agent(&config.user_agent)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return FetchResult {
                url: url.to_string(),
                content: String::new(),
                content_hash: content_hash(""),
                http_status: 0,
                fetched_at: now,
                changed: false,
                error: Some(format!("failed to build HTTP client: {}", e)),
            };
        }
    };

    match client.get(url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            match resp.text().await {
                Ok(mut body) => {
                    // Hash the full body BEFORE truncation so tail changes
                    // are detected even when max_size_bytes is set.
                    let hash = content_hash(&body);
                    // Enforce max_size_bytes if set.
                    if config.max_size_bytes > 0
                        && body.len() as u64 > config.max_size_bytes
                    {
                        body.truncate(config.max_size_bytes as usize);
                    }
                    FetchResult {
                        url: url.to_string(),
                        content: body,
                        content_hash: hash,
                        http_status: status,
                        fetched_at: now,
                        changed: false, // Caller sets this after comparing.
                        error: None,
                    }
                }
                Err(e) => FetchResult {
                    url: url.to_string(),
                    content: String::new(),
                    content_hash: content_hash(""),
                    http_status: status,
                    fetched_at: now,
                    changed: false,
                    error: Some(format!("failed to read response body: {}", e)),
                },
            }
        }
        Err(e) => FetchResult {
            url: url.to_string(),
            content: String::new(),
            content_hash: content_hash(""),
            http_status: 0,
            fetched_at: now,
            changed: false,
            error: Some(format!("HTTP request failed: {}", e)),
        },
    }
}

/// Fetch all sources for a schedule concurrently, detecting changes against previous state.
pub async fn fetch_all_sources(schedule: &Schedule) -> Vec<FetchResult> {
    let futs: Vec<_> = schedule
        .sources
        .iter()
        .map(|url| {
            let url = url.clone();
            let config = schedule.fetch_config.clone();
            async move { fetch_source(&url, &config).await }
        })
        .collect();

    let mut results = futures_util::future::join_all(futs).await;
    for result in &mut results {
        if result.error.is_none() {
            let prev = schedule.source_states.get(&result.url);
            result.changed = has_changed(&result.content_hash, prev);
        }
    }
    results
}

/// Strip HTML tags to extract plain text. Preserves block-level whitespace.
pub fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_block = false;

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
            }
            '>' if in_tag => {
                in_tag = false;
                if !last_was_block {
                    // Block tags → newline
                    last_was_block = true;
                }
            }
            _ if !in_tag => {
                if ch == '\n' || ch == '\r' {
                    if !last_was_block {
                        out.push('\n');
                        last_was_block = true;
                    }
                } else {
                    last_was_block = false;
                    out.push(ch);
                }
            }
            _ => {}
        }
    }
    // Collapse runs of whitespace-only lines.
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_empty = false;
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_empty {
                collapsed.push('\n');
            }
            prev_empty = true;
        } else {
            if prev_empty && !collapsed.is_empty() {
                collapsed.push('\n');
            }
            collapsed.push_str(trimmed);
            collapsed.push('\n');
            prev_empty = false;
        }
    }
    collapsed.trim().to_string()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Prompt building
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Build the digest prompt from fetched sources and the schedule config.
///
/// Supports placeholders in `prompt_template`:
/// - `{{sources}}` — all source URLs (bullet list)
/// - `{{changed_sources}}` — only changed source URLs
/// - `{{date}}` — current date in YYYY-MM-DD format
/// - `{{time}}` — current time in HH:MM UTC format
/// - `{{content}}` — concatenated source content (per digest_mode)
/// - `{{schedule_name}}` — name of the schedule
/// - `{{timezone}}` — schedule's configured timezone
pub fn build_digest_prompt(
    schedule: &Schedule,
    results: &[FetchResult],
) -> String {
    let now = Utc::now();

    // Build content based on digest mode.
    let included: Vec<&FetchResult> = match schedule.digest_mode {
        DigestMode::Full => results
            .iter()
            .filter(|r| r.error.is_none())
            .collect(),
        DigestMode::ChangesOnly => results
            .iter()
            .filter(|r| r.error.is_none() && r.changed)
            .collect(),
    };

    let content_block = if included.is_empty() {
        "No content available.".to_string()
    } else {
        included
            .iter()
            .map(|r| {
                // Strip HTML tags from content to reduce token waste.
                let clean = if r.content.contains('<') && r.content.contains('>') {
                    strip_html_tags(&r.content)
                } else {
                    r.content.clone()
                };
                format!("## {}\n\n{}", r.url, clean)
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    };

    let all_sources = results
        .iter()
        .map(|r| format!("- {}", r.url))
        .collect::<Vec<_>>()
        .join("\n");

    let changed_sources = results
        .iter()
        .filter(|r| r.changed)
        .map(|r| format!("- {}", r.url))
        .collect::<Vec<_>>()
        .join("\n");

    let template = &schedule.prompt_template;

    // If template has placeholders, substitute them.
    if template.contains("{{") {
        template
            .replace("{{sources}}", &all_sources)
            .replace("{{changed_sources}}", &changed_sources)
            .replace("{{date}}", &now.format("%Y-%m-%d").to_string())
            .replace("{{time}}", &now.format("%H:%M UTC").to_string())
            .replace("{{content}}", &content_block)
            .replace("{{schedule_name}}", &schedule.name)
            .replace("{{timezone}}", &schedule.timezone)
    } else {
        // Legacy mode: append content after the template.
        if included.is_empty() {
            template.clone()
        } else {
            format!(
                "{}\n\nURLs:\n{}\n\n---\n\n{}",
                template, all_sources, content_block
            )
        }
    }
}

/// Convert fetch results into updated SourceState entries.
pub fn build_source_states(results: &[FetchResult]) -> std::collections::HashMap<String, SourceState> {
    results
        .iter()
        .map(|r| {
            (
                r.url.clone(),
                SourceState {
                    last_fetched_at: Some(r.fetched_at),
                    last_content_hash: if r.error.is_none() {
                        Some(r.content_hash.clone())
                    } else {
                        None
                    },
                    last_http_status: if r.http_status > 0 {
                        Some(r.http_status)
                    } else {
                        None
                    },
                    last_error: r.error.clone(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::schedules::*;
    use std::collections::HashMap;

    fn make_result(url: &str, content: &str, changed: bool) -> FetchResult {
        FetchResult {
            url: url.to_string(),
            content: content.to_string(),
            content_hash: content_hash(content),
            http_status: 200,
            fetched_at: Utc::now(),
            changed,
            error: None,
        }
    }

    fn make_error_result(url: &str, err: &str) -> FetchResult {
        FetchResult {
            url: url.to_string(),
            content: String::new(),
            content_hash: content_hash(""),
            http_status: 0,
            fetched_at: Utc::now(),
            changed: false,
            error: Some(err.to_string()),
        }
    }

    fn test_schedule_for_digest(
        mode: DigestMode,
        template: &str,
        sources: Vec<&str>,
    ) -> Schedule {
        Schedule {
            id: uuid::Uuid::new_v4(),
            name: "digest-test".into(),
            cron: "0 * * * *".into(),
            timezone: "UTC".into(),
            enabled: true,
            agent_id: String::new(),
            prompt_template: template.to_string(),
            sources: sources.into_iter().map(String::from).collect(),
            delivery_targets: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_run_id: None,
            last_run_at: None,
            next_run_at: None,
            missed_policy: MissedPolicy::default(),
            max_concurrency: 1,
            timeout_ms: None,
            digest_mode: mode,
            fetch_config: FetchConfig::default(),
            max_catchup_runs: 5,
            source_states: HashMap::new(),
            last_error: None,
            last_error_at: None,
            consecutive_failures: 0,
            cooldown_until: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_runs: 0,
        }
    }

    #[test]
    fn content_hash_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);
        let h3 = content_hash("different");
        assert_ne!(h1, h3);
    }

    #[test]
    fn has_changed_no_previous_state() {
        assert!(has_changed("abc123", None));
    }

    #[test]
    fn has_changed_same_hash() {
        let state = SourceState {
            last_fetched_at: Some(Utc::now()),
            last_content_hash: Some("abc123".into()),
            last_http_status: Some(200),
            last_error: None,
        };
        assert!(!has_changed("abc123", Some(&state)));
    }

    #[test]
    fn has_changed_different_hash() {
        let state = SourceState {
            last_fetched_at: Some(Utc::now()),
            last_content_hash: Some("abc123".into()),
            last_http_status: Some(200),
            last_error: None,
        };
        assert!(has_changed("xyz789", Some(&state)));
    }

    #[test]
    fn build_digest_full_mode() {
        let sched = test_schedule_for_digest(
            DigestMode::Full,
            "Summarize these articles",
            vec!["https://a.com", "https://b.com"],
        );
        let results = vec![
            make_result("https://a.com", "Article A content", true),
            make_result("https://b.com", "Article B content", false),
        ];
        let prompt = build_digest_prompt(&sched, &results);
        assert!(prompt.contains("Article A content"), "Full mode includes all sources");
        assert!(prompt.contains("Article B content"), "Full mode includes unchanged too");
    }

    #[test]
    fn build_digest_changes_only_mode() {
        let sched = test_schedule_for_digest(
            DigestMode::ChangesOnly,
            "Summarize changes: {{content}}",
            vec!["https://a.com", "https://b.com"],
        );
        let results = vec![
            make_result("https://a.com", "New content A", true),
            make_result("https://b.com", "Same content B", false),
        ];
        let prompt = build_digest_prompt(&sched, &results);
        assert!(prompt.contains("New content A"), "Should include changed source");
        assert!(!prompt.contains("Same content B"), "Should exclude unchanged source");
    }

    #[test]
    fn build_digest_placeholder_substitution() {
        let sched = test_schedule_for_digest(
            DigestMode::Full,
            "Date: {{date}}\nSources: {{sources}}\nChanged: {{changed_sources}}\n\n{{content}}",
            vec!["https://a.com", "https://b.com"],
        );
        let results = vec![
            make_result("https://a.com", "Content A", true),
            make_result("https://b.com", "Content B", false),
        ];
        let prompt = build_digest_prompt(&sched, &results);
        assert!(prompt.contains("Date: "), "Should replace {{date}}");
        assert!(prompt.contains("- https://a.com"), "Should list all sources");
        assert!(prompt.contains("- https://b.com"), "Should list all sources");
        assert!(!prompt.contains("{{sources}}"), "Placeholder should be replaced");
    }

    #[test]
    fn build_digest_no_sources_content() {
        let sched = test_schedule_for_digest(
            DigestMode::ChangesOnly,
            "Changes: {{content}}",
            vec!["https://a.com"],
        );
        // All unchanged → no content in changes_only mode.
        let results = vec![make_result("https://a.com", "Old content", false)];
        let prompt = build_digest_prompt(&sched, &results);
        assert!(prompt.contains("No content available"), "Should show no-content message");
    }

    #[test]
    fn build_digest_error_sources_excluded() {
        let sched = test_schedule_for_digest(
            DigestMode::Full,
            "Report: {{content}}",
            vec!["https://a.com", "https://bad.com"],
        );
        let results = vec![
            make_result("https://a.com", "Good content", true),
            make_error_result("https://bad.com", "connection refused"),
        ];
        let prompt = build_digest_prompt(&sched, &results);
        assert!(prompt.contains("Good content"), "Should include successful fetch");
        assert!(!prompt.contains("connection refused"), "Error content should not be in prompt");
    }

    #[test]
    fn build_source_states_from_results() {
        let results = vec![
            make_result("https://a.com", "content", true),
            make_error_result("https://bad.com", "timeout"),
        ];
        let states = build_source_states(&results);
        assert_eq!(states.len(), 2);
        assert!(states["https://a.com"].last_content_hash.is_some());
        assert!(states["https://bad.com"].last_content_hash.is_none());
        assert!(states["https://bad.com"].last_error.is_some());
    }
}
