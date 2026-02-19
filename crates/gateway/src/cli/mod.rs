pub mod config;
pub mod doctor;

use clap::{Parser, Subcommand};

/// SerialAgent — an agentic AI gateway.
#[derive(Debug, Parser)]
#[command(name = "serialagent", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the gateway server (default when no subcommand is given).
    Serve,
    /// Run diagnostic checks against the current configuration.
    Doctor,
    /// Configuration utilities.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Print version information.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Parse the config file and report any errors.
    Validate,
    /// Dump the resolved configuration (with defaults) as TOML.
    Show,
    /// Store an API key in the OS keychain for a provider.
    SetSecret {
        /// Provider ID from config.toml.
        provider_id: String,
    },
    /// Read and display (masked) an API key from the OS keychain.
    GetSecret {
        /// Provider ID from config.toml.
        provider_id: String,
    },
}

// ── Config loading helper ─────────────────────────────────────────────

/// Load the configuration from the path specified by `SA_CONFIG` (or
/// `config.toml` by default).  Returns the parsed [`Config`] and the
/// path that was used.
///
/// This is shared by `serve`, `doctor`, and `config` subcommands so the
/// logic lives in one place.
pub fn load_config() -> anyhow::Result<(sa_domain::config::Config, String)> {
    let config_path =
        std::env::var("SA_CONFIG").unwrap_or_else(|_| "config.toml".into());

    let config = if std::path::Path::new(&config_path).exists() {
        let raw = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow::anyhow!("reading {config_path}: {e}"))?;
        toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("parsing {config_path}: {e}"))?
    } else {
        sa_domain::config::Config::default()
    };

    Ok((config, config_path))
}
