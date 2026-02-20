#!/usr/bin/env bash
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# SerialAgent — one-click build & launch
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

GATEWAY_PORT=3210
MEMORY_PORT=4545
LOG_DIR="$ROOT/data/logs"
mkdir -p "$LOG_DIR"

# ── Colors ────────────────────────────────────────────────────────────
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[serialagent]${NC} $1"; }
ok()   { echo -e "${GREEN}[✓]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }

# ── 1. Check SerialMemory ────────────────────────────────────────────
if ss -tlnp 2>/dev/null | grep -q ":${MEMORY_PORT} "; then
    ok "SerialMemory already running on :${MEMORY_PORT}"
else
    MEMORY_DIR="$HOME/Projects/SerialMemoryServer"
    if [ -d "$MEMORY_DIR" ]; then
        log "Starting SerialMemory..."
        (cd "$MEMORY_DIR" && nohup cargo run --release > "$LOG_DIR/serialmemory.log" 2>&1 &)
        sleep 3
        if ss -tlnp 2>/dev/null | grep -q ":${MEMORY_PORT} "; then
            ok "SerialMemory started on :${MEMORY_PORT}"
        else
            warn "SerialMemory may still be building — check $LOG_DIR/serialmemory.log"
        fi
    else
        warn "SerialMemory not found at $MEMORY_DIR — skipping (memory features disabled)"
    fi
fi

# ── 2. Build dashboard ───────────────────────────────────────────────
log "Building dashboard..."
(cd apps/dashboard && npm run build --silent 2>&1) || {
    warn "Dashboard build failed — gateway will run without UI"
}
ok "Dashboard built"

# ── 3. Build gateway ─────────────────────────────────────────────────
log "Building gateway..."
cargo build -p sa-gateway 2>&1 | tail -1
ok "Gateway built"

# ── 4. Stop old gateway if running ───────────────────────────────────
if ss -tlnp 2>/dev/null | grep -q ":${GATEWAY_PORT} "; then
    log "Stopping old gateway..."
    pkill -f "serialagent serve" 2>/dev/null || true
    sleep 1
fi

# ── 5. Start gateway ─────────────────────────────────────────────────
log "Starting gateway..."
nohup ./target/debug/serialagent serve > "$LOG_DIR/gateway.log" 2>&1 &
GATEWAY_PID=$!

# Wait for it to bind
for i in $(seq 1 10); do
    if ss -tlnp 2>/dev/null | grep -q ":${GATEWAY_PORT} "; then
        break
    fi
    sleep 0.5
done

if ss -tlnp 2>/dev/null | grep -q ":${GATEWAY_PORT} "; then
    ok "Gateway running on http://127.0.0.1:${GATEWAY_PORT} (PID $GATEWAY_PID)"
else
    warn "Gateway failed to start — check $LOG_DIR/gateway.log"
    exit 1
fi

# ── 6. Open browser ──────────────────────────────────────────────────
URL="http://localhost:${GATEWAY_PORT}/app"
log "Opening $URL"
xdg-open "$URL" 2>/dev/null || open "$URL" 2>/dev/null || warn "Open $URL in your browser"

echo ""
ok "SerialAgent is running!"
echo "   Dashboard:  $URL"
echo "   API:        http://localhost:${GATEWAY_PORT}/v1/health"
echo "   Logs:       $LOG_DIR/"
echo "   Stop:       pkill -f 'serialagent serve'"
