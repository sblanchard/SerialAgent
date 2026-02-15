use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Memory search ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    #[serde(default = "default_search_mode")]
    pub mode: String,
    #[serde(default = "default_search_limit")]
    pub limit: u32,
    #[serde(default = "default_search_threshold")]
    pub threshold: f64,
    #[serde(default = "default_true")]
    pub include_entities: bool,
    #[serde(default)]
    pub memory_type: Option<String>,
}

fn default_search_mode() -> String {
    "hybrid".into()
}
fn default_search_limit() -> u32 {
    10
}
fn default_search_threshold() -> f64 {
    0.7
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub content: String,
    pub created_at: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub similarity: f64,
    #[serde(default)]
    pub rank: f64,
    #[serde(default)]
    pub entities: Vec<EntityRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRef {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
}

// ── Memory ingest ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    pub content: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(default = "default_true")]
    pub extract_entities: bool,
    #[serde(default = "default_dedup_mode")]
    pub dedup_mode: String,
    #[serde(default = "default_dedup_threshold")]
    pub dedup_threshold: f64,
    #[serde(default = "default_memory_type")]
    pub memory_type: String,
}

fn default_dedup_mode() -> String {
    "warn".into()
}
fn default_dedup_threshold() -> f64 {
    0.85
}
fn default_memory_type() -> String {
    "knowledge".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResponse {
    pub memory_id: String,
    #[serde(default)]
    pub entities_created: u32,
    #[serde(default)]
    pub relationships_created: u32,
    #[serde(default)]
    pub entities: Vec<ExtractedEntity>,
    #[serde(default)]
    pub relationships: Vec<ExtractedRelationship>,
    #[serde(default)]
    pub duplicate_detected: bool,
    #[serde(default)]
    pub duplicate_of: Option<String>,
    #[serde(default)]
    pub duplicate_similarity: f64,
    #[serde(default)]
    pub similar_memories: Vec<SimilarMemory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    #[serde(default)]
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelationship {
    pub source_id: String,
    pub target_id: String,
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub relationship_type: String,
    #[serde(default)]
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarMemory {
    pub memory_id: String,
    pub similarity: f64,
    #[serde(default)]
    pub content_preview: Option<String>,
}

// ── User profile ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    #[serde(default)]
    pub preferences: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub skills: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub goals: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub background: HashMap<String, serde_json::Value>,
}

// ── Multi-hop search ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiHopQuery {
    pub query: String,
    #[serde(default = "default_hops")]
    pub hops: u32,
    #[serde(default = "default_max_per_hop")]
    pub max_results_per_hop: u32,
}

fn default_hops() -> u32 {
    2
}
fn default_max_per_hop() -> u32 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiHopResult {
    pub hops: u32,
    #[serde(default)]
    pub memories: Vec<SearchResult>,
    #[serde(default)]
    pub entities: Vec<EntityRef>,
    #[serde(default)]
    pub relationships: Vec<ExtractedRelationship>,
}

// ── Context instantiation ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRequest {
    pub project_or_subject: String,
    #[serde(default = "default_days_back")]
    pub days_back: u32,
    #[serde(default = "default_context_limit")]
    pub limit: u32,
    #[serde(default = "default_true")]
    pub include_entities: bool,
}

fn default_days_back() -> u32 {
    3
}
fn default_context_limit() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResponse {
    #[serde(default)]
    pub from_date: Option<String>,
    #[serde(default)]
    pub to_date: Option<String>,
    #[serde(default)]
    pub project_or_subject: Option<String>,
    #[serde(default)]
    pub memory_count: u32,
    #[serde(default)]
    pub recent_memory_count: u32,
    #[serde(default)]
    pub context_memory_count: u32,
    #[serde(default)]
    pub session_summary: Option<String>,
    #[serde(default)]
    pub memories: Vec<SearchResult>,
    #[serde(default)]
    pub top_entities: Vec<EntityRef>,
    #[serde(default)]
    pub top_relationships: Vec<ExtractedRelationship>,
}

// ── Memory lifecycle ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRequest {
    pub memory_id: String,
    pub new_content: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRequest {
    pub memory_id: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub superseded_by_id: Option<String>,
}

// ── Session management ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitSessionRequest {
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default)]
    pub client_type: Option<String>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitSessionResponse {
    pub session_id: String,
    pub started_at: String,
}

// ── RAG ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagSearchRequest {
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagAnswerRequest {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagAnswerResponse {
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub sources: Vec<SearchResult>,
}
