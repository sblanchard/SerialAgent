/// A section being accumulated for total-cap processing.
pub struct Section {
    pub filename: String,
    pub content: String,
    pub raw_chars: usize,
    pub truncated_per_file: bool,
    pub truncated_total_cap: bool,
    pub included: bool,
    pub missing: bool,
}

/// Per-file truncation.
///
/// If `content` exceeds `max_chars`, truncate to the first `max_chars` characters
/// (at a valid UTF-8 boundary) and append `\n\n[TRUNCATED]\n`.
pub fn truncate_per_file(content: &str, max_chars: usize) -> (String, bool) {
    if content.len() <= max_chars {
        return (content.to_string(), false);
    }
    let boundary = content.floor_char_boundary(max_chars);
    let mut result = content[..boundary].to_string();
    result.push_str("\n\n[TRUNCATED]\n");
    (result, true)
}

/// Apply total cap across accumulated sections in order.
pub fn apply_total_cap(sections: &mut [Section], total_max_chars: usize) {
    let mut accumulated: usize = 0;

    for section in sections.iter_mut() {
        if !section.included {
            continue;
        }

        let section_len = section.content.len();

        if accumulated + section_len <= total_max_chars {
            accumulated += section_len;
        } else if accumulated < total_max_chars {
            let remaining = total_max_chars - accumulated;
            let boundary = section.content.floor_char_boundary(remaining);
            section.content = format!(
                "{}\n\n[TRUNCATED_TOTAL_CAP]\n",
                &section.content[..boundary]
            );
            section.truncated_total_cap = true;
            accumulated = total_max_chars;
        } else {
            section.content.clear();
            section.included = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_truncation_when_under_limit() {
        let (result, truncated) = truncate_per_file("hello world", 100);
        assert_eq!(result, "hello world");
        assert!(!truncated);
    }

    #[test]
    fn truncates_at_limit() {
        let content = "abcdefghij";
        let (result, truncated) = truncate_per_file(content, 5);
        assert!(truncated);
        assert!(result.starts_with("abcde"));
        assert!(result.contains("[TRUNCATED]"));
    }

    #[test]
    fn total_cap_excludes_overflow() {
        let mut sections = vec![
            Section {
                filename: "A.md".into(),
                content: "aaaa".into(),
                raw_chars: 4,
                truncated_per_file: false,
                truncated_total_cap: false,
                included: true,
                missing: false,
            },
            Section {
                filename: "B.md".into(),
                content: "bbbbbb".into(),
                raw_chars: 6,
                truncated_per_file: false,
                truncated_total_cap: false,
                included: true,
                missing: false,
            },
            Section {
                filename: "C.md".into(),
                content: "cccc".into(),
                raw_chars: 4,
                truncated_per_file: false,
                truncated_total_cap: false,
                included: true,
                missing: false,
            },
        ];

        apply_total_cap(&mut sections, 8);

        assert!(sections[0].included);
        assert!(!sections[0].truncated_total_cap);
        assert!(sections[1].included);
        assert!(sections[1].truncated_total_cap);
        assert!(!sections[2].included);
    }
}
