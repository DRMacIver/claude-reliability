#!/bin/bash
# Pre-tool-use hook to prevent git commit --no-verify.
#
# Usage: Configure in .claude/settings.local.json:
# {
#   "hooks": {
#     "PreToolUse": ["./scripts/no-verify-hook.sh"]
#   }
# }

set -e

# Find the binary - try release first, then debug
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

BINARY=""
if [[ -x "$PROJECT_DIR/target/release/claude-reliability" ]]; then
    BINARY="$PROJECT_DIR/target/release/claude-reliability"
elif [[ -x "$PROJECT_DIR/target/debug/claude-reliability" ]]; then
    BINARY="$PROJECT_DIR/target/debug/claude-reliability"
else
    echo "Error: claude-reliability binary not found. Run 'cargo build --release'." >&2
    exit 1
fi

exec "$BINARY" pre-tool-use no-verify
