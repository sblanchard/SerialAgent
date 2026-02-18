//! Input validation for schedule fields (URLs, cron expressions, timezones).

/// Validate a URL for safety: must be http(s) and must not target private/internal networks.
///
/// Prevents SSRF by blocking:
/// - Non-http(s) schemes (file://, ftp://, etc.)
/// - Loopback addresses (127.0.0.0/8, ::1)
/// - Private networks (10/8, 172.16/12, 192.168/16)
/// - Link-local addresses (169.254/16 — includes cloud metadata endpoints)
/// - Known metadata hostnames (metadata.google.internal)
/// - Userinfo in URLs (http://evil@internal tricks)
pub fn validate_url(url: &str) -> Result<(), String> {
    use std::net::{Ipv4Addr, Ipv6Addr};

    let lower = url.to_ascii_lowercase();

    // Must use http or https scheme
    let after_scheme = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else {
        return Err("URL must use http or https scheme".into());
    };

    // Reject userinfo (prevent http://evil@internal-host tricks)
    let after_userinfo = match after_scheme.split_once('@') {
        Some((_, rest)) => rest,
        None => after_scheme,
    };

    // Extract authority (before first /)
    let authority = after_userinfo.split('/').next().unwrap_or("");

    // Handle IPv6 bracket notation [::1]:port
    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .unwrap_or("")
            .trim_start_matches('[')
    } else {
        // Strip port
        authority.split(':').next().unwrap_or("")
    };

    if host.is_empty() {
        return Err("URL has empty host".into());
    }

    // Block known dangerous hostnames
    if host == "localhost"
        || host.ends_with(".localhost")
        || host == "metadata.google.internal"
    {
        return Err(format!("URL must not target internal host: {}", host));
    }

    // Check IPv4
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        if ip.is_loopback()
            || ip.is_private()
            || ip.is_link_local()
            || ip.is_unspecified()
            || ip.is_broadcast()
        {
            return Err(format!(
                "URL must not target private/internal IP: {}",
                ip
            ));
        }
    }

    // Check IPv6
    if let Ok(ip) = host.parse::<Ipv6Addr>() {
        if ip.is_loopback() || ip.is_unspecified() {
            return Err(format!(
                "URL must not target private/internal IPv6: {}",
                ip
            ));
        }
        // Check IPv4-mapped IPv6 (::ffff:x.x.x.x)
        let segs = ip.segments();
        if segs[..6] == [0, 0, 0, 0, 0, 0xffff] {
            let mapped = Ipv4Addr::new(
                (segs[6] >> 8) as u8,
                segs[6] as u8,
                (segs[7] >> 8) as u8,
                segs[7] as u8,
            );
            if mapped.is_loopback()
                || mapped.is_private()
                || mapped.is_link_local()
                || mapped.is_unspecified()
            {
                return Err(format!(
                    "URL must not target private/internal IP: {}",
                    mapped
                ));
            }
        }
    }

    Ok(())
}

/// Validate an IANA timezone string.
pub fn validate_timezone(tz: &str) -> Result<(), String> {
    if tz.parse::<chrono_tz::Tz>().is_err() {
        Err(format!(
            "invalid timezone: '{}' — use IANA names like 'America/New_York' or 'UTC'",
            tz
        ))
    } else {
        Ok(())
    }
}

/// Validate a 5-field cron expression. Returns `Ok(())` or an error message.
pub fn validate_cron(cron: &str) -> Result<(), String> {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "expected 5 fields (minute hour dom month dow), got {}",
            fields.len()
        ));
    }
    let names = ["minute", "hour", "day-of-month", "month", "day-of-week"];
    let ranges: [(u32, u32); 5] = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];

    for (i, field) in fields.iter().enumerate() {
        validate_cron_field(field, names[i], ranges[i].0, ranges[i].1)?;
    }
    Ok(())
}

fn validate_cron_field(field: &str, name: &str, min: u32, max: u32) -> Result<(), String> {
    if field == "*" {
        return Ok(());
    }
    if let Some(step) = field.strip_prefix("*/") {
        let n: u32 = step
            .parse()
            .map_err(|_| format!("{}: invalid step '*/{}' — expected a number", name, step))?;
        if n == 0 || n > max {
            return Err(format!("{}: step {} out of range 1..={}", name, n, max));
        }
        return Ok(());
    }
    for part in field.split(',') {
        if let Some((start_s, end_s)) = part.split_once('-') {
            let start: u32 = start_s.parse().map_err(|_| {
                format!("{}: invalid range start '{}'", name, start_s)
            })?;
            let end: u32 = end_s.parse().map_err(|_| {
                format!("{}: invalid range end '{}'", name, end_s)
            })?;
            if start < min || start > max || end < min || end > max {
                return Err(format!(
                    "{}: range {}-{} out of bounds {}..={}",
                    name, start, end, min, max
                ));
            }
            if start > end {
                return Err(format!(
                    "{}: range start {} > end {}",
                    name, start, end
                ));
            }
        } else {
            let n: u32 = part.parse().map_err(|_| {
                format!("{}: invalid value '{}'", name, part)
            })?;
            if n < min || n > max {
                return Err(format!(
                    "{}: value {} out of range {}..={}",
                    name, n, min, max
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cron validation ──────────────────────────────────────────────

    #[test]
    fn validate_cron_accepts_valid() {
        assert!(validate_cron("0 * * * *").is_ok());
        assert!(validate_cron("*/5 9-17 * * 1-5").is_ok());
        assert!(validate_cron("30 9 1,15 * *").is_ok());
        assert!(validate_cron("0 0 * * 0").is_ok());
    }

    #[test]
    fn validate_cron_rejects_invalid() {
        assert!(validate_cron("* * *").is_err());
        assert!(validate_cron("* * * * * *").is_err());
        assert!(validate_cron("60 * * * *").is_err());
        assert!(validate_cron("* 24 * * *").is_err());
        assert!(validate_cron("* * 0 * *").is_err());
        assert!(validate_cron("* * * 13 *").is_err());
        assert!(validate_cron("* * * * 7").is_err());
        assert!(validate_cron("*/0 * * * *").is_err());
        assert!(validate_cron("abc * * * *").is_err());
    }

    // ── URL validation (SSRF prevention) ────────────────────────────

    #[test]
    fn validate_url_accepts_valid() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/path?q=1").is_ok());
        assert!(validate_url("https://8.8.8.8/dns").is_ok());
        assert!(validate_url("https://sub.domain.com:8443/api").is_ok());
    }

    #[test]
    fn validate_url_rejects_non_http() {
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("gopher://evil.com").is_err());
    }

    #[test]
    fn validate_url_rejects_private_ips() {
        assert!(validate_url("http://127.0.0.1").is_err());
        assert!(validate_url("http://127.0.0.1:8080/api").is_err());
        assert!(validate_url("http://10.0.0.1").is_err());
        assert!(validate_url("http://172.16.0.1").is_err());
        assert!(validate_url("http://192.168.1.1").is_err());
        assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_url("http://0.0.0.0").is_err());
    }

    #[test]
    fn validate_url_rejects_localhost() {
        assert!(validate_url("http://localhost").is_err());
        assert!(validate_url("http://localhost:3000").is_err());
        assert!(validate_url("https://app.localhost/api").is_err());
    }

    #[test]
    fn validate_url_rejects_metadata_hosts() {
        assert!(validate_url("http://metadata.google.internal").is_err());
    }

    #[test]
    fn validate_url_rejects_ipv6_loopback() {
        assert!(validate_url("http://[::1]").is_err());
        assert!(validate_url("http://[::1]:8080/path").is_err());
    }

    #[test]
    fn validate_url_rejects_empty_host() {
        assert!(validate_url("http://").is_err());
        assert!(validate_url("http:///path").is_err());
    }

    // ── Timezone validation ──────────────────────────────────────────

    #[test]
    fn validate_timezone_accepts_valid() {
        assert!(validate_timezone("UTC").is_ok());
        assert!(validate_timezone("America/New_York").is_ok());
        assert!(validate_timezone("Europe/London").is_ok());
        assert!(validate_timezone("Asia/Tokyo").is_ok());
    }

    #[test]
    fn validate_timezone_rejects_invalid() {
        assert!(validate_timezone("Not/Real").is_err());
        assert!(validate_timezone("").is_err());
        assert!(validate_timezone("GMT+5").is_err());
        assert!(validate_timezone("FakeZone").is_err());
    }
}
