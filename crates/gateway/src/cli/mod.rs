pub mod chat;
pub mod config;
pub mod doctor;
pub mod import_cmd;
pub mod init;
pub mod login;
pub mod pid;
pub mod run;
pub mod systemd;

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
    /// Initialize a new SerialAgent project in the current directory.
    Init {
        /// Skip interactive prompts and use sensible defaults (OpenAI provider).
        #[arg(long)]
        defaults: bool,
    },
    /// Send a single message to the agent and print the response.
    Run {
        /// The message to send.
        message: String,
        /// Session key (defaults to "cli:run").
        #[arg(long, default_value = "cli:run")]
        session: String,
        /// Model override (e.g. "openai/gpt-4o").
        #[arg(long)]
        model: Option<String>,
        /// Output the full response as JSON instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// Print version information.
    Version,
    /// Systemd service management.
    #[command(subcommand)]
    Systemd(SystemdCommand),
    /// Import data from external systems (e.g. OpenClaw).
    #[command(subcommand)]
    Import(ImportCommand),
}

#[derive(Debug, Subcommand)]
pub enum SystemdCommand {
    /// Generate a systemd unit file and print it to stdout.
    Generate {
        /// Linux user to run the service as.
        #[arg(long, default_value = "serialagent")]
        user: String,
        /// Working directory for the service.
        #[arg(long)]
        working_dir: Option<String>,
        /// Path to the config file.
        #[arg(long, default_value = "config.toml")]
        config: String,
    },
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
    /// Authenticate with an OAuth provider (e.g. OpenAI Codex).
    Login {
        /// Provider ID from config.toml (must use oauth_device auth mode).
        provider_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ImportCommand {
    /// Preview an OpenClaw import (stage, scan, report).
    Preview {
        /// Path to the .openclaw directory (local).
        #[arg(long)]
        path: String,
        /// Include workspaces (default: true).
        #[arg(long, default_value = "true")]
        include_workspaces: bool,
        /// Include sessions (default: true).
        #[arg(long, default_value = "true")]
        include_sessions: bool,
        /// Include model configs.
        #[arg(long)]
        include_models: bool,
    },
    /// Apply a staged import.
    Apply {
        /// Staging ID from the preview step.
        staging_id: String,
        /// Merge strategy: merge_safe, replace, or skip_existing.
        #[arg(long, default_value = "merge_safe")]
        strategy: String,
    },
    /// List all staged imports.
    StagingList,
    /// Delete a staged import.
    StagingDelete {
        /// Staging ID to delete.
        id: String,
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
