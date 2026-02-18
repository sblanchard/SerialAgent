use sa_domain::config::{Config, ConfigSeverity};

/// Run all diagnostic checks and print a summary.
///
/// Returns `Ok(true)` when every check passes, `Ok(false)` when at least
/// one check failed.
pub async fn run(config: &Config, config_path: &str) -> anyhow::Result<bool> {
    println!("serialagent doctor");
    println!("==================\n");

    let mut all_passed = true;

    // 1. Config file
    check_config_file(config_path, &mut all_passed);

    // 2. Config validation
    check_config_validation(config, &mut all_passed);

    // 3. SerialMemory connectivity
    check_serial_memory(config, &mut all_passed).await;

    // 4. LLM providers
    check_llm_providers(config, &mut all_passed);

    // 5. Workspace directory
    check_workspace(config, &mut all_passed);

    // Summary
    println!();
    if all_passed {
        println!("All checks passed.");
    } else {
        println!("Some checks failed. Review the output above.");
    }

    Ok(all_passed)
}

// ── Individual checks ─────────────────────────────────────────────────

fn check_config_file(config_path: &str, all_passed: &mut bool) {
    let exists = std::path::Path::new(config_path).exists();
    print_check(
        "Config file exists",
        exists,
        if exists {
            config_path.to_owned()
        } else {
            format!("{config_path} not found (using defaults)")
        },
    );
    if !exists {
        *all_passed = false;
    }
}

fn check_config_validation(config: &Config, all_passed: &mut bool) {
    let issues = config.validate();
    let error_count = issues
        .iter()
        .filter(|e| e.severity == ConfigSeverity::Error)
        .count();

    if issues.is_empty() {
        print_check("Config validation", true, "no issues".into());
    } else {
        print_check(
            "Config validation",
            error_count == 0,
            format!(
                "{} issue(s) ({} error(s))",
                issues.len(),
                error_count,
            ),
        );
        for issue in &issues {
            println!("      {issue}");
        }
        if error_count > 0 {
            *all_passed = false;
        }
    }
}

async fn check_serial_memory(config: &Config, all_passed: &mut bool) {
    let url = &config.serial_memory.base_url;
    let reachable = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(client) => client.get(url).send().await.is_ok(),
        Err(_) => false,
    };

    print_check(
        "SerialMemory reachable",
        reachable,
        if reachable {
            url.clone()
        } else {
            format!("{url} (unreachable)")
        },
    );

    if !reachable {
        *all_passed = false;
    }
}

fn check_llm_providers(config: &Config, all_passed: &mut bool) {
    let count = config.llm.providers.len();
    let ok = count > 0;

    print_check(
        "LLM providers configured",
        ok,
        if ok {
            format!("{count} provider(s)")
        } else {
            "none configured".into()
        },
    );

    if !ok {
        *all_passed = false;
    }
}

fn check_workspace(config: &Config, all_passed: &mut bool) {
    let path = &config.workspace.path;
    let exists = path.exists();
    let writable = if exists {
        // Try creating a temp file to verify write access.
        let probe = path.join(".serialagent_doctor_probe");
        let w = std::fs::write(&probe, b"probe").is_ok();
        let _ = std::fs::remove_file(&probe);
        w
    } else {
        false
    };

    let ok = exists && writable;
    let detail = match (exists, writable) {
        (true, true) => format!("{} (writable)", path.display()),
        (true, false) => format!("{} (not writable)", path.display()),
        _ => format!("{} (does not exist)", path.display()),
    };

    print_check("Workspace directory", ok, detail);

    if !ok {
        *all_passed = false;
    }
}

// ── Formatting helper ─────────────────────────────────────────────────

fn print_check(name: &str, passed: bool, detail: String) {
    let status = if passed { "PASS" } else { "FAIL" };
    println!("  [{status}] {name}: {detail}");
}
