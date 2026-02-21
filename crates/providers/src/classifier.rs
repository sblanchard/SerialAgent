//! Embedding-based prompt classifier for smart model routing.
//!
//! Uses cosine similarity between prompt embeddings and pre-computed tier
//! centroids to classify incoming prompts as Simple, Complex, or Reasoning.
//! Embeddings are fetched from an Ollama-compatible endpoint and cached
//! in-memory with TTL-based eviction.

use parking_lot::RwLock;
use sa_domain::config::{ClassifierConfig, ModelTier, RouterThresholds};
use sa_domain::error::{Error, Result};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Maximum number of cached embeddings before eviction runs.
const CACHE_MAX_ENTRIES: usize = 10_000;

/// Timeout for individual embedding requests.
const EMBEDDING_TIMEOUT: Duration = Duration::from_millis(500);

/// Timeout for batch initialization (fetching all reference embeddings).
const BATCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Approximate chars-per-token multiplier for agentic detection.
const CHARS_PER_TOKEN: usize = 4;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Reference prompts
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Reference prompts used to build tier centroids at startup.
///
/// Each tier gets a set of representative prompts whose embeddings are
/// averaged to form that tier's centroid vector.
pub fn default_reference_prompts() -> HashMap<ModelTier, Vec<&'static str>> {
    let mut prompts = HashMap::new();

    prompts.insert(
        ModelTier::Simple,
        vec![
            "What is the capital of France?",
            "Convert 5 miles to kilometers",
            "What time is it in Tokyo?",
            "Define the word 'ephemeral'",
            "How many cups in a gallon?",
            "What year was the Eiffel Tower built?",
            "Translate 'hello' to Spanish",
            "What is 15% of 200?",
        ],
    );

    prompts.insert(
        ModelTier::Complex,
        vec![
            "Write a Python script that scrapes a website and stores the data in a SQLite database with proper error handling",
            "Explain the differences between microservices and monolithic architectures, including trade-offs for a startup",
            "Design a REST API for a multi-tenant SaaS application with rate limiting and authentication",
            "Refactor this legacy codebase to use dependency injection and add comprehensive test coverage",
            "Create a data pipeline that ingests CSV files, validates schemas, transforms data, and loads into PostgreSQL",
            "Build a React component library with TypeScript, Storybook documentation, and unit tests",
            "Implement a caching strategy for a high-traffic e-commerce API with cache invalidation",
            "Debug this distributed system issue where messages are being processed out of order",
        ],
    );

    prompts.insert(
        ModelTier::Reasoning,
        vec![
            "Prove that the square root of 2 is irrational using proof by contradiction",
            "Analyze the computational complexity of this recursive algorithm and suggest optimizations with formal proofs",
            "Design a consensus protocol for a Byzantine fault-tolerant distributed system and prove its safety properties",
            "Evaluate the philosophical implications of artificial general intelligence on human autonomy and free will",
            "Derive the optimal strategy for this game theory problem using backward induction and Nash equilibrium",
            "Compare and critically evaluate three competing theories of consciousness with respect to the hard problem",
            "Analyze the economic second-order effects of universal basic income on labor markets and innovation",
            "Formally verify the correctness of this concurrent data structure using temporal logic",
        ],
    );

    prompts
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Vector math
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Cosine similarity between two vectors.
///
/// Returns a value in `[-1.0, 1.0]`. Returns `0.0` if either vector has
/// zero magnitude (avoiding division by zero).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        tracing::warn!(
            len_a = a.len(),
            len_b = b.len(),
            "cosine_similarity: mismatched vector lengths, returning 0.0"
        );
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }

    dot / (mag_a * mag_b)
}

/// Compute the centroid (element-wise average) of a set of vectors.
///
/// Returns an empty vector if the input is empty.
pub fn compute_centroid(vectors: &[Vec<f32>]) -> Vec<f32> {
    if vectors.is_empty() {
        return Vec::new();
    }

    let dim = vectors[0].len();
    let count = vectors.len() as f32;

    let mut centroid = vec![0.0f32; dim];
    for v in vectors {
        for (acc, val) in centroid.iter_mut().zip(v.iter()) {
            *acc += val;
        }
    }
    for val in &mut centroid {
        *val /= count;
    }

    centroid
}

/// Build centroids from pre-computed reference embeddings.
///
/// Each tier maps to a list of embedding vectors; this function computes
/// the centroid of each tier's vectors.
pub fn build_centroids(
    embeddings: &HashMap<ModelTier, Vec<Vec<f32>>>,
) -> HashMap<ModelTier, Vec<f32>> {
    embeddings
        .iter()
        .map(|(tier, vecs)| (*tier, compute_centroid(vecs)))
        .collect()
}

/// Classify a prompt embedding against tier centroids.
///
/// Returns the best-matching tier and a map of all tier scores.
/// If centroids are empty, defaults to `ModelTier::Complex`.
pub fn classify_against_centroids(
    embedding: &[f32],
    centroids: &HashMap<ModelTier, Vec<f32>>,
) -> (ModelTier, HashMap<ModelTier, f32>) {
    let mut scores = HashMap::new();
    let mut best_tier = ModelTier::Complex;
    let mut best_score = f32::NEG_INFINITY;

    for (tier, centroid) in centroids {
        let score = cosine_similarity(embedding, centroid);
        scores.insert(*tier, score);
        if score > best_score {
            best_score = score;
            best_tier = *tier;
        }
    }

    // When centroids are empty or all scores are tied / ambiguous, default to Complex.
    if centroids.is_empty() {
        return (ModelTier::Complex, scores);
    }

    (best_tier, scores)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Cache entry
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A cached embedding vector with expiration time.
struct CachedEmbedding {
    embedding: Vec<f32>,
    expires_at: Instant,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Classifier result
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Result of classifying a prompt.
#[derive(Debug, Clone)]
pub struct ClassifyResult {
    /// The selected model tier.
    pub tier: ModelTier,
    /// Cosine similarity scores for each tier.
    pub scores: HashMap<ModelTier, f32>,
    /// Classification latency in milliseconds.
    pub latency_ms: u64,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Embedding classifier
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Embedding-based prompt classifier.
///
/// Maintains pre-computed centroids for each model tier and classifies
/// incoming prompts by comparing their embeddings against those centroids.
pub struct EmbeddingClassifier {
    config: ClassifierConfig,
    thresholds: RouterThresholds,
    centroids: HashMap<ModelTier, Vec<f32>>,
    http: reqwest::Client,
    cache: RwLock<HashMap<u64, CachedEmbedding>>,
}

impl EmbeddingClassifier {
    /// Create a classifier with pre-computed centroids (useful for testing
    /// or when centroids are loaded from a snapshot).
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

    /// Initialize the classifier by fetching embeddings for all reference
    /// prompts and building centroids.
    ///
    /// This makes HTTP calls to the configured embedding endpoint.
    pub async fn initialize(
        config: ClassifierConfig,
        thresholds: RouterThresholds,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(BATCH_TIMEOUT)
            .build()
            .map_err(|e| Error::Http(format!("failed to build HTTP client: {e}")))?;

        let reference_prompts = default_reference_prompts();
        let mut tier_embeddings: HashMap<ModelTier, Vec<Vec<f32>>> = HashMap::new();

        for (tier, prompts) in &reference_prompts {
            let texts: Vec<&str> = prompts.iter().copied().collect();
            let embeddings = Self::fetch_embeddings_batch(&http, &config, &texts).await?;
            tier_embeddings.insert(*tier, embeddings);
        }

        let centroids = build_centroids(&tier_embeddings);

        tracing::info!(
            tiers = centroids.len(),
            "embedding classifier initialized with centroids"
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
    ///
    /// 1. Checks the embedding cache.
    /// 2. Fetches embedding from the provider if not cached.
    /// 3. Compares against centroids.
    /// 4. Applies threshold rules and agentic escalation.
    pub async fn classify(&self, prompt: &str) -> Result<ClassifyResult> {
        let start = Instant::now();

        // Check cache first.
        let cache_key = hash_prompt(prompt);
        if let Some(cached) = self.get_cached(cache_key) {
            let (tier, scores) = classify_against_centroids(&cached, &self.centroids);
            let final_tier = self.apply_thresholds(tier, &scores, prompt);
            return Ok(ClassifyResult {
                tier: final_tier,
                scores,
                latency_ms: start.elapsed().as_millis() as u64,
            });
        }

        // Fetch embedding from the provider.
        let embedding = Self::fetch_embedding(&self.http, &self.config, prompt).await?;

        // Cache the result.
        self.put_cached(cache_key, &embedding);

        // Classify.
        let (tier, scores) = classify_against_centroids(&embedding, &self.centroids);
        let final_tier = self.apply_thresholds(tier, &scores, prompt);

        Ok(ClassifyResult {
            tier: final_tier,
            scores,
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Apply threshold rules to potentially escalate or de-escalate the tier.
    ///
    /// Rules:
    /// - If classified as Simple but score < simple_min_score, escalate to Complex.
    /// - If classified as Reasoning but score < reasoning_min_score, fall back to Complex.
    /// - If prompt is long (agentic), escalate Simple to Complex.
    fn apply_thresholds(
        &self,
        tier: ModelTier,
        scores: &HashMap<ModelTier, f32>,
        prompt: &str,
    ) -> ModelTier {
        // Agentic detection: long prompts escalate Simple -> Complex.
        let char_threshold = self.thresholds.escalate_token_threshold * CHARS_PER_TOKEN;
        let after_length = if tier == ModelTier::Simple && prompt.len() > char_threshold {
            tracing::debug!(
                prompt_len = prompt.len(),
                threshold = char_threshold,
                "escalating Simple -> Complex due to prompt length"
            );
            ModelTier::Complex
        } else {
            tier
        };

        // Threshold checks.
        match after_length {
            ModelTier::Simple => {
                let score = scores
                    .get(&ModelTier::Simple)
                    .copied()
                    .unwrap_or(0.0) as f64;
                if score < self.thresholds.simple_min_score {
                    tracing::debug!(
                        score,
                        min = self.thresholds.simple_min_score,
                        "escalating Simple -> Complex due to low score"
                    );
                    ModelTier::Complex
                } else {
                    ModelTier::Simple
                }
            }
            ModelTier::Reasoning => {
                let score = scores
                    .get(&ModelTier::Reasoning)
                    .copied()
                    .unwrap_or(0.0) as f64;
                if score < self.thresholds.reasoning_min_score {
                    tracing::debug!(
                        score,
                        min = self.thresholds.reasoning_min_score,
                        "de-escalating Reasoning -> Complex due to low score"
                    );
                    ModelTier::Complex
                } else {
                    ModelTier::Reasoning
                }
            }
            other => other,
        }
    }

    /// Fetch a single embedding vector from the Ollama-compatible endpoint.
    async fn fetch_embedding(
        http: &reqwest::Client,
        config: &ClassifierConfig,
        text: &str,
    ) -> Result<Vec<f32>> {
        let url = format!("{}/api/embeddings", config.endpoint.trim_end_matches('/'));

        let body = serde_json::json!({
            "model": config.model,
            "prompt": text,
        });

        let resp = http
            .post(&url)
            .timeout(EMBEDDING_TIMEOUT)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Http(format!("embedding request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(Error::Provider {
                provider: config.provider.clone(),
                message: format!("embedding HTTP {status}: {body_text}"),
            });
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Http(format!("failed to parse embedding response: {e}")))?;

        let embedding = json
            .get("embedding")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Error::Provider {
                    provider: config.provider.clone(),
                    message: "response missing 'embedding' array".into(),
                }
            })?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        Ok(embedding)
    }

    /// Fetch embeddings for multiple texts sequentially.
    ///
    /// Uses the batch timeout for the overall operation. Individual requests
    /// use the standard embedding timeout.
    async fn fetch_embeddings_batch(
        http: &reqwest::Client,
        config: &ClassifierConfig,
        texts: &[&str],
    ) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            let embedding = Self::fetch_embedding(http, config, text).await?;
            results.push(embedding);
        }
        Ok(results)
    }

    /// Check whether the embedding endpoint is reachable.
    pub async fn health_check(&self) -> bool {
        let test_result =
            Self::fetch_embedding(&self.http, &self.config, "health check").await;
        test_result.is_ok()
    }

    /// Get a reference to the classifier config.
    pub fn config(&self) -> &ClassifierConfig {
        &self.config
    }

    /// Get a reference to the centroids.
    pub fn centroids(&self) -> &HashMap<ModelTier, Vec<f32>> {
        &self.centroids
    }

    // ── Cache helpers ──────────────────────────────────────────────

    /// Look up a cached embedding by prompt hash. Returns `None` if absent or expired.
    fn get_cached(&self, key: u64) -> Option<Vec<f32>> {
        let cache = self.cache.read();
        cache.get(&key).and_then(|entry| {
            if Instant::now() < entry.expires_at {
                Some(entry.embedding.clone())
            } else {
                None
            }
        })
    }

    /// Store an embedding in the cache. Evicts expired entries if over capacity.
    fn put_cached(&self, key: u64, embedding: &[f32]) {
        let ttl = Duration::from_secs(self.config.cache_ttl_secs);
        let entry = CachedEmbedding {
            embedding: embedding.to_vec(),
            expires_at: Instant::now() + ttl,
        };

        let mut cache = self.cache.write();

        // Evict expired entries when over capacity.
        if cache.len() >= CACHE_MAX_ENTRIES {
            let now = Instant::now();
            cache.retain(|_, v| v.expires_at > now);
        }

        cache.insert(key, entry);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Hash a prompt string to a u64 for cache lookup.
fn hash_prompt(prompt: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    prompt.hash(&mut hasher);
    hasher.finish()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "identical vectors should have similarity ~1.0, got {sim}"
        );
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 1e-6,
            "orthogonal vectors should have similarity ~0.0, got {sim}"
        );
    }

    #[test]
    fn cosine_similarity_opposite_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - (-1.0)).abs() < 1e-6,
            "opposite vectors should have similarity ~-1.0, got {sim}"
        );
    }

    #[test]
    fn cosine_similarity_zero_vector_returns_zero() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 1e-6,
            "zero vector should yield similarity 0.0, got {sim}"
        );
    }

    #[test]
    fn compute_centroid_single_vector() {
        let vectors = vec![vec![1.0, 2.0, 3.0]];
        let centroid = compute_centroid(&vectors);
        assert_eq!(centroid, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn compute_centroid_average() {
        let vectors = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0], vec![0.0, 0.0, 1.0]];
        let centroid = compute_centroid(&vectors);
        let expected = vec![1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0];
        for (a, b) in centroid.iter().zip(expected.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "centroid mismatch: got {a}, expected {b}"
            );
        }
    }

    #[test]
    fn compute_centroid_empty_returns_empty() {
        let vectors: Vec<Vec<f32>> = vec![];
        let centroid = compute_centroid(&vectors);
        assert!(centroid.is_empty());
    }

    #[test]
    fn classify_with_centroids_picks_nearest() {
        // Build centroids that are clearly separated in 3D space.
        let mut centroids = HashMap::new();
        centroids.insert(ModelTier::Simple, vec![1.0, 0.0, 0.0]);
        centroids.insert(ModelTier::Complex, vec![0.0, 1.0, 0.0]);
        centroids.insert(ModelTier::Reasoning, vec![0.0, 0.0, 1.0]);

        // A vector close to the Simple centroid.
        let embedding = vec![0.9, 0.1, 0.0];
        let (tier, scores) = classify_against_centroids(&embedding, &centroids);

        assert_eq!(tier, ModelTier::Simple);
        assert!(scores[&ModelTier::Simple] > scores[&ModelTier::Complex]);
        assert!(scores[&ModelTier::Simple] > scores[&ModelTier::Reasoning]);
    }

    #[test]
    fn classify_ambiguous_defaults_to_complex() {
        // Empty centroids should default to Complex.
        let centroids: HashMap<ModelTier, Vec<f32>> = HashMap::new();
        let embedding = vec![1.0, 2.0, 3.0];
        let (tier, _scores) = classify_against_centroids(&embedding, &centroids);
        assert_eq!(tier, ModelTier::Complex);
    }

    #[test]
    fn build_centroids_from_embeddings() {
        let mut embeddings = HashMap::new();
        embeddings.insert(
            ModelTier::Simple,
            vec![vec![1.0, 0.0], vec![0.8, 0.2]],
        );
        embeddings.insert(
            ModelTier::Complex,
            vec![vec![0.0, 1.0], vec![0.2, 0.8]],
        );

        let centroids = build_centroids(&embeddings);

        assert_eq!(centroids.len(), 2);

        let simple = &centroids[&ModelTier::Simple];
        assert!((simple[0] - 0.9).abs() < 1e-6);
        assert!((simple[1] - 0.1).abs() < 1e-6);

        let complex = &centroids[&ModelTier::Complex];
        assert!((complex[0] - 0.1).abs() < 1e-6);
        assert!((complex[1] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn default_reference_prompts_has_all_tiers() {
        let prompts = default_reference_prompts();
        assert!(prompts.contains_key(&ModelTier::Simple));
        assert!(prompts.contains_key(&ModelTier::Complex));
        assert!(prompts.contains_key(&ModelTier::Reasoning));
        // Each tier should have multiple reference prompts.
        for (_tier, texts) in &prompts {
            assert!(texts.len() >= 3, "each tier should have at least 3 reference prompts");
        }
    }

    #[test]
    fn apply_thresholds_escalates_low_simple_score() {
        let config = ClassifierConfig::default();
        let thresholds = RouterThresholds {
            simple_min_score: 0.6,
            complex_min_score: 0.5,
            reasoning_min_score: 0.55,
            escalate_token_threshold: 8000,
        };

        let classifier = EmbeddingClassifier::with_centroids(
            config,
            thresholds,
            HashMap::new(),
        );

        // Simple score below threshold -> should escalate to Complex.
        let mut scores = HashMap::new();
        scores.insert(ModelTier::Simple, 0.4_f32); // below 0.6
        scores.insert(ModelTier::Complex, 0.3_f32);
        scores.insert(ModelTier::Reasoning, 0.2_f32);

        let result = classifier.apply_thresholds(ModelTier::Simple, &scores, "short prompt");
        assert_eq!(result, ModelTier::Complex);
    }

    #[test]
    fn apply_thresholds_deescalates_low_reasoning_score() {
        let config = ClassifierConfig::default();
        let thresholds = RouterThresholds {
            simple_min_score: 0.6,
            complex_min_score: 0.5,
            reasoning_min_score: 0.55,
            escalate_token_threshold: 8000,
        };

        let classifier = EmbeddingClassifier::with_centroids(
            config,
            thresholds,
            HashMap::new(),
        );

        // Reasoning score below threshold -> should fall back to Complex.
        let mut scores = HashMap::new();
        scores.insert(ModelTier::Simple, 0.2_f32);
        scores.insert(ModelTier::Complex, 0.3_f32);
        scores.insert(ModelTier::Reasoning, 0.4_f32); // below 0.55

        let result = classifier.apply_thresholds(ModelTier::Reasoning, &scores, "short prompt");
        assert_eq!(result, ModelTier::Complex);
    }

    #[test]
    fn apply_thresholds_escalates_long_prompt() {
        let config = ClassifierConfig::default();
        let thresholds = RouterThresholds {
            simple_min_score: 0.3, // low threshold so score check won't trigger
            complex_min_score: 0.5,
            reasoning_min_score: 0.55,
            escalate_token_threshold: 100, // 100 tokens * 4 chars = 400 chars
        };

        let classifier = EmbeddingClassifier::with_centroids(
            config,
            thresholds,
            HashMap::new(),
        );

        let mut scores = HashMap::new();
        scores.insert(ModelTier::Simple, 0.9_f32);
        scores.insert(ModelTier::Complex, 0.3_f32);

        // A prompt longer than 400 chars should escalate Simple -> Complex.
        let long_prompt = "a".repeat(500);
        let result = classifier.apply_thresholds(ModelTier::Simple, &scores, &long_prompt);
        assert_eq!(result, ModelTier::Complex);
    }

    #[test]
    fn apply_thresholds_keeps_good_simple_score() {
        let config = ClassifierConfig::default();
        let thresholds = RouterThresholds::default();

        let classifier = EmbeddingClassifier::with_centroids(
            config,
            thresholds,
            HashMap::new(),
        );

        let mut scores = HashMap::new();
        scores.insert(ModelTier::Simple, 0.8_f32); // above 0.6 threshold
        scores.insert(ModelTier::Complex, 0.3_f32);
        scores.insert(ModelTier::Reasoning, 0.2_f32);

        let result = classifier.apply_thresholds(ModelTier::Simple, &scores, "short");
        assert_eq!(result, ModelTier::Simple);
    }

    #[test]
    fn cache_stores_and_retrieves() {
        let config = ClassifierConfig {
            cache_ttl_secs: 300,
            ..ClassifierConfig::default()
        };
        let classifier = EmbeddingClassifier::with_centroids(
            config,
            RouterThresholds::default(),
            HashMap::new(),
        );

        let key = hash_prompt("test prompt");
        let embedding = vec![1.0, 2.0, 3.0];

        classifier.put_cached(key, &embedding);
        let result = classifier.get_cached(key);

        assert!(result.is_some());
        assert_eq!(result.unwrap(), embedding);
    }

    #[test]
    fn cache_returns_none_for_missing() {
        let classifier = EmbeddingClassifier::with_centroids(
            ClassifierConfig::default(),
            RouterThresholds::default(),
            HashMap::new(),
        );

        let result = classifier.get_cached(999);
        assert!(result.is_none());
    }

    #[test]
    fn hash_prompt_deterministic() {
        let h1 = hash_prompt("hello world");
        let h2 = hash_prompt("hello world");
        let h3 = hash_prompt("different prompt");

        assert_eq!(h1, h2, "same input should produce same hash");
        assert_ne!(h1, h3, "different input should produce different hash");
    }
}
