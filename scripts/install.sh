#!/usr/bin/env bash
# Builds token-proxy, installs it in ~/.local/bin and registers a LaunchAgent
# so it starts with every macOS session. Safe to run again to update.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LABEL="local.token-proxy"
BIN="$HOME/.local/bin/token-proxy"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG_DIR="$HOME/Library/Logs/token-proxy"
DOMAIN="gui/$(id -u)"
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

echo "→ Building the web UI"
cd "$ROOT/web"
[ -d node_modules ] || npm ci --no-audit --no-fund
npm run build --silent >/dev/null

echo "→ Building the binary"
cd "$ROOT"
cargo build --release --quiet

echo "→ Installing $BIN"
mkdir -p "$(dirname "$BIN")" "$LOG_DIR" "$(dirname "$PLIST")"
launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
install -m 755 "$ROOT/target/release/token-proxy" "$BIN"

cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <!-- Touch ID and the Keychain need the user's graphical session. -->
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>StandardOutPath</key>
  <string>$LOG_DIR/token-proxy.log</string>
  <key>StandardErrorPath</key>
  <string>$LOG_DIR/token-proxy.log</string>
</dict>
</plist>
EOF

echo "→ Starting the LaunchAgent"
echo "  (the new binary will ask for Keychain access: click \"Always Allow\")"
launchctl bootstrap "$DOMAIN" "$PLIST"

# Startup waits for the Keychain prompt answer, so allow up to two minutes.
for _ in $(seq 1 240); do
  if curl -fsS http://127.0.0.1:8788/api/status >/dev/null 2>&1; then
    echo "✓ token-proxy is running"
    echo "  Admin UI: http://127.0.0.1:8788"
    echo "  Proxy:    http://127.0.0.1:8787"
    exit 0
  fi
  sleep 0.5
done
echo "✗ token-proxy is not answering; see $LOG_DIR/token-proxy.log" >&2
exit 1
