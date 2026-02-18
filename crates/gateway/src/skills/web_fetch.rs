//! `web.fetch` skill — fetch a URL with strict limits, optionally extract
//! readable text from HTML.
//!
//! Safety properties:
//! - Hard timeout (default 20s, configurable via SA_WEB_TIMEOUT_SECS)
//! - Max response size (default 5MB, configurable via SA_WEB_MAX_BYTES)
//! - Max text output (default 250k chars, configurable via SA_WEB_MAX_TEXT_CHARS)
//! - Redirect limit (5 hops)
//! - User-Agent identifies the bot

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use reqwest::Url;
use serde_json::{json, Value};

use super::{DangerLevel, Skill, SkillContext, SkillResult, SkillSpec};

/// Returns `true` if the given IP address belongs to a private, loopback,
/// link-local, or otherwise non-public network range.
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()                                       // 127.0.0.0/8
                || v4.is_private()                                 // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()                              // 169.254.0.0/16
                || v4.is_broadcast()                               // 255.255.255.255
                || v4.is_unspecified()                              // 0.0.0.0
                || is_v4_shared_address(v4)                        // 100.64.0.0/10 (CGNAT / shared)
                || is_v4_documentation(v4)                         // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
                || is_v4_benchmarking(v4)                          // 198.18.0.0/15
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()                                       // ::1
                || v6.is_unspecified()                              // ::
                || is_v6_unique_local(v6)                          // fd00::/8 (fc00::/7 unique-local)
                || is_v6_link_local(v6)                            // fe80::/10
        }
    }
}

/// 100.64.0.0/10 — Shared address space (RFC 6598 / CGNAT).
fn is_v4_shared_address(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0xC0) == 64
}

/// Documentation ranges: 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24.
fn is_v4_documentation(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
}

/// Benchmarking range: 198.18.0.0/15.
fn is_v4_benchmarking(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 198 && (octets[1] & 0xFE) == 18
}

/// Unique-local addresses: fc00::/7 (in practice fd00::/8).
fn is_v6_unique_local(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();
    (segments[0] & 0xFE00) == 0xFC00
}

/// Link-local addresses: fe80::/10.
fn is_v6_link_local(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();
    (segments[0] & 0xFFC0) == 0xFE80
}

/// Validates a URL for SSRF safety before making a request.
///
/// Rejects:
/// - Non-http(s) schemes (file://, ftp://, etc.)
/// - Hostnames that resolve to private/internal IP addresses
/// - URLs without a valid host
fn validate_url(raw_url: &str) -> Result<(), String> {
    let parsed = Url::parse(raw_url).map_err(|e| format!("invalid URL: {e}"))?;

    // Only allow http and https schemes
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("blocked scheme: {other}:// (only http/https allowed)")),
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    // Determine port (default to 80/443 based on scheme)
    let port = parsed.port_or_known_default().unwrap_or(80);

    // Resolve hostname to IP addresses
    let addr_str = format!("{host}:{port}");
    let addrs: Vec<_> = addr_str
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {host}: {e}"))?
        .collect();

    if addrs.is_empty() {
        return Err(format!("DNS resolution returned no addresses for {host}"));
    }

    // Reject if ANY resolved address is private/internal
    for addr in &addrs {
        if is_private_ip(&addr.ip()) {
            return Err(format!(
                "blocked request to private/internal address: {host} resolves to {}",
                addr.ip()
            ));
        }
    }

    Ok(())
}

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

        // SSRF protection: validate URL scheme and reject private/internal IPs
        if let Err(reason) = validate_url(url) {
            return Ok(SkillResult {
                ok: false,
                output: json!({
                    "error": "SsrfBlocked",
                    "message": reason,
                }),
                preview: format!("SSRF blocked: {reason}"),
            });
        }

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
            ok: (200..400).contains(&status),
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

    // ── SSRF validation tests ──────────────────────────────────────────

    #[test]
    fn is_private_ip_detects_loopback_v4() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(is_private_ip(&ip));
        let ip2: IpAddr = "127.255.255.255".parse().unwrap();
        assert!(is_private_ip(&ip2));
    }

    #[test]
    fn is_private_ip_detects_rfc1918_ranges() {
        // 10.0.0.0/8
        assert!(is_private_ip(&"10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"10.255.255.255".parse::<IpAddr>().unwrap()));
        // 172.16.0.0/12
        assert!(is_private_ip(&"172.16.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"172.31.255.255".parse::<IpAddr>().unwrap()));
        // 192.168.0.0/16
        assert!(is_private_ip(&"192.168.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"192.168.255.255".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn is_private_ip_detects_link_local_v4() {
        // 169.254.0.0/16 (cloud metadata / link-local)
        assert!(is_private_ip(&"169.254.169.254".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"169.254.0.1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn is_private_ip_detects_cgnat_shared() {
        // 100.64.0.0/10
        assert!(is_private_ip(&"100.64.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"100.127.255.255".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn is_private_ip_allows_public_v4() {
        assert!(!is_private_ip(&"8.8.8.8".parse::<IpAddr>().unwrap()));
        assert!(!is_private_ip(&"1.1.1.1".parse::<IpAddr>().unwrap()));
        assert!(!is_private_ip(&"93.184.216.34".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn is_private_ip_detects_loopback_v6() {
        assert!(is_private_ip(&"::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn is_private_ip_detects_unique_local_v6() {
        // fd00::/8
        assert!(is_private_ip(&"fd12:3456:789a::1".parse::<IpAddr>().unwrap()));
        // fc00::/7 broader range
        assert!(is_private_ip(&"fc00::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn is_private_ip_detects_link_local_v6() {
        // fe80::/10
        assert!(is_private_ip(&"fe80::1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn is_private_ip_allows_public_v6() {
        assert!(!is_private_ip(
            &"2607:f8b0:4004:800::200e".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn is_private_ip_detects_unspecified() {
        assert!(is_private_ip(&"0.0.0.0".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"::".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn validate_url_rejects_file_scheme() {
        let result = validate_url("file:///etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("blocked scheme"));
    }

    #[test]
    fn validate_url_rejects_ftp_scheme() {
        let result = validate_url("ftp://example.com/file");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("blocked scheme"));
    }

    #[test]
    fn validate_url_rejects_data_scheme() {
        let result = validate_url("data:text/html,<h1>hi</h1>");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("blocked scheme"));
    }

    #[test]
    fn validate_url_rejects_gopher_scheme() {
        let result = validate_url("gopher://evil.com/");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("blocked scheme"));
    }

    #[test]
    fn validate_url_rejects_localhost() {
        let result = validate_url("http://localhost/admin");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("private") || err.contains("blocked"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_url_rejects_loopback_ip() {
        let result = validate_url("http://127.0.0.1/admin");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private"));
    }

    #[test]
    fn validate_url_rejects_private_rfc1918() {
        assert!(validate_url("http://10.0.0.1/secret").is_err());
        assert!(validate_url("http://172.16.0.1/secret").is_err());
        assert!(validate_url("http://192.168.1.1/secret").is_err());
    }

    #[test]
    fn validate_url_rejects_cloud_metadata() {
        // AWS/GCP/Azure metadata endpoint
        let result = validate_url("http://169.254.169.254/latest/meta-data/");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private"));
    }

    #[test]
    fn validate_url_rejects_ipv6_loopback() {
        let result = validate_url("http://[::1]/admin");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("private") || err.contains("blocked"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_url_rejects_invalid_url() {
        let result = validate_url("not a url at all");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid URL"));
    }

    #[test]
    fn validate_url_rejects_no_host() {
        let result = validate_url("http:///path");
        assert!(result.is_err());
    }
}
