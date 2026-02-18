//! Schedule store and runner — cron-based job scheduling that creates Runs.
//!
//! Schedules are persisted to `data/schedules.json`. The runner ticks every
//! 30 seconds and triggers runs for any due schedules.
//!
//! Split into submodules for maintainability:
//! - [`model`] — Data types, enums, config structs
//! - [`cron`] — Timezone-aware cron evaluation
//! - [`validation`] — Input validation (URLs, cron, timezones)
//! - [`store`] — Persistent `ScheduleStore` with event broadcasting

pub mod cron;
pub mod model;
pub mod store;
pub mod validation;

// Re-export the public API so existing `use crate::runtime::schedules::X` imports still work.
pub use cron::{cron_matches, cron_next, cron_next_n, cron_next_n_tz, cron_next_tz, parse_tz};
pub use model::{
    cooldown_minutes, DeliveryTarget, DigestMode, FetchConfig, MissedPolicy, Schedule,
    ScheduleEvent, ScheduleStatus, ScheduleView, SourceState,
};
pub use store::ScheduleStore;
pub use validation::{validate_cron, validate_timezone, validate_url};
