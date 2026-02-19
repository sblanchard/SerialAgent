//! MCP configuration types — re-exported from `sa-domain`.
//!
//! The canonical definitions live in `sa_domain::config` so that the
//! gateway config deserializer can include them without depending on
//! the full MCP client crate.

pub use sa_domain::config::{McpConfig, McpServerConfig, McpTransportKind};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_defaults() {
        let cfg: McpConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn deserialize_server_config() {
        let raw = r#"{
            "id": "filesystem",
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            "transport": "stdio"
        }"#;
        let cfg: McpServerConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.id, "filesystem");
        assert_eq!(cfg.command, "npx");
        assert_eq!(cfg.args.len(), 3);
        assert_eq!(cfg.transport, McpTransportKind::Stdio);
    }

    #[test]
    fn transport_kind_defaults_to_stdio() {
        let raw = r#"{ "id": "test", "command": "echo" }"#;
        let cfg: McpServerConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.transport, McpTransportKind::Stdio);
    }

    #[test]
    fn sse_transport() {
        let raw = r#"{ "id": "remote", "transport": "sse", "url": "http://localhost:8080/sse" }"#;
        let cfg: McpServerConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.transport, McpTransportKind::Sse);
        assert_eq!(cfg.url.as_deref(), Some("http://localhost:8080/sse"));
    }

    #[test]
    fn deserialize_with_env() {
        let raw = r#"{
            "id": "test",
            "command": "node",
            "args": ["server.js"],
            "env": { "NODE_ENV": "production" }
        }"#;
        let cfg: McpServerConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.env.get("NODE_ENV").unwrap(), "production");
    }
}
