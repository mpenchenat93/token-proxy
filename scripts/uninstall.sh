#!/usr/bin/env bash
# Stops token-proxy and removes the LaunchAgent and the binary.
# The routes stay in the Keychain (item "token-proxy").
set -euo pipefail

LABEL="local.token-proxy"
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/$LABEL.plist" "$HOME/.local/bin/token-proxy"
echo "✓ token-proxy uninstalled (the routes stay in the Keychain)"
