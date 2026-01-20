#!/usr/bin/env bash
# user-prompt-submit.sh - UserPromptSubmit hook wrapper for claude-reliability
#
# Ensures the binary is available and runs the user-prompt-submit hook.
# This hook resets session state when the user sends a new message.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-binary.sh"

# Get the binary path
BINARY="$("$ENSURE_BINARY")"

# Run the user-prompt-submit hook
exec "$BINARY" user-prompt-submit
