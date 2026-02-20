//! Session management for SerialAgent.
//!
//! Implements the OpenClaw `sessionKey` model: stable session routing from
//! inbound metadata, identity linking across channels, gateway-owned session
//! state with append-only transcripts, and configurable reset lifecycle.

pub mod identity;
pub mod lifecycle;
pub mod search;
pub mod session_key;
pub mod store;
pub mod transcript;

pub use identity::IdentityResolver;
pub use lifecycle::LifecycleManager;
pub use search::{SearchHit, TranscriptIndex};
pub use session_key::{compute_session_key, validate_metadata, SessionKeyValidation};
pub use store::{SessionEntry, SessionStore};
pub use transcript::TranscriptWriter;
