use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Context pack caps
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "d_20000")]
    pub bootstrap_max_chars: usize,
    #[serde(default = "d_24000")]
    pub bootstrap_total_max_chars: usize,
    #[serde(default = "d_4000")]
    pub user_facts_max_chars: usize,
    #[serde(default = "d_2000")]
    pub skills_index_max_chars: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            bootstrap_max_chars: 20_000,
            bootstrap_total_max_chars: 24_000,
            user_facts_max_chars: 4_000,
            skills_index_max_chars: 2_000,
        }
    }
}

// ── serde default helpers ───────────────────────────────────────────

fn d_20000() -> usize {
    20_000
}
fn d_24000() -> usize {
    24_000
}
fn d_4000() -> usize {
    4_000
}
fn d_2000() -> usize {
    2_000
}
