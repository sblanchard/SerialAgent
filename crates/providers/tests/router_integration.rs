//! Integration tests for the smart router — full round-trip without Ollama.
//!
//! These tests validate the complete routing flow across multiple modules
//! (smart_router + decisions) without requiring any external services.
//! All tests are pure and deterministic.

use chrono::Utc;
use sa_domain::config::{ModelTier, RoutingProfile, TierConfig};
use sa_providers::decisions::{Decision, DecisionLog};
use sa_providers::smart_router::resolve_model_for_request;
use std::time::Instant;

fn test_tiers() -> TierConfig {
    TierConfig {
        simple: vec!["deepseek/deepseek-chat".into()],
        complex: vec!["anthropic/claude-sonnet-4-20250514".into()],
        reasoning: vec!["anthropic/claude-opus-4-6".into()],
        free: vec!["venice/venice-uncensored".into()],
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Profile-to-model resolution
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn eco_profile_resolves_simple_tier_and_logs_decision() {
    let tiers = test_tiers();
    let decisions = DecisionLog::new(10);

    let start = Instant::now();
    let decision = resolve_model_for_request(None, RoutingProfile::Eco, None, &tiers);
    let latency_ms = start.elapsed().as_millis() as u64;

    assert_eq!(decision.model, "deepseek/deepseek-chat");
    assert_eq!(decision.tier, ModelTier::Simple);
    assert_eq!(decision.profile, RoutingProfile::Eco);
    assert!(!decision.bypassed);

    // Log the decision to the ring buffer and verify round-trip.
    decisions.record(Decision {
        timestamp: Utc::now(),
        prompt_snippet: "test prompt".into(),
        profile: decision.profile,
        tier: decision.tier,
        model: decision.model.clone(),
        latency_ms,
        bypassed: decision.bypassed,
    });

    let recent = decisions.recent(10);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].model, "deepseek/deepseek-chat");
    assert_eq!(recent[0].tier, ModelTier::Simple);
}

#[test]
fn premium_profile_resolves_complex_tier() {
    let tiers = test_tiers();
    let decision = resolve_model_for_request(None, RoutingProfile::Premium, None, &tiers);

    assert_eq!(decision.model, "anthropic/claude-sonnet-4-20250514");
    assert_eq!(decision.tier, ModelTier::Complex);
    assert!(!decision.bypassed);
}

#[test]
fn reasoning_profile_resolves_reasoning_tier() {
    let tiers = test_tiers();
    let decision = resolve_model_for_request(None, RoutingProfile::Reasoning, None, &tiers);

    assert_eq!(decision.model, "anthropic/claude-opus-4-6");
    assert_eq!(decision.tier, ModelTier::Reasoning);
    assert!(!decision.bypassed);
}

#[test]
fn free_profile_resolves_free_tier() {
    let tiers = test_tiers();
    let decision = resolve_model_for_request(None, RoutingProfile::Free, None, &tiers);

    assert_eq!(decision.model, "venice/venice-uncensored");
    assert_eq!(decision.tier, ModelTier::Free);
    assert!(!decision.bypassed);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Auto profile with classification
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn auto_profile_with_classified_tier_uses_classification() {
    let tiers = test_tiers();
    let decision = resolve_model_for_request(
        None,
        RoutingProfile::Auto,
        Some(ModelTier::Reasoning),
        &tiers,
    );

    assert_eq!(decision.model, "anthropic/claude-opus-4-6");
    assert_eq!(decision.tier, ModelTier::Reasoning);
    assert!(!decision.bypassed);
}

#[test]
fn auto_profile_without_classification_falls_back_to_complex() {
    let tiers = test_tiers();
    let decision = resolve_model_for_request(None, RoutingProfile::Auto, None, &tiers);

    assert_eq!(decision.model, "anthropic/claude-sonnet-4-20250514");
    assert_eq!(decision.tier, ModelTier::Complex);
    assert!(!decision.bypassed);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Explicit model bypass
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn explicit_model_bypasses_router() {
    let tiers = test_tiers();
    let decision = resolve_model_for_request(
        Some("custom/my-fine-tune"),
        RoutingProfile::Eco,
        Some(ModelTier::Simple),
        &tiers,
    );

    assert_eq!(decision.model, "custom/my-fine-tune");
    assert!(decision.bypassed);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Fallback behaviour
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn fallback_when_target_tier_empty() {
    let tiers = TierConfig {
        simple: vec![],
        complex: vec!["fallback-model".into()],
        reasoning: vec![],
        free: vec![],
    };

    let decision = resolve_model_for_request(
        None,
        RoutingProfile::Eco, // maps to Simple, which is empty
        None,
        &tiers,
    );

    assert_eq!(decision.model, "fallback-model");
    assert_eq!(decision.tier, ModelTier::Complex); // fell back to Complex
    assert!(!decision.bypassed);
}

#[test]
fn fallback_walks_full_chain_when_multiple_tiers_empty() {
    let tiers = TierConfig {
        simple: vec![],
        complex: vec![],
        reasoning: vec!["last-resort".into()],
        free: vec![],
    };

    let decision = resolve_model_for_request(
        None,
        RoutingProfile::Eco, // Simple -> fallback: Complex -> Reasoning
        None,
        &tiers,
    );

    assert_eq!(decision.model, "last-resort");
    assert_eq!(decision.tier, ModelTier::Reasoning);
    assert!(!decision.bypassed);
}

#[test]
fn all_tiers_empty_returns_empty_model() {
    let tiers = TierConfig::default(); // all vecs empty

    let decision = resolve_model_for_request(None, RoutingProfile::Eco, None, &tiers);

    assert!(decision.model.is_empty());
    assert!(!decision.bypassed);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Decision log round-trip
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn decision_log_round_trip_multiple_decisions() {
    let tiers = test_tiers();
    let decisions = DecisionLog::new(100);

    // Simulate multiple routing decisions.
    for profile in &[
        RoutingProfile::Eco,
        RoutingProfile::Premium,
        RoutingProfile::Reasoning,
    ] {
        let decision = resolve_model_for_request(None, *profile, None, &tiers);
        decisions.record(Decision {
            timestamp: Utc::now(),
            prompt_snippet: format!("test {:?}", profile),
            profile: decision.profile,
            tier: decision.tier,
            model: decision.model,
            latency_ms: 0,
            bypassed: decision.bypassed,
        });
    }

    let recent = decisions.recent(10);
    assert_eq!(recent.len(), 3);
    // Newest first: Reasoning, Premium, Eco
    assert_eq!(recent[0].tier, ModelTier::Reasoning);
    assert_eq!(recent[1].tier, ModelTier::Complex);
    assert_eq!(recent[2].tier, ModelTier::Simple);
}

#[test]
fn decision_log_capacity_evicts_oldest() {
    let tiers = test_tiers();
    let decisions = DecisionLog::new(2); // capacity of 2

    for profile in &[
        RoutingProfile::Eco,
        RoutingProfile::Premium,
        RoutingProfile::Reasoning,
    ] {
        let decision = resolve_model_for_request(None, *profile, None, &tiers);
        decisions.record(Decision {
            timestamp: Utc::now(),
            prompt_snippet: format!("test {:?}", profile),
            profile: decision.profile,
            tier: decision.tier,
            model: decision.model,
            latency_ms: 0,
            bypassed: decision.bypassed,
        });
    }

    let recent = decisions.recent(10);
    assert_eq!(recent.len(), 2);
    // Only the last two remain: Reasoning (newest) and Premium
    assert_eq!(recent[0].tier, ModelTier::Reasoning);
    assert_eq!(recent[1].tier, ModelTier::Complex);
}
