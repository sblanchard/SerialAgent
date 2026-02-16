use sa_domain::config::Config;

#[test]
fn default_host_is_localhost() {
    let config = Config::default();
    assert_eq!(config.server.host, "127.0.0.1");
}

#[test]
fn explicit_zero_host_parses() {
    let toml_str = r#"
[server]
host = "0.0.0.0"
port = 3210
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.server.host, "0.0.0.0");
}

#[test]
fn default_cors_allows_only_localhost() {
    let config = Config::default();
    assert!(!config.server.cors.allowed_origins.is_empty());
    assert!(config.server.cors.allowed_origins.contains(&"http://localhost:*".to_string()));
    assert!(config.server.cors.allowed_origins.contains(&"http://127.0.0.1:*".to_string()));
}

#[test]
fn cors_config_parses_custom_origins() {
    let toml_str = r#"
[server.cors]
allowed_origins = ["https://myapp.com", "http://localhost:3000"]
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.server.cors.allowed_origins.len(), 2);
    assert!(config.server.cors.allowed_origins.contains(&"https://myapp.com".to_string()));
}

#[test]
fn cors_wildcard_port_preserved_in_config() {
    let toml_str = r#"
[server.cors]
allowed_origins = ["http://localhost:*"]
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert_eq!(config.server.cors.allowed_origins[0], "http://localhost:*");
}

#[test]
fn admin_token_env_default() {
    let config = Config::default();
    assert_eq!(config.admin.token_env, "SA_ADMIN_TOKEN");
}
