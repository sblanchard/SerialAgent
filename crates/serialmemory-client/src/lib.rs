//! `sa-memory` — SerialMemory client crate for SerialAgent.
//!
//! Provides the [`SerialMemoryProvider`] trait that abstracts over the
//! SerialMemoryServer API, a production REST implementation
//! ([`RestSerialMemoryClient`]), an MCP implementation
//! ([`McpSerialMemoryClient`]), typed DTOs matching the OpenAPI schema,
//! and a [`UserFactsBuilder`] that assembles the USER_FACTS context
//! section from persona + search results.
//!
//! # Transport selection
//!
//! Use [`create_provider`] to build the right implementation based on
//! the `serial_memory.transport` config field:
//!
//! | Transport | Implementation          | Best for                        |
//! |-----------|-------------------------|---------------------------------|
//! | `rest`    | `RestSerialMemoryClient` | Gateway hot path (default)      |
//! | `mcp`     | `McpSerialMemoryClient`  | Dev tooling, CLI, interop       |
//! | `hybrid`  | `RestSerialMemoryClient` | REST primary, MCP documented    |
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

pub mod mcp;
pub mod provider;
pub mod rest;
pub mod types;
pub mod user_facts;

// ── Re-exports for ergonomic imports ─────────────────────────────────

pub use mcp::McpSerialMemoryClient;
pub use provider::SerialMemoryProvider;
pub use rest::{from_reqwest, RestSerialMemoryClient};
pub use types::{
    IngestResponse, MemoryIngestRequest, RagAnswerRequest, RagAnswerResponse, RagSearchRequest,
    RagSearchResponse, RetrievedMemoryDto, SessionRequest, UserPersonaRequest,
};
pub use user_facts::UserFactsBuilder;

use std::sync::Arc;

use sa_domain::config::{SerialMemoryConfig, SmTransport};
use sa_domain::error::Result;

/// Create the appropriate [`SerialMemoryProvider`] based on the transport
/// config.
///
/// | `transport` | Result                                               |
/// |-------------|------------------------------------------------------|
/// | `rest`      | [`RestSerialMemoryClient`]                           |
/// | `mcp`       | [`McpSerialMemoryClient`]                            |
/// | `hybrid`    | [`RestSerialMemoryClient`] (REST primary; MCP ready) |
///
/// # Hybrid failure semantics
///
/// In `hybrid` mode the deterministic behavior is **REST-primary, no
/// fallback**.  All reads and writes go through the REST transport.
/// The MCP endpoint is documented and available for *external consumers*
/// (CLI tooling, MCP-native clients) but the gateway itself never falls
/// back to MCP on a REST failure.  This avoids ambiguous dual-write /
/// split-brain scenarios that are painful to debug.
///
/// If you need true dual-transport with automatic failover, implement a
/// dedicated `FallbackProvider` wrapper that retries on the secondary
/// transport — but keep the policy explicit (e.g. "retry reads on MCP,
/// never retry writes").
pub fn create_provider(cfg: &SerialMemoryConfig) -> Result<Arc<dyn SerialMemoryProvider>> {
    match cfg.transport {
        SmTransport::Rest | SmTransport::Hybrid => {
            let client = RestSerialMemoryClient::new(cfg)?;
            if cfg.transport == SmTransport::Hybrid {
                tracing::info!(
                    mcp_endpoint = ?cfg.mcp_endpoint,
                    "hybrid mode: REST is primary transport (no MCP fallback); \
                     MCP endpoint documented for external consumers"
                );
            }
            Ok(Arc::new(client))
        }
        SmTransport::Mcp => {
            let client = McpSerialMemoryClient::new(cfg)?;
            tracing::info!(
                mcp_url = ?cfg.mcp_endpoint,
                "using MCP transport for SerialMemory"
            );
            Ok(Arc::new(client))
        }
    }
}
