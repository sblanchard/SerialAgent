use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Observability (OpenTelemetry) configuration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// OpenTelemetry observability configuration.
///
/// When `otlp_endpoint` is `None` (the default), no OTel exporter is
/// started and the gateway behaves exactly as before (structured JSON
/// logging only).  Setting `otlp_endpoint` enables OTLP/gRPC trace
/// export so that every `tracing` span is also forwarded to a
/// collector (Jaeger, Grafana Tempo, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// OTLP gRPC endpoint (e.g. `http://localhost:4317`).
    /// When `None`, OpenTelemetry export is disabled.
    #[serde(default)]
    pub otlp_endpoint: Option<String>,

    /// The `service.name` resource attribute reported to the collector.
    #[serde(default = "d_service_name")]
    pub service_name: String,

    /// Trace sampling rate (`0.0` = never, `1.0` = always).
    /// Uses `TraceIdRatioBased` sampling so the decision is consistent
    /// across an entire trace.
    #[serde(default = "d_sample_rate")]
    pub sample_rate: f64,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: None,
            service_name: d_service_name(),
            sample_rate: d_sample_rate(),
        }
    }
}

fn d_service_name() -> String {
    "serialagent".into()
}

fn d_sample_rate() -> f64 {
    1.0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_no_endpoint() {
        let cfg = ObservabilityConfig::default();
        assert!(cfg.otlp_endpoint.is_none());
    }

    #[test]
    fn default_service_name_is_serialagent() {
        let cfg = ObservabilityConfig::default();
        assert_eq!(cfg.service_name, "serialagent");
    }

    #[test]
    fn default_sample_rate_is_one() {
        let cfg = ObservabilityConfig::default();
        assert!((cfg.sample_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn deserialize_empty_uses_defaults() {
        let cfg: ObservabilityConfig = toml::from_str("").unwrap();
        assert!(cfg.otlp_endpoint.is_none());
        assert_eq!(cfg.service_name, "serialagent");
        assert!((cfg.sample_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn deserialize_with_endpoint() {
        let toml_str = r#"
            otlp_endpoint = "http://localhost:4317"
            service_name = "my-service"
            sample_rate = 0.5
        "#;
        let cfg: ObservabilityConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            cfg.otlp_endpoint.as_deref(),
            Some("http://localhost:4317")
        );
        assert_eq!(cfg.service_name, "my-service");
        assert!((cfg.sample_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn serde_roundtrip() {
        let cfg = ObservabilityConfig {
            otlp_endpoint: Some("http://otel:4317".into()),
            service_name: "test-svc".into(),
            sample_rate: 0.25,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: ObservabilityConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.otlp_endpoint, cfg.otlp_endpoint);
        assert_eq!(deserialized.service_name, cfg.service_name);
        assert!((deserialized.sample_rate - cfg.sample_rate).abs() < f64::EPSILON);
    }
}
