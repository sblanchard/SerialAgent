//! `sa-memory` — SerialMemory client crate for SerialAgent.
//!
//! Provides the [`SerialMemoryProvider`] trait that abstracts over the
//! SerialMemoryServer API, a production REST implementation
//! ([`RestSerialMemoryClient`]), typed DTOs matching the OpenAPI schema,
//! and a [`UserFactsBuilder`] that assembles the USER_FACTS context
//! section from persona + search results.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use sa_domain::config::SerialMemoryConfig;
//! use sa_memory::{RestSerialMemoryClient, SerialMemoryProvider, RagSearchRequest};
//!
//! # async fn example() -> sa_domain::error::Result<()> {
//! let cfg = SerialMemoryConfig::default();
//! let client = RestSerialMemoryClient::new(&cfg)?;
//!
//! let results = client
//!     .search(RagSearchRequest {
//!         query: "user's favourite language".into(),
//!         limit: Some(5),
//!     })
//!     .await?;
//!
//! println!("found {} memories", results.count);
//! # Ok(())
//! # }
//! ```

pub mod provider;
pub mod rest;
pub mod types;
pub mod user_facts;

// ── Re-exports for ergonomic imports ─────────────────────────────────

pub use provider::SerialMemoryProvider;
pub use rest::{from_reqwest, RestSerialMemoryClient};
pub use types::{
    IngestResponse, MemoryIngestRequest, RagAnswerRequest, RagAnswerResponse, RagSearchRequest,
    RagSearchResponse, RetrievedMemoryDto, SessionRequest, UserPersonaRequest,
};
pub use user_facts::UserFactsBuilder;
