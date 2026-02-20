use std::path::Path;

// ── Provider presets ─────────────────────────────────────────────────

struct ProviderPreset {
    id: &'static str,
    base_url: &'static str,
    env_var: &'static str,
}

const OPENAI: ProviderPreset = ProviderPreset {
    id: "openai",
    base_url: "https://api.openai.com/v1",
    env_var: "OPENAI_API_KEY",
};

const ANTHROPIC: ProviderPreset = ProviderPreset {
    id: "anthropic",
    base_url: "https://api.anthropic.com/v1",
    env_var: "ANTHROPIC_API_KEY",
};

const GOOGLE: ProviderPreset = ProviderPreset {
    id: "google",
    base_url: "https://generativelanguage.googleapis.com/v1beta",
    env_var: "GOOGLE_API_KEY",
};

// ── Public entry point ───────────────────────────────────────────────

/// Scaffold a new SerialAgent project in the current directory.
///
/// When `use_defaults` is `true` the OpenAI preset is used without any
/// interactive prompts.  Otherwise the user is asked to pick a provider.
pub fn init(use_defaults: bool) -> anyhow::Result<()> {
    init_in(Path::new("."), use_defaults)
}

// ── Core implementation (directory-parameterised for testability) ─────

fn init_in(base: &Path, use_defaults: bool) -> anyhow::Result<()> {
    let config_path = base.join("config.toml");

    if config_path.exists() {
        anyhow::bail!(
            "config.toml already exists. Use a different directory or remove it first."
        );
    }

    let (provider_id, base_url, env_var) = if use_defaults {
        (
            OPENAI.id.to_owned(),
            OPENAI.base_url.to_owned(),
            OPENAI.env_var.to_owned(),
        )
    } else {
        prompt_provider()?
    };

    // ── Generate files ───────────────────────────────────────────────
    let config_content = render_config(&provider_id, &base_url, &env_var);
    let env_content = render_dotenv(&env_var);

    std::fs::write(&config_path, config_content)?;
    std::fs::write(base.join(".env"), env_content)?;

    // ── Create directories ───────────────────────────────────────────
    std::fs::create_dir_all(base.join("workspace"))?;
    std::fs::create_dir_all(base.join("data/state"))?;
    std::fs::create_dir_all(base.join("skills"))?;

    // ── Success message ──────────────────────────────────────────────
    eprintln!();
    eprintln!("  SerialAgent project initialized!");
    eprintln!();
    eprintln!("  Created:");
    eprintln!("    config.toml   - gateway configuration");
    eprintln!("    .env          - environment variables (add your API key)");
    eprintln!("    workspace/    - agent workspace directory");
    eprintln!("    data/state/   - persistent state storage");
    eprintln!("    skills/       - custom skill definitions");
    eprintln!();
    eprintln!("  Next steps:");
    eprintln!("    1. Add your API key to .env");
    eprintln!("    2. Run `serialagent doctor` to verify the setup");
    eprintln!("    3. Run `serialagent` to start the gateway");
    eprintln!();

    Ok(())
}

// ── Interactive provider selection ───────────────────────────────────

fn prompt_provider() -> anyhow::Result<(String, String, String)> {
    eprintln!();
    eprintln!("  Welcome to SerialAgent!");
    eprintln!("  Let's set up your project.\n");

    let choice = prompt(
        "  Which LLM provider?\n  [1] OpenAI  [2] Anthropic  [3] Google  [4] Other\n  >",
    );

    match choice.as_str() {
        "1" => Ok((
            OPENAI.id.to_owned(),
            OPENAI.base_url.to_owned(),
            OPENAI.env_var.to_owned(),
        )),
        "2" => Ok((
            ANTHROPIC.id.to_owned(),
            ANTHROPIC.base_url.to_owned(),
            ANTHROPIC.env_var.to_owned(),
        )),
        "3" => Ok((
            GOOGLE.id.to_owned(),
            GOOGLE.base_url.to_owned(),
            GOOGLE.env_var.to_owned(),
        )),
        "4" => prompt_custom_provider(),
        _ => {
            eprintln!("  Invalid choice, defaulting to OpenAI.");
            Ok((
                OPENAI.id.to_owned(),
                OPENAI.base_url.to_owned(),
                OPENAI.env_var.to_owned(),
            ))
        }
    }
}

fn prompt_custom_provider() -> anyhow::Result<(String, String, String)> {
    let provider_id = prompt("  Provider ID (e.g. \"my-llm\"):");
    let base_url = prompt("  Base URL (e.g. \"https://api.example.com/v1\"):");
    let env_var = prompt("  Environment variable for the API key (e.g. \"MY_LLM_API_KEY\"):");

    if provider_id.is_empty() || base_url.is_empty() || env_var.is_empty() {
        anyhow::bail!("All fields are required for a custom provider.");
    }

    Ok((provider_id, base_url, env_var))
}

fn prompt(question: &str) -> String {
    eprint!("{question} ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap_or_default();
    input.trim().to_string()
}

// ── Template rendering ───────────────────────────────────────────────

fn render_config(provider_id: &str, base_url: &str, env_var: &str) -> String {
    format!(
        r#"# SerialAgent configuration
# See docs for all options: https://serialagent.dev/docs/config

[server]
port = 3210
host = "127.0.0.1"

[llm]
default_model = "{provider_id}/default"

[[llm.providers]]
id = "{provider_id}"
base_url = "{base_url}"

[llm.providers.auth]
mode = "env"
env_var = "{env_var}"

[workspace]
# path = "./workspace"

[sessions]
agent_id = "default"
"#
    )
}

fn render_dotenv(env_var: &str) -> String {
    format!(
        "# SerialAgent environment variables\n{env_var}=your-api-key-here\n"
    )
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_config_contains_provider_fields() {
        let output = render_config("openai", "https://api.openai.com/v1", "OPENAI_API_KEY");

        assert!(output.contains("id = \"openai\""));
        assert!(output.contains("base_url = \"https://api.openai.com/v1\""));
        assert!(output.contains("env_var = \"OPENAI_API_KEY\""));
        assert!(output.contains("default_model = \"openai/default\""));
    }

    #[test]
    fn render_config_contains_structure() {
        let output = render_config("test", "https://example.com", "TEST_KEY");

        assert!(output.contains("[server]"));
        assert!(output.contains("port = 3210"));
        assert!(output.contains("[llm]"));
        assert!(output.contains("[[llm.providers]]"));
        assert!(output.contains("[llm.providers.auth]"));
        assert!(output.contains("[workspace]"));
        assert!(output.contains("[sessions]"));
    }

    #[test]
    fn render_dotenv_contains_env_var() {
        let output = render_dotenv("OPENAI_API_KEY");

        assert!(output.contains("OPENAI_API_KEY=your-api-key-here"));
        assert!(output.starts_with("# SerialAgent environment variables"));
    }

    #[test]
    fn init_fails_when_config_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "existing").unwrap();

        let result = init_in(dir.path(), true);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("config.toml already exists")
        );
    }

    #[test]
    fn init_defaults_creates_expected_files() {
        let dir = tempfile::tempdir().unwrap();

        let result = init_in(dir.path(), true);
        assert!(result.is_ok());

        assert!(dir.path().join("config.toml").exists());
        assert!(dir.path().join(".env").exists());
        assert!(dir.path().join("workspace").is_dir());
        assert!(dir.path().join("data/state").is_dir());
        assert!(dir.path().join("skills").is_dir());

        let config = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(config.contains("id = \"openai\""));
        assert!(config.contains("env_var = \"OPENAI_API_KEY\""));

        let dotenv = std::fs::read_to_string(dir.path().join(".env")).unwrap();
        assert!(dotenv.contains("OPENAI_API_KEY=your-api-key-here"));
    }
}
