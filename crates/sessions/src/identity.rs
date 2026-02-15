//! Identity linking — collapse the same person across channels.
//!
//! Maps many raw peer IDs to one canonical identity so "Alice on Telegram"
//! and "Alice on Discord" share the same DM session when desired.
//!
//! Input IDs should be prefixed: `telegram:123`, `discord:987`, `whatsapp:+33…`.
//! If an inbound peer matches any entry, `<peerId>` in the session key is
//! replaced with the canonical identity key (e.g. `alice`).

use std::collections::HashMap;

use sa_domain::config::IdentityLink;
use sa_domain::trace::TraceEvent;

/// Resolves raw peer IDs to canonical identities.
#[derive(Debug, Clone)]
pub struct IdentityResolver {
    /// peer_id → canonical
    map: HashMap<String, String>,
}

impl IdentityResolver {
    /// Build a resolver from the configured identity links.
    pub fn from_config(links: &[IdentityLink]) -> Self {
        let mut map = HashMap::new();
        for link in links {
            for pid in &link.peer_ids {
                map.insert(pid.clone(), link.canonical.clone());
            }
        }
        Self { map }
    }

    /// Resolve a raw peer ID.  If the peer matches a configured identity link,
    /// returns the canonical identity.  Otherwise returns the raw ID unchanged.
    pub fn resolve(&self, raw_peer_id: &str) -> String {
        if let Some(canonical) = self.map.get(raw_peer_id) {
            TraceEvent::IdentityResolved {
                raw_peer_id: raw_peer_id.to_owned(),
                canonical: canonical.clone(),
            }
            .emit();
            canonical.clone()
        } else {
            raw_peer_id.to_owned()
        }
    }

    /// Check whether the resolver has any configured links.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Number of raw peer IDs mapped.
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_known_peer() {
        let links = vec![IdentityLink {
            canonical: "alice".into(),
            peer_ids: vec!["telegram:123".into(), "discord:987".into()],
        }];
        let resolver = IdentityResolver::from_config(&links);
        assert_eq!(resolver.resolve("telegram:123"), "alice");
        assert_eq!(resolver.resolve("discord:987"), "alice");
    }

    #[test]
    fn resolve_unknown_peer() {
        let resolver = IdentityResolver::from_config(&[]);
        assert_eq!(resolver.resolve("telegram:999"), "telegram:999");
    }
}
