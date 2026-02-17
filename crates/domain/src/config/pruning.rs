use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Context pruning (OpenClaw cache-ttl model)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Context pruning configuration — trims oversized tool results before
/// sending to the LLM, following OpenClaw's `cache-ttl` model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruningConfig {
    /// Pruning mode.
    #[serde(default)]
    pub mode: PruningMode,
    /// TTL in seconds; if the last LLM call for this session was within
    /// the TTL, skip pruning (the cache is still warm).
    #[serde(default = "d_300")]
    pub ttl_seconds: u64,
    /// Number of recent assistant messages whose tool results are protected.
    #[serde(default = "d_3u")]
    pub keep_last_assistants: usize,
    /// Only prune tool results longer than this many chars.
    #[serde(default = "d_50000")]
    pub min_prunable_chars: usize,
    /// Ratio of context window at which soft-trim activates.
    #[serde(default = "d_03")]
    pub soft_trim_ratio: f64,
    /// Ratio of context window at which hard-clear activates.
    #[serde(default = "d_05")]
    pub hard_clear_ratio: f64,
    #[serde(default)]
    pub soft_trim: SoftTrimConfig,
    #[serde(default)]
    pub hard_clear: HardClearConfig,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            mode: PruningMode::Off,
            ttl_seconds: 300,
            keep_last_assistants: 3,
            min_prunable_chars: 50_000,
            soft_trim_ratio: 0.3,
            hard_clear_ratio: 0.5,
            soft_trim: SoftTrimConfig::default(),
            hard_clear: HardClearConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PruningMode {
    #[default]
    Off,
    CacheTtl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoftTrimConfig {
    /// Max chars to keep total after trimming.
    #[serde(default = "d_4000u")]
    pub max_chars: usize,
    /// Chars to keep from the head.
    #[serde(default = "d_1500")]
    pub head_chars: usize,
    /// Chars to keep from the tail.
    #[serde(default = "d_1500")]
    pub tail_chars: usize,
}

impl Default for SoftTrimConfig {
    fn default() -> Self {
        Self {
            max_chars: 4_000,
            head_chars: 1_500,
            tail_chars: 1_500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardClearConfig {
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default = "d_placeholder")]
    pub placeholder: String,
}

impl Default for HardClearConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            placeholder: d_placeholder(),
        }
    }
}

// ── serde default helpers ───────────────────────────────────────────

fn d_300() -> u64 {
    300
}
fn d_3u() -> usize {
    3
}
fn d_50000() -> usize {
    50_000
}
fn d_03() -> f64 {
    0.3
}
fn d_05() -> f64 {
    0.5
}
fn d_4000u() -> usize {
    4_000
}
fn d_1500() -> usize {
    1_500
}
fn d_placeholder() -> String {
    "[Old tool result content cleared]".into()
}
fn d_true() -> bool {
    true
}
