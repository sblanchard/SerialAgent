//! Smart router resolution logic.
//!
//! Pure, synchronous functions that resolve a model string from routing
//! profiles, classified tiers, and tier configuration. No HTTP, no async
//! — just deterministic decision logic.

use sa_domain::config::{ModelTier, RoutingProfile, TierConfig};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The result of a routing decision.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub model: String,
    pub tier: ModelTier,
    pub profile: RoutingProfile,
    pub bypassed: bool,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public API
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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
pub fn resolve_tier_model(tier: ModelTier, tiers: &TierConfig) -> Option<&str> {
    let models = match tier {
        ModelTier::Simple => &tiers.simple,
        ModelTier::Complex => &tiers.complex,
        ModelTier::Reasoning => &tiers.reasoning,
        ModelTier::Free => &tiers.free,
    };
    models.first().map(|s| s.as_str())
}

/// Core resolution: explicit model > profile tier > classified tier > fallback.
///
/// Resolution order:
/// 1. If `explicit_model` is `Some`, bypass the router entirely.
/// 2. If the profile maps to a fixed tier, use that tier.
/// 3. If the profile is `Auto`, use the `classified_tier`.
/// 4. If no model is found in the chosen tier, walk the fallback chain.
pub fn resolve_model_for_request(
    explicit_model: Option<&str>,
    profile: RoutingProfile,
    classified_tier: Option<ModelTier>,
    tiers: &TierConfig,
) -> RoutingDecision {
    // 1. Explicit model bypass.
    if let Some(model) = explicit_model {
        return RoutingDecision {
            model: model.to_string(),
            tier: ModelTier::Complex, // sensible default for explicit
            profile,
            bypassed: true,
        };
    }

    // 2. Determine the target tier from profile or classification.
    let target_tier = profile_to_tier(profile)
        .or(classified_tier)
        .unwrap_or(ModelTier::Complex); // fallback default

    // 3. Try the target tier first, then walk fallbacks.
    if let Some(model) = resolve_tier_model(target_tier, tiers) {
        return RoutingDecision {
            model: model.to_string(),
            tier: target_tier,
            profile,
            bypassed: false,
        };
    }

    for fallback_tier in fallback_tiers(target_tier) {
        if let Some(model) = resolve_tier_model(fallback_tier, tiers) {
            return RoutingDecision {
                model: model.to_string(),
                tier: fallback_tier,
                profile,
                bypassed: false,
            };
        }
    }

    // 4. Absolute last resort — nothing configured anywhere.
    RoutingDecision {
        model: String::new(),
        tier: target_tier,
        profile,
        bypassed: false,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Internal helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Tier fallback order when the target tier has no models configured.
fn fallback_tiers(starting: ModelTier) -> Vec<ModelTier> {
    match starting {
        ModelTier::Simple => vec![ModelTier::Complex, ModelTier::Reasoning],
        ModelTier::Complex => vec![ModelTier::Reasoning, ModelTier::Simple],
        ModelTier::Reasoning => vec![ModelTier::Complex, ModelTier::Simple],
        ModelTier::Free => vec![ModelTier::Simple, ModelTier::Complex, ModelTier::Reasoning],
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tiers() -> TierConfig {
        TierConfig {
            simple: vec!["deepseek/deepseek-chat".into()],
            complex: vec!["anthropic/claude-sonnet-4-20250514".into()],
            reasoning: vec!["anthropic/claude-opus-4-6".into()],
            free: vec!["venice/venice-uncensored".into()],
        }
    }

    // ── resolve_tier_model ────────────────────────────────────────

    #[test]
    fn resolve_tier_model_picks_first_in_list() {
        let tiers = TierConfig {
            simple: vec!["model-a".into(), "model-b".into()],
            ..Default::default()
        };
        assert_eq!(resolve_tier_model(ModelTier::Simple, &tiers), Some("model-a"));
    }

    #[test]
    fn resolve_tier_model_empty_tier_returns_none() {
        let tiers = TierConfig::default();
        assert_eq!(resolve_tier_model(ModelTier::Simple, &tiers), None);
    }

    // ── profile_to_tier ───────────────────────────────────────────

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

    // ── resolve_model_for_request ─────────────────────────────────

    #[test]
    fn resolve_with_explicit_model_bypasses_router() {
        let tiers = test_tiers();
        let decision = resolve_model_for_request(
            Some("custom/my-model"),
            RoutingProfile::Auto,
            None,
            &tiers,
        );
        assert_eq!(decision.model, "custom/my-model");
        assert!(decision.bypassed);
    }

    #[test]
    fn resolve_with_eco_profile_uses_simple_tier() {
        let tiers = test_tiers();
        let decision = resolve_model_for_request(
            None,
            RoutingProfile::Eco,
            None,
            &tiers,
        );
        assert_eq!(decision.model, "deepseek/deepseek-chat");
        assert_eq!(decision.tier, ModelTier::Simple);
        assert!(!decision.bypassed);
    }

    #[test]
    fn resolve_with_auto_profile_uses_classified_tier() {
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
    fn resolve_falls_back_across_tiers() {
        // Simple tier is empty, should fall back to Complex.
        let tiers = TierConfig {
            simple: vec![],
            complex: vec!["fallback-model".into()],
            ..Default::default()
        };
        let decision = resolve_model_for_request(
            None,
            RoutingProfile::Eco, // maps to Simple
            None,
            &tiers,
        );
        assert_eq!(decision.model, "fallback-model");
        assert_eq!(decision.tier, ModelTier::Complex);
        assert!(!decision.bypassed);
    }
}
