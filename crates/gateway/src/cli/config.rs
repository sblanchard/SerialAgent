use sa_domain::config::{Config, ConfigSeverity};

/// Parse and validate the config, printing any issues.
///
/// Exits with code 0 when valid, code 1 when errors are found.
pub fn validate(config: &Config, config_path: &str) -> bool {
    let issues = config.validate();

    if issues.is_empty() {
        println!("Config OK ({config_path})");
        return true;
    }

    let error_count = issues
        .iter()
        .filter(|e| e.severity == ConfigSeverity::Error)
        .count();
    let warning_count = issues.len() - error_count;

    for issue in &issues {
        println!("{issue}");
    }

    println!(
        "\n{} error(s), {} warning(s) in {config_path}",
        error_count, warning_count,
    );

    error_count == 0
}

/// Dump the resolved config (with all defaults filled in) as TOML.
pub fn show(config: &Config) {
    match toml::to_string_pretty(config) {
        Ok(output) => print!("{output}"),
        Err(e) => {
            eprintln!("Failed to serialize config: {e}");
            std::process::exit(1);
        }
    }
}
