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
