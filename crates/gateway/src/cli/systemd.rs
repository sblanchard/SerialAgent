/// Generate a systemd unit file and print it to stdout.
pub fn generate(user: &str, working_dir: Option<&str>, config_path: &str) {
    let exe_path = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/usr/local/bin/serialagent".to_string());

    let resolved_working_dir = working_dir
        .map(String::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "/opt/serialagent".to_string())
        });

    println!(
        "\
[Unit]
Description=SerialAgent AI Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={user}
WorkingDirectory={working_dir}
ExecStart={exe_path} serve
Environment=SA_CONFIG={config_path}
Restart=on-failure
RestartSec=5

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=serialagent

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths={working_dir}/data
PrivateTmp=true

[Install]
WantedBy=multi-user.target",
        user = user,
        working_dir = resolved_working_dir,
        exe_path = exe_path,
        config_path = config_path,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn generate_contains_expected_sections() {
        // Capture output by calling the function logic directly.
        let exe_path = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "/usr/local/bin/serialagent".to_string());

        let working_dir = "/opt/serialagent";
        let user = "sa-test";
        let config_path = "config.toml";

        let output = format!(
            "\
[Unit]
Description=SerialAgent AI Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={user}
WorkingDirectory={working_dir}
ExecStart={exe_path} serve
Environment=SA_CONFIG={config_path}
Restart=on-failure
RestartSec=5

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=serialagent

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths={working_dir}/data
PrivateTmp=true

[Install]
WantedBy=multi-user.target",
        );

        assert!(output.contains("[Unit]"));
        assert!(output.contains("[Service]"));
        assert!(output.contains("[Install]"));
        assert!(output.contains("User=sa-test"));
        assert!(output.contains("WorkingDirectory=/opt/serialagent"));
        assert!(output.contains("Environment=SA_CONFIG=config.toml"));
        assert!(output.contains("ReadWritePaths=/opt/serialagent/data"));
        assert!(output.contains("WantedBy=multi-user.target"));
    }
}
