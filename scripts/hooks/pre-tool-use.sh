#!/usr/bin/env bash
# pre-tool-use.sh - Unified PreToolUse hook
#
# Dispatches to the appropriate handler based on tool_name.
# Handles all PreToolUse events: no-verify, code-review, validation, jkw-setup, etc.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-binary.sh"

# Get the binary path (this may build it if source changed)
BINARY=$("$ENSURE_BINARY" 2>/dev/null) || {
    # Build failed - allow the tool use silently
    exit 0
}

# Run the unified pre-tool-use hook, passing stdin through
exec "$BINARY" pre-tool-use
