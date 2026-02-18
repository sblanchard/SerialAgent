//! The `SerialMemoryProvider` trait defines the interface for all
//! SerialMemory backends (REST, MCP, hybrid, mock/test).

use async_trait::async_trait;
use sa_domain::error::Result;

use crate::types::{
    IngestResponse, MemoryIngestRequest, RagAnswerRequest, RagAnswerResponse, RagSearchRequest,
    RagSearchResponse, SessionRequest, UserPersonaRequest,
};

/// Abstraction over the SerialMemoryServer API surface.
///
/// Implementations may talk to the real REST API, an MCP bridge, or a
/// test double. All methods return `sa_domain::error::Result`.
#[async_trait]
pub trait SerialMemoryProvider: Send + Sync {
    /// Semantic search across the memory graph (POST /api/rag/search).
    async fn search(&self, req: RagSearchRequest) -> Result<RagSearchResponse>;

    /// RAG-powered answer grounded in the user's memories (POST /api/rag/answer).
    async fn answer(&self, req: RagAnswerRequest) -> Result<RagAnswerResponse>;

    /// Ingest a new memory (POST /api/memories).
    async fn ingest(&self, req: MemoryIngestRequest) -> Result<IngestResponse>;

    /// Fetch the user persona (GET /api/persona).
    async fn get_persona(&self) -> Result<serde_json::Value>;

    /// Set / update a persona attribute (POST /api/persona).
    async fn set_persona(&self, req: UserPersonaRequest) -> Result<()>;

    /// Initialize a new session (POST /api/sessions).
    async fn init_session(&self, req: SessionRequest) -> Result<serde_json::Value>;

    /// End an active session (POST /api/sessions/{sessionId}/end).
    async fn end_session(&self, session_id: &str) -> Result<()>;

    /// Retrieve the knowledge graph neighbourhood (GET /api/graph).
    async fn graph(&self, hops: u32, limit: u32) -> Result<serde_json::Value>;

    /// Fetch memory/entity/relationship statistics (GET /api/stats).
    async fn stats(&self) -> Result<serde_json::Value>;

    /// Health check (GET /admin/health + GET /admin/status).
    async fn health(&self) -> Result<serde_json::Value>;

    /// Update an existing memory (PATCH /api/memories/{id}).
    async fn update_memory(&self, id: &str, content: &str) -> Result<serde_json::Value>;

    /// Delete a memory (DELETE /api/memories/{id}).
    async fn delete_memory(&self, id: &str) -> Result<()>;
}
