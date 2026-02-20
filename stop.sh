#!/usr/bin/env bash
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# SerialAgent — stop all services
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

ok()   { echo -e "${GREEN}[✓]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }

# Stop gateway
if pkill -f "serialagent serve" 2>/dev/null; then
    ok "Gateway stopped"
else
    warn "Gateway was not running"
fi

# Stop OpenBB
if pkill -f "openbb-api" 2>/dev/null; then
    ok "OpenBB stopped"
else
    warn "OpenBB was not running"
fi

# Stop SearXNG (Docker)
if docker stop serialagent-searxng 2>/dev/null; then
    ok "SearXNG stopped"
else
    warn "SearXNG was not running"
fi

# Stop SerialMemory
if pkill -f "serial.memory" 2>/dev/null || pkill -f "serialmemory" 2>/dev/null; then
    ok "SerialMemory stopped"
else
    warn "SerialMemory was not running"
fi

ok "All services stopped"
