# LLM Router Design

**Date:** 2026-02-21
**Status:** Approved
**Inspiration:** [NadirClaw](https://github.com/doramirdor/NadirClaw)

## Problem

SerialAgent's current LLM routing is static: roles map to fixed `provider/model` strings in `config.toml`. Every scheduled task, agent turn, and API call uses the same model regardless of prompt complexity. This wastes money on simple prompts and underperforms on complex ones.

## Solution

An embedding-based smart router inside the gateway that classifies prompts (~10ms) and routes them to appropriate model tiers automatically.

## Architecture

```
Request
  |
  v
[Explicit model override?] --yes--> Use that model directly
  | no
  v
[Resolve routing profile]
  |
  +-- eco       --> Always use simple tier
  +-- premium   --> Always use complex tier
  +-- free      --> Always use free tier
  +-- reasoning --> Always use reasoning tier
  +-- auto      --> Classify via embeddings
                      |
                   [Embedding classifier]
                   Ollama / configurable provider
                   ~10ms latency
                      |
                   +--simple----> Tier 1 models
                   +--complex---> Tier 2 models
                   +--reasoning-> Tier 3 models
```

### Priority Order (unchanged by router)

1. Explicit `model` on request/schedule/agent -> bypass router entirely
2. Routing profile -> determines tier selection method
3. Tier -> ordered list of models, first available wins
4. Fallback -> if entire tier unavailable, fall back to next tier up

## Routing Profiles

| Profile     | Behavior                                    | Use Case                      |
|-------------|---------------------------------------------|-------------------------------|
| `auto`      | Embedding classifier picks tier             | Default for all requests      |
| `eco`       | Always simple tier                          | Batch jobs, low-priority      |
| `premium`   | Always complex tier                         | Critical tasks, user-facing   |
| `free`      | Always free/local tier                      | Cost-zero operations          |
| `reasoning` | Always reasoning tier (chain-of-thought)    | Planning, analysis, debugging |

## Embedding Classifier

### Approach

Use a lightweight embedding model to classify prompts into complexity tiers. Train/tune a small classifier (cosine similarity against reference embeddings for each tier) on labeled prompt examples.

### Reference Embeddings

Ship a set of reference prompts per tier:

- **Simple:** "What time is it?", "Summarize this text", "Translate to French", "List the files"
- **Complex:** "Analyze the performance bottleneck in this code", "Compare three architectural approaches", "Debug this race condition", "Write a comprehensive test suite"
- **Reasoning:** "Plan the implementation of a distributed cache", "Prove this algorithm is O(n log n)", "Design a migration strategy for the database schema"

At startup, embed all reference prompts and cache the vectors. At runtime, embed the incoming prompt, compute cosine similarity to each tier's centroid, pick the highest-scoring tier.

### Agentic Detection

If the prompt contains tool-calling patterns, multi-step instructions, or context > 8K tokens, auto-escalate to complex tier minimum.

### Configuration

```toml
[llm.router]
enabled = true
default_profile = "auto"

[llm.router.classifier]
provider = "ollama"
model = "nomic-embed-text"
endpoint = "http://localhost:11434"
cache_ttl_secs = 300            # cache embeddings for repeated prompts

[llm.router.tiers]
simple = ["deepseek/deepseek-chat", "google/gemini-2.0-flash"]
complex = ["anthropic/claude-sonnet-4-20250514", "openai/gpt-4o"]
reasoning = ["anthropic/claude-opus-4-6"]
free = ["venice/venice-uncensored"]

[llm.router.thresholds]
simple_min_score = 0.6          # minimum cosine similarity to classify as simple
complex_min_score = 0.5         # minimum for complex; below both = complex (safe default)
reasoning_min_score = 0.55      # minimum for reasoning
escalate_token_threshold = 8000 # auto-escalate if input > N tokens
```

## Code Changes

### New Crate: `crates/llm-router/`

Standalone crate with no gateway dependency. Contains:

- `classifier.rs` — Embedding-based prompt classifier
  - `EmbeddingClassifier` struct: holds reference embeddings, provider config
  - `classify(prompt: &str) -> Tier` — returns Simple/Complex/Reasoning
  - Cosine similarity computation
  - Reference embedding cache (computed once at startup)
- `router.rs` — Core routing logic
  - `SmartRouter` struct: holds classifier + tier config + profile
  - `route(prompt: &str, profile: Option<Profile>, explicit_model: Option<&str>) -> ResolvedModel`
  - Fallback chain within and across tiers
- `config.rs` — Router configuration types
  - `RouterConfig`, `ClassifierConfig`, `TierConfig`, `Thresholds`
- `types.rs` — Tier enum, Profile enum, RoutingDecision (for logging)
- `decisions.rs` — Recent decisions ring buffer for observability
- `mod.rs` — Public API

### Modified: `crates/providers/src/registry.rs`

- Add `SmartRouter` as optional field on `ProviderRegistry`
- New method: `resolve_model_smart(prompt, profile, explicit_model) -> (provider, model)`
- Falls back to existing role-based resolution if router disabled

### Modified: `crates/gateway/src/runtime/turn.rs`

- Before calling LLM, if no explicit model: call `registry.resolve_model_smart()`
- Log routing decision to the decisions ring buffer

### Modified: `crates/gateway/src/runtime/schedule_runner.rs`

- Pass schedule's routing profile (new field) to `resolve_model_smart()`

### Modified: `crates/domain/src/config/`

- Add `RouterConfig` to `LlmConfig`
- Add `routing_profile: Option<String>` to `AgentConfig` and schedule model

### New API Endpoints: `crates/gateway/src/api/router.rs`

```
GET  /v1/router/status     — classifier health, active profile, tier config
PUT  /v1/router/config     — update profile, tiers, classifier (persists to config)
POST /v1/router/classify   — test: send a prompt, get back tier + scores + model
GET  /v1/router/decisions  — last 100 routing decisions (prompt snippet, tier, model, latency_ms)
```

### Schedule & Agent: Profile Field

Add `routing_profile` (optional) to:
- `Schedule` struct (already has `model` — profile is used when model is None)
- `AgentConfig` in config.toml
- `CreateScheduleRequest` / `UpdateScheduleRequest`
- `TurnInput`

## Dashboard GUI

### Settings Page: New "LLM Router" Card

**View mode:**
- Active profile badge (Auto/Eco/Premium/Free/Reasoning)
- Classifier status: provider, model, connected/disconnected, avg classification latency
- Tier assignments table: 4 rows (Simple/Complex/Reasoning/Free), each showing ordered model list
- Collapsible "Recent Decisions" log: last 20 routing decisions with prompt snippet (first 80 chars), tier chosen, model used, classification latency

**Edit mode (via existing ConfigEditor or inline):**
- Default profile dropdown
- Classifier endpoint + model inputs
- Tier model lists: multi-select from available providers, drag to reorder priority
- Threshold sliders (simple_min_score, complex_min_score, reasoning_min_score, escalate_token_threshold)

### Schedule Edit Form

Add "Routing Profile" dropdown alongside existing "Model" field:
- Options: Default (inherit) / Auto / Eco / Premium / Free / Reasoning
- When "Model" is explicitly set, profile dropdown is disabled (greyed out)

### Agent Config (future)

Same profile dropdown per agent in agent settings.

### API Client (`api/client.ts`)

New methods:
```typescript
routerStatus(): Promise<RouterStatus>
updateRouterConfig(config: RouterConfigUpdate): Promise<RouterStatus>
classifyPrompt(prompt: string): Promise<ClassifyResult>
routerDecisions(limit?: number): Promise<Decision[]>
```

New types:
```typescript
interface RouterStatus {
  enabled: boolean
  default_profile: string
  classifier: { provider: string; model: string; connected: boolean; avg_latency_ms: number }
  tiers: Record<string, string[]>
  thresholds: Record<string, number>
}

interface ClassifyResult {
  tier: string
  scores: Record<string, number>
  resolved_model: string
  latency_ms: number
}

interface Decision {
  timestamp: string
  prompt_snippet: string
  profile: string
  tier: string
  model: string
  latency_ms: number
}
```

## Testing

- **Unit:** Classifier cosine similarity, tier resolution, fallback chains, profile overrides
- **Integration:** End-to-end classify endpoint, router status API
- **Embedding mock:** Test classifier with pre-computed vectors (no Ollama dependency in CI)

## Migration

- `[llm.router]` section is optional. If absent, existing role-based routing is unchanged.
- `enabled = false` (or section missing) means zero behavioral change.
- Existing `router_mode = "capability"` continues to work as-is.

## Risks

| Risk | Mitigation |
|------|-----------|
| Ollama unavailable | Fallback to complex tier (safe default) |
| Classification latency spike | Cache embeddings, 500ms timeout, fallback |
| Wrong tier selection | Decisions log for debugging, tunable thresholds |
| Config complexity | Sensible defaults, GUI makes it approachable |
