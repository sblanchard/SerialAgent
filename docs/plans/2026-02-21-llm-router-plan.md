# LLM Router Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an embedding-based smart router to SerialAgent that classifies prompts (~10ms) and routes them to appropriate model tiers (Simple/Complex/Reasoning/Free) automatically.

**Architecture:** Extend the existing `sa-providers` crate with a `SmartRouter` that wraps `ProviderRegistry`. The router uses a lightweight embedding model (default: Ollama nomic-embed-text) to compute cosine similarity against reference prompt embeddings per tier. Routing profiles (Auto/Eco/Premium/Free/Reasoning) determine tier selection strategy. The `resolve_provider()` function in the gateway wires it in.

**Tech Stack:** Rust (sa-providers crate extension), Vue 3 + TypeScript (dashboard), Axum (API endpoints), Ollama (default embedding provider)

**Design Doc:** `docs/plans/2026-02-21-llm-router-design.md`

---

## Task 1: Router Config Types in `sa-domain`

**Files:**
- Modify: `crates/domain/src/config/llm.rs`

**Step 1: Write the failing test**

Add tests that deserialize the new router config from JSON.

```rust
// At the bottom of the existing #[cfg(test)] mod tests { ... } block

#[test]
fn router_config_deserializes() {
    let json = r#"{
        "router": {
            "enabled": true,
            "default_profile": "auto",
            "classifier": {
                "provider": "ollama",
                "model": "nomic-embed-text",
                "endpoint": "http://localhost:11434",
                "cache_ttl_secs": 300
            },
            "tiers": {
                "simple": ["deepseek/deepseek-chat"],
                "complex": ["anthropic/claude-sonnet-4-20250514"],
                "reasoning": ["anthropic/claude-opus-4-6"],
                "free": ["venice/venice-uncensored"]
            },
            "thresholds": {
                "simple_min_score": 0.6,
                "complex_min_score": 0.5,
                "reasoning_min_score": 0.55,
                "escalate_token_threshold": 8000
            }
        }
    }"#;
    let config: LlmConfig = serde_json::from_str(json).unwrap();
    let router = config.router.unwrap();
    assert!(router.enabled);
    assert_eq!(router.default_profile, RoutingProfile::Auto);
    assert_eq!(router.classifier.model, "nomic-embed-text");
    assert_eq!(router.tiers.simple.len(), 1);
    assert!((router.thresholds.simple_min_score - 0.6).abs() < 1e-10);
}

#[test]
fn router_config_defaults_when_absent() {
    let json = r#"{}"#;
    let config: LlmConfig = serde_json::from_str(json).unwrap();
    assert!(config.router.is_none());
}

#[test]
fn routing_profile_serde_roundtrip() {
    for profile in &["auto", "eco", "premium", "free", "reasoning"] {
        let json = format!("\"{}\"", profile);
        let parsed: RoutingProfile = serde_json::from_str(&json).unwrap();
        let back = serde_json::to_string(&parsed).unwrap();
        assert_eq!(back, json);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-domain -- router_config`
Expected: FAIL — `RouterConfig`, `RoutingProfile`, etc. don't exist yet

**Step 3: Write minimal implementation**

Add these types to `crates/domain/src/config/llm.rs`, above the existing tests:

```rust
/// Routing profile determines how the smart router selects a model tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RoutingProfile {
    #[default]
    Auto,
    Eco,
    Premium,
    Free,
    Reasoning,
}

/// Model tier for router classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    Simple,
    Complex,
    Reasoning,
    Free,
}

/// Smart router configuration (optional section under [llm]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub default_profile: RoutingProfile,
    #[serde(default)]
    pub classifier: ClassifierConfig,
    #[serde(default)]
    pub tiers: TierConfig,
    #[serde(default)]
    pub thresholds: RouterThresholds,
}

/// Embedding classifier configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierConfig {
    #[serde(default = "d_ollama")]
    pub provider: String,
    #[serde(default = "d_nomic")]
    pub model: String,
    #[serde(default = "d_ollama_endpoint")]
    pub endpoint: String,
    #[serde(default = "d_300")]
    pub cache_ttl_secs: u64,
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".into(),
            model: "nomic-embed-text".into(),
            endpoint: "http://localhost:11434".into(),
            cache_ttl_secs: 300,
        }
    }
}

/// Per-tier ordered list of `provider/model` strings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TierConfig {
    #[serde(default)]
    pub simple: Vec<String>,
    #[serde(default)]
    pub complex: Vec<String>,
    #[serde(default)]
    pub reasoning: Vec<String>,
    #[serde(default)]
    pub free: Vec<String>,
}

/// Cosine similarity thresholds for the classifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterThresholds {
    #[serde(default = "d_0_6")]
    pub simple_min_score: f64,
    #[serde(default = "d_0_5")]
    pub complex_min_score: f64,
    #[serde(default = "d_0_55")]
    pub reasoning_min_score: f64,
    #[serde(default = "d_8000")]
    pub escalate_token_threshold: usize,
}

impl Default for RouterThresholds {
    fn default() -> Self {
        Self {
            simple_min_score: 0.6,
            complex_min_score: 0.5,
            reasoning_min_score: 0.55,
            escalate_token_threshold: 8000,
        }
    }
}

// Serde default helpers for router config
fn d_ollama() -> String { "ollama".into() }
fn d_nomic() -> String { "nomic-embed-text".into() }
fn d_ollama_endpoint() -> String { "http://localhost:11434".into() }
fn d_300() -> u64 { 300 }
fn d_0_6() -> f64 { 0.6 }
fn d_0_5() -> f64 { 0.5 }
fn d_0_55() -> f64 { 0.55 }
fn d_8000() -> usize { 8000 }
```

Add the `router` field to `LlmConfig`:

```rust
// Inside the LlmConfig struct, add:
#[serde(default)]
pub router: Option<RouterConfig>,
```

And update `LlmConfig::default()`:

```rust
// Add to Default impl:
router: None,
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p sa-domain -- router_config`
Expected: PASS

Run: `cargo test -p sa-domain`
Expected: All existing tests PASS

**Step 5: Commit**

```bash
git add crates/domain/src/config/llm.rs
git commit -m "feat(domain): add RouterConfig, RoutingProfile, ModelTier types"
```

---

## Task 2: Embedding Classifier in `sa-providers`

**Files:**
- Create: `crates/providers/src/classifier.rs`
- Modify: `crates/providers/src/lib.rs` (add `pub mod classifier;`)
- Modify: `crates/providers/Cargo.toml` (no new deps needed — `reqwest` already present)

**Step 1: Write the failing test**

Create `crates/providers/src/classifier.rs` with tests that exercise cosine similarity, centroid computation, and classification using pre-computed mock embeddings (no Ollama needed):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn mock_embeddings() -> HashMap<ModelTier, Vec<Vec<f32>>> {
        // Simple tier: vectors near [1, 0, 0]
        // Complex tier: vectors near [0, 1, 0]
        // Reasoning tier: vectors near [0, 0, 1]
        let mut m = HashMap::new();
        m.insert(ModelTier::Simple, vec![
            vec![0.9, 0.1, 0.0],
            vec![1.0, 0.0, 0.1],
        ]);
        m.insert(ModelTier::Complex, vec![
            vec![0.1, 0.9, 0.0],
            vec![0.0, 1.0, 0.1],
        ]);
        m.insert(ModelTier::Reasoning, vec![
            vec![0.0, 0.1, 0.9],
            vec![0.1, 0.0, 1.0],
        ]);
        m
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn compute_centroid_single_vector() {
        let vecs = vec![vec![1.0, 2.0, 3.0]];
        let centroid = compute_centroid(&vecs);
        assert_eq!(centroid, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn compute_centroid_average() {
        let vecs = vec![
            vec![0.0, 0.0, 2.0],
            vec![0.0, 0.0, 4.0],
        ];
        let centroid = compute_centroid(&vecs);
        assert!((centroid[2] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn classify_with_centroids_picks_nearest() {
        let embs = mock_embeddings();
        let centroids = build_centroids(&embs);

        // A vector near [1, 0, 0] should classify as Simple
        let simple_vec = vec![0.95, 0.05, 0.0];
        let (tier, scores) = classify_against_centroids(&simple_vec, &centroids);
        assert_eq!(tier, ModelTier::Simple);
        assert!(scores[&ModelTier::Simple] > scores[&ModelTier::Complex]);

        // A vector near [0, 0, 1] should classify as Reasoning
        let reasoning_vec = vec![0.0, 0.05, 0.95];
        let (tier, _) = classify_against_centroids(&reasoning_vec, &centroids);
        assert_eq!(tier, ModelTier::Reasoning);
    }

    #[test]
    fn classify_ambiguous_defaults_to_complex() {
        let centroids = HashMap::new(); // empty = no match
        let vec = vec![0.5, 0.5, 0.5];
        let (tier, _) = classify_against_centroids(&vec, &centroids);
        assert_eq!(tier, ModelTier::Complex); // safe default
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-providers -- classifier`
Expected: FAIL — module doesn't exist

**Step 3: Write minimal implementation**

```rust
//! Embedding-based prompt classifier.
//!
//! Classifies prompts into model tiers (Simple/Complex/Reasoning) by comparing
//! their embeddings against reference centroids using cosine similarity.

use sa_domain::config::{ClassifierConfig, ModelTier, RouterThresholds};
use sa_domain::error::{Error, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use parking_lot::RwLock;

/// Reference prompts used to build tier centroids at startup.
pub fn default_reference_prompts() -> HashMap<ModelTier, Vec<&'static str>> {
    let mut m = HashMap::new();
    m.insert(ModelTier::Simple, vec![
        "What time is it?",
        "Summarize this text",
        "Translate to French",
        "List the files",
        "What is the weather?",
        "Convert this to JSON",
        "Count the words",
        "What is 2 + 2?",
    ]);
    m.insert(ModelTier::Complex, vec![
        "Analyze the performance bottleneck in this code",
        "Compare three architectural approaches for this system",
        "Debug this race condition in the async handler",
        "Write a comprehensive test suite for the auth module",
        "Refactor this module to use the strategy pattern",
        "Review this pull request for security vulnerabilities",
        "Explain the tradeoffs between these database designs",
        "Optimize this SQL query that scans millions of rows",
    ]);
    m.insert(ModelTier::Reasoning, vec![
        "Plan the implementation of a distributed cache with invalidation",
        "Prove this algorithm is O(n log n) in the worst case",
        "Design a migration strategy for the database schema across 3 services",
        "Derive the optimal sharding key given these access patterns",
        "Create a formal specification for this consensus protocol",
        "Analyze the correctness of this lock-free data structure",
    ]);
    m
}

/// Cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a < 1e-10 || mag_b < 1e-10 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

/// Compute the centroid (element-wise average) of a set of vectors.
pub fn compute_centroid(vectors: &[Vec<f32>]) -> Vec<f32> {
    if vectors.is_empty() {
        return Vec::new();
    }
    let dim = vectors[0].len();
    let n = vectors.len() as f32;
    let mut centroid = vec![0.0f32; dim];
    for v in vectors {
        for (i, val) in v.iter().enumerate() {
            centroid[i] += val;
        }
    }
    for c in &mut centroid {
        *c /= n;
    }
    centroid
}

/// Build centroids from pre-computed reference embeddings.
pub fn build_centroids(
    embeddings: &HashMap<ModelTier, Vec<Vec<f32>>>,
) -> HashMap<ModelTier, Vec<f32>> {
    embeddings
        .iter()
        .map(|(tier, vecs)| (*tier, compute_centroid(vecs)))
        .collect()
}

/// Classify a prompt embedding against tier centroids.
/// Returns the best-matching tier and all scores.
pub fn classify_against_centroids(
    embedding: &[f32],
    centroids: &HashMap<ModelTier, Vec<f32>>,
) -> (ModelTier, HashMap<ModelTier, f32>) {
    let mut scores = HashMap::new();
    let mut best_tier = ModelTier::Complex; // safe default
    let mut best_score = f32::NEG_INFINITY;

    for (tier, centroid) in centroids {
        let score = cosine_similarity(embedding, centroid);
        scores.insert(*tier, score);
        if score > best_score {
            best_score = score;
            best_tier = *tier;
        }
    }

    (best_tier, scores)
}

/// Cached embedding entry.
struct CachedEmbedding {
    embedding: Vec<f32>,
    cached_at: Instant,
}

/// The embedding classifier. Holds reference centroids and an HTTP client
/// for calling the embedding provider.
pub struct EmbeddingClassifier {
    config: ClassifierConfig,
    thresholds: RouterThresholds,
    centroids: HashMap<ModelTier, Vec<f32>>,
    http: reqwest::Client,
    cache: RwLock<HashMap<u64, CachedEmbedding>>,
}

impl EmbeddingClassifier {
    /// Create a classifier with pre-computed centroids (for testing or
    /// when reference embeddings are already available).
    pub fn with_centroids(
        config: ClassifierConfig,
        thresholds: RouterThresholds,
        centroids: HashMap<ModelTier, Vec<f32>>,
    ) -> Self {
        Self {
            config,
            thresholds,
            centroids,
            http: reqwest::Client::new(),
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Initialize the classifier by embedding all reference prompts.
    /// Calls the configured embedding provider to get vectors.
    pub async fn initialize(
        config: ClassifierConfig,
        thresholds: RouterThresholds,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Config(format!("failed to build HTTP client: {}", e)))?;

        let reference_prompts = default_reference_prompts();
        let mut tier_embeddings: HashMap<ModelTier, Vec<Vec<f32>>> = HashMap::new();

        for (tier, prompts) in &reference_prompts {
            let embeddings = Self::fetch_embeddings_batch(&http, &config, prompts).await?;
            tier_embeddings.insert(*tier, embeddings);
        }

        let centroids = build_centroids(&tier_embeddings);

        tracing::info!(
            tiers = centroids.len(),
            provider = %config.provider,
            model = %config.model,
            "smart router classifier initialized"
        );

        Ok(Self {
            config,
            thresholds,
            centroids,
            http,
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// Classify a prompt into a model tier.
    pub async fn classify(&self, prompt: &str) -> Result<ClassifyResult> {
        let start = Instant::now();

        // Check cache first.
        let cache_key = hash_prompt(prompt);
        {
            let cache = self.cache.read();
            if let Some(cached) = cache.get(&cache_key) {
                if cached.cached_at.elapsed().as_secs() < self.config.cache_ttl_secs {
                    let (tier, scores) =
                        classify_against_centroids(&cached.embedding, &self.centroids);
                    let tier = self.apply_thresholds(tier, &scores, prompt);
                    return Ok(ClassifyResult {
                        tier,
                        scores,
                        latency_ms: start.elapsed().as_millis() as u64,
                    });
                }
            }
        }

        // Fetch embedding from provider.
        let embedding =
            Self::fetch_embedding(&self.http, &self.config, prompt).await?;

        // Cache it.
        {
            let mut cache = self.cache.write();
            // Evict expired entries if cache is large.
            if cache.len() > 10_000 {
                let ttl = self.config.cache_ttl_secs;
                cache.retain(|_, v| v.cached_at.elapsed().as_secs() < ttl);
            }
            cache.insert(cache_key, CachedEmbedding {
                embedding: embedding.clone(),
                cached_at: Instant::now(),
            });
        }

        let (tier, scores) = classify_against_centroids(&embedding, &self.centroids);
        let tier = self.apply_thresholds(tier, &scores, prompt);

        Ok(ClassifyResult {
            tier,
            scores,
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Apply threshold checks and agentic escalation.
    fn apply_thresholds(
        &self,
        tier: ModelTier,
        scores: &HashMap<ModelTier, f32>,
        prompt: &str,
    ) -> ModelTier {
        // Agentic detection: long prompts or tool-calling patterns escalate.
        let char_threshold = self.thresholds.escalate_token_threshold * 4; // rough chars-to-tokens
        if prompt.len() > char_threshold {
            return match tier {
                ModelTier::Simple => ModelTier::Complex,
                other => other,
            };
        }

        // Check if the winning tier meets its minimum score threshold.
        match tier {
            ModelTier::Simple => {
                let score = scores.get(&ModelTier::Simple).copied().unwrap_or(0.0);
                if score < self.thresholds.simple_min_score as f32 {
                    ModelTier::Complex // not confident enough → safe default
                } else {
                    ModelTier::Simple
                }
            }
            ModelTier::Reasoning => {
                let score = scores.get(&ModelTier::Reasoning).copied().unwrap_or(0.0);
                if score < self.thresholds.reasoning_min_score as f32 {
                    ModelTier::Complex
                } else {
                    ModelTier::Reasoning
                }
            }
            other => other,
        }
    }

    /// Fetch a single embedding from the configured provider.
    async fn fetch_embedding(
        http: &reqwest::Client,
        config: &ClassifierConfig,
        text: &str,
    ) -> Result<Vec<f32>> {
        let url = format!("{}/api/embeddings", config.endpoint);
        let body = serde_json::json!({
            "model": config.model,
            "prompt": text,
        });

        let resp = http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_millis(500))
            .send()
            .await
            .map_err(|e| Error::Provider {
                provider: config.provider.clone(),
                message: format!("embedding request failed: {}", e),
            })?;

        if !resp.status().is_success() {
            return Err(Error::Provider {
                provider: config.provider.clone(),
                message: format!("embedding HTTP {}", resp.status()),
            });
        }

        let json: serde_json::Value = resp.json().await.map_err(|e| Error::Provider {
            provider: config.provider.clone(),
            message: format!("embedding response parse error: {}", e),
        })?;

        let embedding = json
            .get("embedding")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Provider {
                provider: config.provider.clone(),
                message: "missing 'embedding' field in response".into(),
            })?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();

        Ok(embedding)
    }

    /// Fetch embeddings for a batch of texts.
    async fn fetch_embeddings_batch(
        http: &reqwest::Client,
        config: &ClassifierConfig,
        texts: &[&str],
    ) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(Self::fetch_embedding(http, config, text).await?);
        }
        Ok(results)
    }

    /// Check if the classifier's embedding provider is reachable.
    pub async fn health_check(&self) -> bool {
        Self::fetch_embedding(&self.http, &self.config, "health check")
            .await
            .is_ok()
    }

    /// Get the classifier config (for status reporting).
    pub fn config(&self) -> &ClassifierConfig {
        &self.config
    }
}

/// Classification result.
#[derive(Debug, Clone)]
pub struct ClassifyResult {
    pub tier: ModelTier,
    pub scores: HashMap<ModelTier, f32>,
    pub latency_ms: u64,
}

/// Simple hash for cache keys.
fn hash_prompt(prompt: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    prompt.hash(&mut hasher);
    hasher.finish()
}
```

Also add `pub mod classifier;` to `crates/providers/src/lib.rs`, and add `parking_lot` to `crates/providers/Cargo.toml` dependencies (already a workspace dep).

**Step 4: Run test to verify it passes**

Run: `cargo test -p sa-providers -- classifier`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/providers/src/classifier.rs crates/providers/src/lib.rs crates/providers/Cargo.toml
git commit -m "feat(providers): add embedding-based prompt classifier"
```

---

## Task 3: Smart Router in `sa-providers`

**Files:**
- Create: `crates/providers/src/smart_router.rs`
- Modify: `crates/providers/src/lib.rs` (add `pub mod smart_router;`)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sa_domain::config::*;

    fn test_tier_config() -> TierConfig {
        TierConfig {
            simple: vec!["deepseek/deepseek-chat".into()],
            complex: vec!["anthropic/claude-sonnet-4-20250514".into(), "openai/gpt-4o".into()],
            reasoning: vec!["anthropic/claude-opus-4-6".into()],
            free: vec!["venice/venice-uncensored".into()],
        }
    }

    #[test]
    fn resolve_tier_model_picks_first_in_list() {
        let tiers = test_tier_config();
        let model = resolve_tier_model(ModelTier::Complex, &tiers);
        assert_eq!(model, Some("anthropic/claude-sonnet-4-20250514"));
    }

    #[test]
    fn resolve_tier_model_empty_tier_returns_none() {
        let tiers = TierConfig::default();
        let model = resolve_tier_model(ModelTier::Simple, &tiers);
        assert!(model.is_none());
    }

    #[test]
    fn profile_to_tier_eco_is_simple() {
        assert_eq!(profile_to_tier(RoutingProfile::Eco), Some(ModelTier::Simple));
    }

    #[test]
    fn profile_to_tier_premium_is_complex() {
        assert_eq!(profile_to_tier(RoutingProfile::Premium), Some(ModelTier::Complex));
    }

    #[test]
    fn profile_to_tier_auto_is_none() {
        assert_eq!(profile_to_tier(RoutingProfile::Auto), None);
    }

    #[test]
    fn resolve_with_explicit_model_bypasses_router() {
        let tiers = test_tier_config();
        let result = resolve_model_for_request(
            Some("openai/gpt-4o-mini"),
            RoutingProfile::Eco,
            None,
            &tiers,
        );
        assert_eq!(result.model, "openai/gpt-4o-mini");
        assert!(result.bypassed);
    }

    #[test]
    fn resolve_with_eco_profile_uses_simple_tier() {
        let tiers = test_tier_config();
        let result = resolve_model_for_request(
            None,
            RoutingProfile::Eco,
            None,
            &tiers,
        );
        assert_eq!(result.model, "deepseek/deepseek-chat");
        assert!(!result.bypassed);
    }

    #[test]
    fn resolve_with_auto_profile_uses_classified_tier() {
        let tiers = test_tier_config();
        let result = resolve_model_for_request(
            None,
            RoutingProfile::Auto,
            Some(ModelTier::Reasoning),
            &tiers,
        );
        assert_eq!(result.model, "anthropic/claude-opus-4-6");
    }

    #[test]
    fn resolve_falls_back_across_tiers() {
        // Only reasoning tier has models
        let tiers = TierConfig {
            simple: vec![],
            complex: vec![],
            reasoning: vec!["anthropic/claude-opus-4-6".into()],
            free: vec![],
        };
        let result = resolve_model_for_request(
            None,
            RoutingProfile::Eco, // wants simple, but empty
            None,
            &tiers,
        );
        // Should fall back: simple(empty) -> complex(empty) -> reasoning
        assert_eq!(result.model, "anthropic/claude-opus-4-6");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-providers -- smart_router`
Expected: FAIL

**Step 3: Write minimal implementation**

```rust
//! Smart router — resolves the best model for a request based on routing
//! profile and optional classifier tier.

use sa_domain::config::{ModelTier, RoutingProfile, TierConfig};

/// The result of a routing decision.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    /// The resolved `provider/model` string.
    pub model: String,
    /// Which tier was selected.
    pub tier: ModelTier,
    /// The routing profile that was used.
    pub profile: RoutingProfile,
    /// True if an explicit model override bypassed the router.
    pub bypassed: bool,
}

/// Map a fixed profile to its corresponding tier.
/// Returns `None` for `Auto` (requires classification).
pub fn profile_to_tier(profile: RoutingProfile) -> Option<ModelTier> {
    match profile {
        RoutingProfile::Auto => None,
        RoutingProfile::Eco => Some(ModelTier::Simple),
        RoutingProfile::Premium => Some(ModelTier::Complex),
        RoutingProfile::Free => Some(ModelTier::Free),
        RoutingProfile::Reasoning => Some(ModelTier::Reasoning),
    }
}

/// Get the first available model from a tier.
pub fn resolve_tier_model<'a>(tier: ModelTier, tiers: &'a TierConfig) -> Option<&'a str> {
    let list = match tier {
        ModelTier::Simple => &tiers.simple,
        ModelTier::Complex => &tiers.complex,
        ModelTier::Reasoning => &tiers.reasoning,
        ModelTier::Free => &tiers.free,
    };
    list.first().map(|s| s.as_str())
}

/// Tier fallback order: Simple -> Complex -> Reasoning.
fn fallback_tiers(starting: ModelTier) -> Vec<ModelTier> {
    match starting {
        ModelTier::Simple => vec![ModelTier::Complex, ModelTier::Reasoning],
        ModelTier::Complex => vec![ModelTier::Reasoning, ModelTier::Simple],
        ModelTier::Reasoning => vec![ModelTier::Complex, ModelTier::Simple],
        ModelTier::Free => vec![ModelTier::Simple, ModelTier::Complex, ModelTier::Reasoning],
    }
}

/// Core resolution: explicit model > profile tier > classified tier > fallback.
pub fn resolve_model_for_request(
    explicit_model: Option<&str>,
    profile: RoutingProfile,
    classified_tier: Option<ModelTier>,
    tiers: &TierConfig,
) -> RoutingDecision {
    // 1. Explicit model override bypasses everything.
    if let Some(model) = explicit_model {
        return RoutingDecision {
            model: model.to_string(),
            tier: ModelTier::Complex, // doesn't matter when bypassed
            profile,
            bypassed: true,
        };
    }

    // 2. Determine tier from profile or classifier.
    let target_tier = profile_to_tier(profile)
        .or(classified_tier)
        .unwrap_or(ModelTier::Complex); // safe default

    // 3. Try target tier, then fallbacks.
    if let Some(model) = resolve_tier_model(target_tier, tiers) {
        return RoutingDecision {
            model: model.to_string(),
            tier: target_tier,
            profile,
            bypassed: false,
        };
    }

    for fallback in fallback_tiers(target_tier) {
        if let Some(model) = resolve_tier_model(fallback, tiers) {
            return RoutingDecision {
                model: model.to_string(),
                tier: fallback,
                profile,
                bypassed: false,
            };
        }
    }

    // Last resort: return empty (caller should handle this).
    RoutingDecision {
        model: String::new(),
        tier: target_tier,
        profile,
        bypassed: false,
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p sa-providers -- smart_router`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/providers/src/smart_router.rs crates/providers/src/lib.rs
git commit -m "feat(providers): add smart router resolution logic"
```

---

## Task 4: Decisions Ring Buffer in `sa-providers`

**Files:**
- Create: `crates/providers/src/decisions.rs`
- Modify: `crates/providers/src/lib.rs` (add `pub mod decisions;`)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sa_domain::config::{ModelTier, RoutingProfile};

    #[test]
    fn ring_buffer_stores_up_to_capacity() {
        let buf = DecisionLog::new(3);
        for i in 0..5 {
            buf.record(Decision {
                timestamp: chrono::Utc::now(),
                prompt_snippet: format!("prompt {}", i),
                profile: RoutingProfile::Auto,
                tier: ModelTier::Simple,
                model: "test/model".into(),
                latency_ms: 5,
                bypassed: false,
            });
        }
        let recent = buf.recent(10);
        assert_eq!(recent.len(), 3);
        // Most recent first
        assert!(recent[0].prompt_snippet.contains("4"));
    }

    #[test]
    fn ring_buffer_recent_respects_limit() {
        let buf = DecisionLog::new(100);
        for i in 0..50 {
            buf.record(Decision {
                timestamp: chrono::Utc::now(),
                prompt_snippet: format!("prompt {}", i),
                profile: RoutingProfile::Eco,
                tier: ModelTier::Simple,
                model: "test/model".into(),
                latency_ms: 1,
                bypassed: false,
            });
        }
        let recent = buf.recent(5);
        assert_eq!(recent.len(), 5);
    }

    #[test]
    fn ring_buffer_empty() {
        let buf = DecisionLog::new(10);
        assert!(buf.recent(5).is_empty());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p sa-providers -- decisions`
Expected: FAIL

**Step 3: Write minimal implementation**

```rust
//! Ring buffer for recent routing decisions (observability).

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use sa_domain::config::{ModelTier, RoutingProfile};
use serde::Serialize;
use std::collections::VecDeque;

/// A single routing decision record.
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub timestamp: DateTime<Utc>,
    pub prompt_snippet: String,
    pub profile: RoutingProfile,
    pub tier: ModelTier,
    pub model: String,
    pub latency_ms: u64,
    pub bypassed: bool,
}

/// Thread-safe ring buffer of recent decisions.
pub struct DecisionLog {
    inner: Mutex<VecDeque<Decision>>,
    capacity: usize,
}

impl DecisionLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    pub fn record(&self, decision: Decision) {
        let mut buf = self.inner.lock();
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(decision);
    }

    /// Get the N most recent decisions, newest first.
    pub fn recent(&self, limit: usize) -> Vec<Decision> {
        let buf = self.inner.lock();
        buf.iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p sa-providers -- decisions`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/providers/src/decisions.rs crates/providers/src/lib.rs
git commit -m "feat(providers): add routing decisions ring buffer"
```

---

## Task 5: Wire Router into Gateway `resolve_provider()`

**Files:**
- Modify: `crates/gateway/src/state.rs` (add `SmartRouter` to `AppState`)
- Modify: `crates/gateway/src/runtime/mod.rs` (update `resolve_provider()`)
- Modify: `crates/gateway/src/runtime/turn.rs` (pass prompt to resolve)

**Context:** The `resolve_provider()` function at `crates/gateway/src/runtime/mod.rs:84-121` currently resolves providers by: explicit override → agent model → role default → any. We need to add a step between "explicit override" and "agent model" that checks the smart router when enabled.

**Step 1: Add `SmartRouter` to `AppState`**

Read `crates/gateway/src/state.rs` to find the `AppState` struct, then add:

```rust
/// Smart router (None when [llm.router] is not configured or disabled).
pub smart_router: Option<Arc<SmartRouterState>>,
```

Where `SmartRouterState` bundles the classifier + tiers + decisions log:

```rust
pub struct SmartRouterState {
    pub classifier: Option<sa_providers::classifier::EmbeddingClassifier>,
    pub tiers: sa_domain::config::TierConfig,
    pub default_profile: sa_domain::config::RoutingProfile,
    pub decisions: sa_providers::decisions::DecisionLog,
}
```

**Step 2: Update `resolve_provider()` signature**

Change the signature to also accept the prompt text and optional routing profile:

```rust
pub(super) fn resolve_provider(
    state: &AppState,
    model_override: Option<&str>,
    agent_ctx: Option<&agent::AgentContext>,
    prompt_for_routing: Option<&str>,
    routing_profile: Option<RoutingProfile>,
) -> Result<(Arc<dyn sa_providers::LlmProvider>, Option<String>), Box<dyn std::error::Error + Send + Sync>>
```

The return type now includes an optional model name override (from the router).

**Step 3: Add router logic after explicit override check**

Inside `resolve_provider()`, after the explicit override check (step 1) and before agent-level mapping (step 2), add:

```rust
// 1.5 Smart router (when enabled and no explicit override).
if let Some(router) = &state.smart_router {
    let profile = routing_profile.unwrap_or(router.default_profile);

    // For non-Auto profiles, resolve tier directly (no classifier needed).
    if let Some(tier) = sa_providers::smart_router::profile_to_tier(profile) {
        if let Some(model_spec) = sa_providers::smart_router::resolve_tier_model(tier, &router.tiers) {
            let provider_id = model_spec.split('/').next().unwrap_or(model_spec);
            if let Some(p) = state.llm.get(provider_id) {
                // Record decision
                router.decisions.record(sa_providers::decisions::Decision {
                    timestamp: chrono::Utc::now(),
                    prompt_snippet: prompt_for_routing.unwrap_or("").chars().take(80).collect(),
                    profile,
                    tier,
                    model: model_spec.to_string(),
                    latency_ms: 0,
                    bypassed: false,
                });
                return Ok((p, Some(model_spec.split('/').nth(1).unwrap_or("").to_string())));
            }
        }
    }

    // Auto profile: classify with embeddings if classifier is available.
    // (Async classification happens in turn.rs before calling resolve_provider)
}
```

**Step 4: Update all callers of `resolve_provider()`**

In `turn.rs` at `prepare_turn_context()`, pass the user message and routing profile:

```rust
let (provider, model_override) = resolve_provider(
    &state,
    input.model.as_deref(),
    input.agent.as_ref(),
    Some(&input.user_message),
    None, // routing_profile from schedule or agent (future)
)?;
```

In `schedule_runner.rs`, if the schedule has a routing_profile field (future), pass it.

**Step 5: Test**

Run: `cargo test -p sa-gateway`
Expected: All existing tests PASS (router is None by default, so no behavioral change)

**Step 6: Commit**

```bash
git add crates/gateway/src/state.rs crates/gateway/src/runtime/mod.rs crates/gateway/src/runtime/turn.rs
git commit -m "feat(gateway): wire smart router into resolve_provider()"
```

---

## Task 6: Router API Endpoints

**Files:**
- Create: `crates/gateway/src/api/router.rs`
- Modify: `crates/gateway/src/api/mod.rs` (add `pub mod router;` and routes)

**Step 1: Implement 4 endpoints**

```
GET  /v1/router/status     — classifier health, active profile, tier config
PUT  /v1/router/config     — update profile, tiers (persists to config)
POST /v1/router/classify   — test: send a prompt, get back tier + scores + model
GET  /v1/router/decisions   — last 100 routing decisions
```

**Step 2: Add response types**

```rust
#[derive(Serialize)]
struct RouterStatusResponse {
    enabled: bool,
    default_profile: String,
    classifier: ClassifierStatus,
    tiers: HashMap<String, Vec<String>>,
    thresholds: HashMap<String, f64>,
}

#[derive(Serialize)]
struct ClassifierStatus {
    provider: String,
    model: String,
    connected: bool,
    avg_latency_ms: u64,
}

#[derive(Deserialize)]
struct ClassifyRequest {
    prompt: String,
}

#[derive(Serialize)]
struct ClassifyResponse {
    tier: String,
    scores: HashMap<String, f32>,
    resolved_model: String,
    latency_ms: u64,
}
```

**Step 3: Wire routes into `api/mod.rs`**

Add to the protected router:

```rust
.route("/v1/router/status", get(router::status))
.route("/v1/router/config", put(router::update_config))
.route("/v1/router/classify", post(router::classify))
.route("/v1/router/decisions", get(router::decisions))
```

**Step 4: Test with curl**

```bash
# Status (should return enabled: false when not configured)
curl -s http://localhost:3210/v1/router/status -H "Authorization: Bearer $TOKEN" | jq .

# Classify (when enabled)
curl -s -X POST http://localhost:3210/v1/router/classify \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "What time is it?"}' | jq .
```

**Step 5: Commit**

```bash
git add crates/gateway/src/api/router.rs crates/gateway/src/api/mod.rs
git commit -m "feat(gateway): add router API endpoints (status, classify, decisions)"
```

---

## Task 7: `routing_profile` Field on Schedule

**Files:**
- Modify: `crates/gateway/src/runtime/schedules/model.rs` (add field)
- Modify: `crates/gateway/src/api/schedules.rs` (add to create/update requests)
- Modify: `crates/gateway/src/runtime/schedule_runner.rs` (pass to resolve_provider)

**Step 1: Add `routing_profile` to `Schedule` struct**

```rust
/// Routing profile override for this schedule.
/// None = use default profile from router config.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub routing_profile: Option<String>,
```

**Step 2: Wire through create/update API**

Add `routing_profile: Option<String>` to `CreateScheduleRequest` and `UpdateScheduleRequest`.

**Step 3: Pass to `resolve_provider()` in schedule runner**

In `schedule_runner.rs`, parse the profile string and pass it:

```rust
let routing_profile = schedule.routing_profile.as_deref()
    .and_then(|s| serde_json::from_str(&format!("\"{}\"", s)).ok());
```

**Step 4: Update test helpers**

Add `routing_profile: None` to all test `Schedule` constructors.

**Step 5: Test**

Run: `cargo test -p sa-gateway`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/gateway/src/runtime/schedules/model.rs crates/gateway/src/api/schedules.rs crates/gateway/src/runtime/schedule_runner.rs
git commit -m "feat(gateway): add routing_profile field to schedules"
```

---

## Task 8: Router Initialization at Startup

**Files:**
- Modify: `crates/gateway/src/main.rs` or startup code (wherever AppState is built)

**Step 1: Initialize SmartRouterState from config**

During gateway startup, after building the `ProviderRegistry`, check for `[llm.router]` config:

```rust
let smart_router = if let Some(ref router_cfg) = config.llm.router {
    if router_cfg.enabled {
        let classifier = match sa_providers::classifier::EmbeddingClassifier::initialize(
            router_cfg.classifier.clone(),
            router_cfg.thresholds.clone(),
        ).await {
            Ok(c) => {
                tracing::info!("smart router classifier initialized");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(error = %e, "smart router classifier failed to init, routing without classification");
                None
            }
        };
        Some(Arc::new(SmartRouterState {
            classifier,
            tiers: router_cfg.tiers.clone(),
            default_profile: router_cfg.default_profile,
            decisions: sa_providers::decisions::DecisionLog::new(100),
        }))
    } else {
        None
    }
} else {
    None
};
```

**Step 2: Test startup without config**

Run: `cargo build --release -p sa-gateway`
Expected: Builds clean. Gateway starts without `[llm.router]` section (no behavioral change).

**Step 3: Test startup with config**

Add to `config.toml`:

```toml
[llm.router]
enabled = true
default_profile = "eco"

[llm.router.tiers]
simple = ["deepseek/deepseek-chat", "google/gemini-2.0-flash"]
complex = ["anthropic/claude-sonnet-4-20250514", "openai/gpt-4o"]
reasoning = ["anthropic/claude-opus-4-6"]
free = ["venice/venice-uncensored"]
```

Start the gateway and verify:
- Logs show "smart router classifier initialized" or appropriate fallback
- `GET /v1/router/status` returns the expected config
- Existing functionality unchanged

**Step 4: Commit**

```bash
git add crates/gateway/src/main.rs crates/gateway/src/state.rs
git commit -m "feat(gateway): initialize smart router at startup from config"
```

---

## Task 9: Dashboard API Client Methods

**Files:**
- Modify: `apps/dashboard/src/api/client.ts`

**Step 1: Add types**

```typescript
export interface RouterStatus {
  enabled: boolean;
  default_profile: string;
  classifier: {
    provider: string;
    model: string;
    connected: boolean;
    avg_latency_ms: number;
  };
  tiers: Record<string, string[]>;
  thresholds: Record<string, number>;
}

export interface ClassifyResult {
  tier: string;
  scores: Record<string, number>;
  resolved_model: string;
  latency_ms: number;
}

export interface RouterDecision {
  timestamp: string;
  prompt_snippet: string;
  profile: string;
  tier: string;
  model: string;
  latency_ms: number;
  bypassed: boolean;
}
```

**Step 2: Add API methods**

```typescript
async routerStatus(): Promise<RouterStatus> {
  return get("/v1/router/status");
},

async updateRouterConfig(config: Partial<RouterStatus>): Promise<RouterStatus> {
  return put("/v1/router/config", config);
},

async classifyPrompt(prompt: string): Promise<ClassifyResult> {
  return post("/v1/router/classify", { prompt });
},

async routerDecisions(limit = 100): Promise<RouterDecision[]> {
  return get(`/v1/router/decisions?limit=${limit}`);
},
```

**Step 3: Test**

Verify TypeScript compiles: `cd apps/dashboard && npx tsc --noEmit`

**Step 4: Commit**

```bash
git add apps/dashboard/src/api/client.ts
git commit -m "feat(dashboard): add router API client methods and types"
```

---

## Task 10: Dashboard Settings — LLM Router Card

**Files:**
- Modify: `apps/dashboard/src/pages/Settings.vue`

**Step 1: Add router state and fetch**

Add to `<script setup>`:

```typescript
import type { RouterStatus, RouterDecision } from "@/api/client";

const routerStatus = ref<RouterStatus | null>(null);
const routerDecisions = ref<RouterDecision[]>([]);
const routerLoading = ref(false);
const routerError = ref("");
const decisionsExpanded = ref(false);

async function loadRouter() {
  routerLoading.value = true;
  routerError.value = "";
  try {
    routerStatus.value = await api.routerStatus();
    routerDecisions.value = await api.routerDecisions(20);
  } catch (e: unknown) {
    routerError.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    routerLoading.value = false;
  }
}
```

Call `loadRouter()` inside `onMounted` alongside existing `load()`.

**Step 2: Add the router card template**

After the "Environment" card, add an "LLM Router" card:

```html
<Card v-if="routerStatus" title="LLM Router">
  <div class="readiness-header">
    <span :class="routerStatus.enabled ? 'status-ok' : 'status-warn'">
      {{ routerStatus.enabled ? "Enabled" : "Disabled" }}
    </span>
    <span class="profile-badge">{{ routerStatus.default_profile }}</span>
  </div>

  <!-- Classifier Status -->
  <div class="sub-heading">Classifier</div>
  <div class="settings-grid">
    <div><span class="label">Provider</span> <span class="mono val">{{ routerStatus.classifier.provider }}</span></div>
    <div><span class="label">Model</span> <span class="mono val">{{ routerStatus.classifier.model }}</span></div>
    <div><span class="label">Status</span>
      <span :class="routerStatus.classifier.connected ? 'status-ok' : 'status-warn'">
        {{ routerStatus.classifier.connected ? "Connected" : "Disconnected" }}
      </span>
    </div>
    <div><span class="label">Avg Latency</span> <span class="mono val">{{ routerStatus.classifier.avg_latency_ms }}ms</span></div>
  </div>

  <!-- Tier Assignments -->
  <div class="sub-heading">Tier Assignments</div>
  <div v-for="(models, tier) in routerStatus.tiers" :key="tier" class="tier-row">
    <span class="tier-label">{{ tier }}</span>
    <span class="mono val">{{ models.join(", ") || "—" }}</span>
  </div>

  <!-- Recent Decisions (collapsible) -->
  <div class="sub-heading clickable" @click="decisionsExpanded = !decisionsExpanded">
    Recent Decisions {{ decisionsExpanded ? "▾" : "▸" }}
  </div>
  <div v-if="decisionsExpanded && routerDecisions.length > 0" class="decisions-log">
    <div v-for="d in routerDecisions" :key="d.timestamp" class="decision-row">
      <span class="dim">{{ new Date(d.timestamp).toLocaleTimeString() }}</span>
      <span class="tier-badge" :class="'tier-' + d.tier">{{ d.tier }}</span>
      <span class="mono">{{ d.model }}</span>
      <span class="dim">{{ d.latency_ms }}ms</span>
      <span class="dim decision-snippet">{{ d.prompt_snippet }}</span>
    </div>
  </div>
  <div v-if="decisionsExpanded && routerDecisions.length === 0" class="dim">
    No routing decisions recorded yet.
  </div>

  <p v-if="routerError" class="error">{{ routerError }}</p>
</Card>
```

**Step 3: Add styles**

```css
.profile-badge {
  background: var(--accent);
  color: #fff;
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
}
.tier-row {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  padding: 0.2rem 0;
  font-size: 0.82rem;
}
.tier-label {
  min-width: 5rem;
  color: var(--text-dim);
  font-weight: 500;
  text-transform: capitalize;
}
.clickable { cursor: pointer; user-select: none; }
.decisions-log {
  max-height: 300px;
  overflow-y: auto;
  font-size: 0.78rem;
}
.decision-row {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  padding: 0.2rem 0;
  border-bottom: 1px solid var(--border);
}
.decision-snippet {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tier-badge {
  padding: 0.1rem 0.4rem;
  border-radius: 3px;
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
}
.tier-simple { background: var(--green); color: #000; }
.tier-complex { background: var(--accent); color: #fff; }
.tier-reasoning { background: var(--red); color: #fff; }
.tier-free { background: var(--text-dim); color: #fff; }
```

**Step 4: Test**

1. Run the dashboard dev server: `cd apps/dashboard && npm run dev`
2. Open Settings page
3. Verify the Router card appears (shows "Disabled" if no config, or "Enabled" with tier info if configured)

**Step 5: Commit**

```bash
git add apps/dashboard/src/pages/Settings.vue
git commit -m "feat(dashboard): add LLM Router card to Settings page"
```

---

## Task 11: Schedule Edit Form — Routing Profile Dropdown

**Files:**
- Find and modify the schedule create/edit form component in `apps/dashboard/src/`

**Step 1: Locate the schedule form component**

Search for the schedule edit form in the dashboard: `grep -r "routing_profile\|prompt_template\|CreateSchedule" apps/dashboard/src/`

**Step 2: Add a "Routing Profile" dropdown**

Add a `<select>` for routing profile alongside the existing "Model" field:

```html
<div class="form-group">
  <label>Routing Profile</label>
  <select v-model="form.routing_profile" :disabled="!!form.model">
    <option value="">Default (inherit)</option>
    <option value="auto">Auto</option>
    <option value="eco">Eco</option>
    <option value="premium">Premium</option>
    <option value="free">Free</option>
    <option value="reasoning">Reasoning</option>
  </select>
  <span v-if="form.model" class="dim">Disabled — explicit model is set</span>
</div>
```

**Step 3: Wire into create/update API calls**

Ensure the form submits `routing_profile` in the request body.

**Step 4: Test**

1. Open schedule create form in dashboard
2. Verify dropdown appears and is disabled when Model is set
3. Create/edit a schedule with a routing profile

**Step 5: Commit**

```bash
git add apps/dashboard/src/
git commit -m "feat(dashboard): add routing profile dropdown to schedule form"
```

---

## Task 12: Integration Test — Full Round-Trip

**Files:**
- Create: test in `crates/gateway/` or existing test file

**Step 1: Write an integration test**

Test the full flow: config → router init → classify → resolve → decision logged.

Use mock/pre-computed embeddings (no Ollama dependency):

```rust
#[tokio::test]
async fn smart_router_eco_profile_resolves_simple_tier() {
    // Build a SmartRouterState with no classifier (fixed profiles don't need one)
    let router = SmartRouterState {
        classifier: None,
        tiers: TierConfig {
            simple: vec!["deepseek/deepseek-chat".into()],
            complex: vec!["anthropic/claude-sonnet-4-20250514".into()],
            reasoning: vec![],
            free: vec![],
        },
        default_profile: RoutingProfile::Eco,
        decisions: DecisionLog::new(10),
    };

    let decision = resolve_model_for_request(
        None,
        RoutingProfile::Eco,
        None,
        &router.tiers,
    );

    assert_eq!(decision.model, "deepseek/deepseek-chat");
    assert_eq!(decision.tier, ModelTier::Simple);
    assert!(!decision.bypassed);
}
```

**Step 2: Run all tests**

Run: `cargo test`
Expected: All tests PASS, including new router tests

**Step 3: Build release**

Run: `cargo build --release -p sa-gateway`
Expected: Clean build

**Step 4: Commit**

```bash
git add .
git commit -m "test(gateway): add smart router integration test"
```

---

## Summary

| Task | What | Files | Est. |
|------|------|-------|------|
| 1 | Config types in sa-domain | `config/llm.rs` | 10 min |
| 2 | Embedding classifier | `classifier.rs` (new) | 15 min |
| 3 | Smart router resolution | `smart_router.rs` (new) | 10 min |
| 4 | Decisions ring buffer | `decisions.rs` (new) | 5 min |
| 5 | Wire into gateway | `state.rs`, `mod.rs`, `turn.rs` | 15 min |
| 6 | Router API endpoints | `api/router.rs` (new) | 10 min |
| 7 | Schedule routing_profile | `model.rs`, `schedules.rs`, `schedule_runner.rs` | 10 min |
| 8 | Startup initialization | `main.rs` | 10 min |
| 9 | Dashboard API client | `client.ts` | 5 min |
| 10 | Dashboard Router card | `Settings.vue` | 15 min |
| 11 | Schedule form dropdown | schedule form component | 10 min |
| 12 | Integration test | test files | 5 min |
