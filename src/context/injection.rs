/// Format a workspace file section with machine-inspectable delimiters.
///
/// Produces:
/// ```text
/// === WORKSPACE_FILE: SOUL.md ===
/// RAW_CHARS: 31240
/// INJECTED_CHARS: 20014
/// TRUNCATED_PER_FILE: true
/// TRUNCATED_TOTAL_CAP: false
/// --- BEGIN ---
/// <content…>
/// --- END ---
/// ```
pub fn format_workspace_section(
    filename: &str,
    content: &str,
    raw_chars: usize,
    truncated_per_file: bool,
    truncated_total_cap: bool,
) -> String {
    let injected_chars = content.len();
    format!(
        "\
=== WORKSPACE_FILE: {filename} ===
RAW_CHARS: {raw_chars}
INJECTED_CHARS: {injected_chars}
TRUNCATED_PER_FILE: {truncated_per_file}
TRUNCATED_TOTAL_CAP: {truncated_total_cap}
--- BEGIN ---
{content}
--- END ---
"
    )
}

/// Format the compact skills index section.
pub fn format_skills_index(index_content: &str) -> String {
    format!(
        "\
=== SKILLS_INDEX ===
{index_content}
=== END_SKILLS_INDEX ===
"
    )
}

/// Format the USER_FACTS section (learned facts from SerialMemory).
pub fn format_user_facts(facts_content: &str) -> String {
    format!(
        "\
=== USER_FACTS ===
{facts_content}
=== END_USER_FACTS ===
"
    )
}
