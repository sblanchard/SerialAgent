use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Compaction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Compaction collapses old conversation history into a summary so the
/// context window doesn't overflow after many turns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Enable automatic compaction when turn count exceeds `max_turns`.
    #[serde(default = "d_true")]
    pub auto: bool,
    /// Maximum turns (user messages) before auto-compaction triggers.
    #[serde(default = "d_80")]
    pub max_turns: usize,
    /// Number of recent turns to keep verbatim after compaction.
    #[serde(default = "d_12")]
    pub keep_last_turns: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            auto: true,
            max_turns: 80,
            keep_last_turns: 12,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Memory lifecycle
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Controls automatic memory capture — the always-on behaviour that
/// makes the agent feel alive across sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryLifecycleConfig {
    /// Automatically capture each turn to long-term memory.
    #[serde(default = "d_true")]
    pub auto_capture: bool,
    /// Ingest a session summary to memory when compaction runs.
    #[serde(default = "d_true")]
    pub capture_on_compaction: bool,
}

impl Default for MemoryLifecycleConfig {
    fn default() -> Self {
        Self {
            auto_capture: true,
            capture_on_compaction: true,
        }
    }
}

// ── serde default helpers ───────────────────────────────────────────

fn d_true() -> bool {
    true
}
fn d_80() -> usize {
    80
}
fn d_12() -> usize {
    12
}
