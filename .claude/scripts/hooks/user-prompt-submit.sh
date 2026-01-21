#!/usr/bin/env bash
# user-prompt-submit.sh - UserPromptSubmit hook
#
# Clears validation markers when user sends a message.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
BINARY=$("$ENSURE_BINARY" 2>/dev/null) || {
    # Build failed - allow silently (this hook just clears markers)
    exit 0
}

# Run the user-prompt-submit hook
exec "$BINARY" user-prompt-submit
