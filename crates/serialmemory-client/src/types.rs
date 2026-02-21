//! Data Transfer Objects matching the real SerialMemoryServer OpenAPI schema.
//!
//! Field names use `camelCase` on the wire (matching the .NET API) and
//! `snake_case` in Rust code via `#[serde(rename_all = "camelCase")]`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// RAG search
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// POST /api/rag/search — request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSearchRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Minimum similarity threshold (0.0-1.0). SerialMemory defaults to 0.7
    /// which is often too strict. We default to 0.3 for broader recall.
    #[serde(default = "default_threshold")]
    pub threshold: f64,
}

fn default_threshold() -> f64 {
    0.3
}

impl Default for RagSearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            limit: None,
            threshold: default_threshold(),
        }
    }
}

/// POST /api/rag/search — response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagSearchResponse {
    pub query: String,
    pub memories: Vec<RetrievedMemoryDto>,
    pub count: u32,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// RAG answer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// POST /api/rag/answer — request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagAnswerRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_memories: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_l1: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_l3: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_l4: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_reasoning_trace: Option<bool>,
}

/// POST /api/rag/answer — response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagAnswerResponse {
    pub answer: String,
    #[serde(default)]
    pub query_id: Option<String>,
    #[serde(default)]
    pub memories: Vec<RetrievedMemoryDto>,
    #[serde(default)]
    pub reasoning_trace: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Retrieved memory DTO (shared by search & answer)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedMemoryDto {
    #[serde(default)]
    pub id: Option<String>,
    pub content: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub similarity: Option<f64>,
    #[serde(default)]
    pub rank: Option<f64>,
    /// Timestamp string from SerialMemory (may lack timezone suffix).
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub entities: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub layer: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Memory ingest
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// POST /api/memories — request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract_entities: Option<bool>,
}

/// POST /api/memories — response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestResponse {
    #[serde(alias = "memory_id")]
    pub memory_id: String,
    #[serde(default, alias = "entitiesCreated")]
    pub entities_extracted: Option<u32>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Persona
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// POST /api/persona — request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPersonaRequest {
    pub attribute_type: String,
    pub attribute_key: String,
    pub attribute_value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sessions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// POST /api/sessions — request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRequest {
    pub session_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_type: Option<String>,
}
