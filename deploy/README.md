# SerialAgent Service Deployment

This directory contains service templates for running SerialAgent as a managed background service on Linux (systemd) and macOS (launchd).

Both templates assume the following installation layout:

```
/opt/serialagent/
  serialagent          # binary
  config.toml          # configuration
  data/                # runtime state
  workspace/           # workspace files
  logs/                # log files (macOS only)
  .env                 # environment secrets (optional, Linux)
```

---

## Prerequisites

Build the release binary:

```bash
cargo build --release --bin serialagent
```

The binary will be at `target/release/serialagent`.

---

## Linux (systemd)

### Install

```bash
# Create a dedicated system user
sudo useradd --system --no-create-home --shell /usr/sbin/nologin serialagent

# Create the installation directory
sudo mkdir -p /opt/serialagent/{data,workspace}
sudo cp target/release/serialagent /opt/serialagent/
sudo cp config.toml /opt/serialagent/
sudo chown -R serialagent:serialagent /opt/serialagent

# (Optional) Create an environment file for secrets
sudo tee /opt/serialagent/.env > /dev/null <<'EOF'
SA_CONFIG=/opt/serialagent/config.toml
SA_API_TOKEN=your-api-token-here
SA_ADMIN_TOKEN=your-admin-token-here
RUST_LOG=info,sa_gateway=debug
EOF
sudo chmod 600 /opt/serialagent/.env
sudo chown serialagent:serialagent /opt/serialagent/.env

# Install the unit file
sudo cp deploy/serialagent.service /etc/systemd/system/
sudo systemctl daemon-reload
```

If you use the environment file, uncomment the `EnvironmentFile=` line in the unit file.

### Enable and start

```bash
sudo systemctl enable serialagent   # start on boot
sudo systemctl start serialagent    # start now
```

### View logs

```bash
# Follow live logs
sudo journalctl -u serialagent -f

# Show logs since last boot
sudo journalctl -u serialagent -b

# Show last 100 lines
sudo journalctl -u serialagent -n 100
```

### Check status

```bash
sudo systemctl status serialagent
```

### Restart / stop

```bash
sudo systemctl restart serialagent
sudo systemctl stop serialagent
```

### Uninstall

```bash
sudo systemctl stop serialagent
sudo systemctl disable serialagent
sudo rm /etc/systemd/system/serialagent.service
sudo systemctl daemon-reload

# Remove installation (optional)
sudo userdel serialagent
sudo rm -rf /opt/serialagent
```

---

## macOS (launchd)

### Install

```bash
# Create the installation directory
sudo mkdir -p /opt/serialagent/{data,workspace,logs}
sudo cp target/release/serialagent /opt/serialagent/
sudo cp config.toml /opt/serialagent/

# Install the plist
# System-wide (runs as root):
sudo cp deploy/com.serialagent.plist /Library/LaunchDaemons/

# Or current-user only (runs as your user):
# cp deploy/com.serialagent.plist ~/Library/LaunchAgents/
```

Edit the plist to set your `SA_API_TOKEN` and `SA_ADMIN_TOKEN` values by uncommenting the relevant XML blocks.

### Enable and start

```bash
# System-wide daemon
sudo launchctl load /Library/LaunchDaemons/com.serialagent.plist

# Or current-user agent
# launchctl load ~/Library/LaunchAgents/com.serialagent.plist
```

The service starts immediately (`RunAtLoad=true`) and restarts automatically (`KeepAlive=true`).

### View logs

```bash
# stdout
tail -f /opt/serialagent/logs/serialagent.stdout.log

# stderr
tail -f /opt/serialagent/logs/serialagent.stderr.log
```

### Check status

```bash
# System-wide
sudo launchctl list | grep com.serialagent

# Current-user
# launchctl list | grep com.serialagent
```

A `0` in the status column means the process exited cleanly; a PID means it is running.

### Restart / stop

```bash
# Stop (system-wide)
sudo launchctl unload /Library/LaunchDaemons/com.serialagent.plist
# Start again
sudo launchctl load /Library/LaunchDaemons/com.serialagent.plist
```

### Uninstall

```bash
# System-wide
sudo launchctl unload /Library/LaunchDaemons/com.serialagent.plist
sudo rm /Library/LaunchDaemons/com.serialagent.plist

# Or current-user
# launchctl unload ~/Library/LaunchAgents/com.serialagent.plist
# rm ~/Library/LaunchAgents/com.serialagent.plist

# Remove installation (optional)
sudo rm -rf /opt/serialagent
```

---

## Environment Variables

| Variable | Description | Default |
|---|---|---|
| `SA_CONFIG` | Path to `config.toml` | `config.toml` (cwd) |
| `SA_API_TOKEN` | Bearer token for API authentication | (none) |
| `SA_ADMIN_TOKEN` | Bearer token for admin endpoints | (none) |
| `RUST_LOG` | Log level filter | `info,sa_gateway=debug` |

---

## Updating the Binary

```bash
# Build the new version
cargo build --release --bin serialagent

# Linux
sudo cp target/release/serialagent /opt/serialagent/
sudo systemctl restart serialagent

# macOS
sudo cp target/release/serialagent /opt/serialagent/
sudo launchctl unload /Library/LaunchDaemons/com.serialagent.plist
sudo launchctl load /Library/LaunchDaemons/com.serialagent.plist
```
