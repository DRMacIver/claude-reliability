#!/usr/bin/env bash
# user-prompt-submit.sh - UserPromptSubmit hook wrapper for claude-reliability
#
# Ensures the binary is available and runs the user-prompt-submit hook.
# This hook resets session state when the user sends a new message.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-binary.sh"

# Get the binary path (this may build it if source changed)
BINARY=$("$ENSURE_BINARY" 2>/dev/null) || {
    # Build failed - allow silently
    exit 0
}

# Run the user-prompt-submit hook
exec "$BINARY" user-prompt-submit
