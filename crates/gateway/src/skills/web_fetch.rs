//! `web.fetch` skill — fetch a URL with strict limits, optionally extract
//! readable text from HTML.
//!
//! Safety properties:
//! - Hard timeout (default 20s, configurable via SA_WEB_TIMEOUT_SECS)
//! - Max response size (default 5MB, configurable via SA_WEB_MAX_BYTES)
//! - Max text output (default 250k chars, configurable via SA_WEB_MAX_TEXT_CHARS)
//! - Redirect limit (5 hops)
//! - User-Agent identifies the bot

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};

use super::{DangerLevel, Skill, SkillContext, SkillResult, SkillSpec};

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub struct WebFetchSkill {
    client: reqwest::Client,
    max_bytes: usize,
    max_text_chars: usize,
}

impl WebFetchSkill {
    pub fn new() -> Result<Self> {
        let timeout_s = std::env::var("SA_WEB_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(20);

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_s))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .context("build reqwest client for web.fetch")?;

        Ok(Self {
            client,
            max_bytes: env_usize("SA_WEB_MAX_BYTES", 5 * 1024 * 1024),
            max_text_chars: env_usize("SA_WEB_MAX_TEXT_CHARS", 250_000),
        })
    }

    /// Simple HTML-to-text extraction without external dependencies.
    /// Strips tags, collapses whitespace, extracts text content.
    fn html_to_text(&self, html: &str) -> String {
        let mut out = String::new();
        let mut in_tag = false;
        let mut in_script = false;
        let mut in_style = false;
        let mut tag_buf = String::new();

        for ch in html.chars() {
            if out.chars().count() >= self.max_text_chars {
                break;
            }

            match ch {
                '<' => {
                    in_tag = true;
                    tag_buf.clear();
                }
                '>' if in_tag => {
                    in_tag = false;
                    let tag_lower = tag_buf.to_lowercase();

                    // Track script/style blocks
                    if tag_lower.starts_with("script") {
                        in_script = true;
                    } else if tag_lower.starts_with("/script") {
                        in_script = false;
                    } else if tag_lower.starts_with("style") {
                        in_style = true;
                    } else if tag_lower.starts_with("/style") {
                        in_style = false;
                    }

                    // Block-level tags → newline
                    if tag_lower.starts_with('/')
                        && matches!(
                            tag_lower.trim_start_matches('/'),
                            "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                                | "li" | "tr" | "br" | "article" | "section"
                                | "header" | "footer" | "blockquote"
                        )
                    {
                        if !out.ends_with('\n') {
                            out.push('\n');
                        }
                    } else if tag_lower == "br" || tag_lower == "br/" {
                        out.push('\n');
                    }

                    tag_buf.clear();
                }
                _ if in_tag => {
                    tag_buf.push(ch);
                }
                _ if in_script || in_style => {
                    // Skip content inside script/style
                }
                _ => {
                    out.push(ch);
                }
            }
        }

        // Decode common HTML entities
        let out = out
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ");

        // Collapse excessive whitespace (but keep newlines)
        let mut result = String::new();
        let mut prev_newline = false;
        for line in out.lines() {
            let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if trimmed.is_empty() {
                if !prev_newline {
                    result.push('\n');
                    prev_newline = true;
                }
            } else {
                result.push_str(&trimmed);
                result.push('\n');
                prev_newline = false;
            }
        }

        result.trim().to_string()
    }
}

#[async_trait::async_trait]
impl Skill for WebFetchSkill {
    fn spec(&self) -> SkillSpec {
        SkillSpec {
            name: "web.fetch".to_string(),
            title: "Web Fetch".to_string(),
            description: "Fetch a URL with strict limits; optionally extract readable text from HTML.".to_string(),
            args_schema: json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" },
                    "extract_text": { "type": "boolean", "default": true, "description": "Extract readable text from HTML" },
                    "accept": { "type": "string", "default": "text/html,application/xhtml+xml,application/json,text/plain" }
                }
            }),
            returns_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "status": { "type": "integer" },
                    "content_type": { "type": "string" },
                    "bytes": { "type": "integer" },
                    "text": { "type": "string" },
                    "raw_snippet": { "type": "string" }
                }
            }),
            danger_level: DangerLevel::Network,
        }
    }

    async fn call(&self, _ctx: SkillContext, args: Value) -> Result<SkillResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing args.url"))?;
        let extract_text = args
            .get("extract_text")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let accept = args
            .get("accept")
            .and_then(|v| v.as_str())
            .unwrap_or("text/html,application/xhtml+xml,application/json,text/plain");

        let resp = self
            .client
            .get(url)
            .header(USER_AGENT, "SerialAgent/1.0 (+https://serialcoder.com)")
            .header("Accept", accept)
            .send()
            .await
            .with_context(|| format!("fetch {}", url))?;

        let status = resp.status().as_u16() as i64;
        let ct = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Stream body with hard byte cap
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if buf.len() + chunk.len() > self.max_bytes {
                return Ok(SkillResult {
                    ok: false,
                    output: json!({
                        "error": "SizeLimitExceeded",
                        "message": format!("response exceeded {} bytes limit", self.max_bytes)
                    }),
                    preview: "SizeLimitExceeded: response too large".to_string(),
                });
            }
            buf.extend_from_slice(&chunk);
        }

        let raw_snippet = String::from_utf8_lossy(&buf[..buf.len().min(2048)]).to_string();

        let text = if extract_text && ct.contains("html") {
            self.html_to_text(&String::from_utf8_lossy(&buf))
        } else if ct.contains("json") || ct.contains("text/") || ct.is_empty() {
            let s = String::from_utf8_lossy(&buf).to_string();
            if s.chars().count() > self.max_text_chars {
                s.chars().take(self.max_text_chars).collect()
            } else {
                s
            }
        } else {
            String::new()
        };

        let preview: String = text.chars().take(400).collect();

        let output = json!({
            "url": url,
            "status": status,
            "content_type": ct,
            "bytes": buf.len(),
            "text": text,
            "raw_snippet": raw_snippet,
        });

        Ok(SkillResult {
            ok: status >= 200 && status < 400,
            preview,
            output,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_text_strips_tags() {
        let skill = WebFetchSkill {
            client: reqwest::Client::new(),
            max_bytes: 1024,
            max_text_chars: 10_000,
        };
        let html = "<html><body><h1>Hello</h1><p>World</p><script>var x=1;</script></body></html>";
        let text = skill.html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("var x=1"));
    }

    #[test]
    fn html_to_text_decodes_entities() {
        let skill = WebFetchSkill {
            client: reqwest::Client::new(),
            max_bytes: 1024,
            max_text_chars: 10_000,
        };
        let html = "<p>A &amp; B &lt; C</p>";
        let text = skill.html_to_text(html);
        assert!(text.contains("A & B < C"));
    }

    #[test]
    fn html_to_text_respects_char_limit() {
        let skill = WebFetchSkill {
            client: reqwest::Client::new(),
            max_bytes: 1024,
            max_text_chars: 10,
        };
        let html = "<p>This is a very long text that should be truncated</p>";
        let text = skill.html_to_text(html);
        assert!(text.chars().count() <= 15); // some slack for cleanup
    }
}
