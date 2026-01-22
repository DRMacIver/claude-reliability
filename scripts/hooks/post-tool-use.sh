#!/usr/bin/env bash
# post-tool-use.sh - PostToolUse hook
#
# Handles PostToolUse events, currently:
# - ExitPlanMode: Creates tasks to track plan implementation

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-binary.sh"

# Get the binary path (this may build it if source changed)
BINARY=$("$ENSURE_BINARY" 2>/dev/null) || {
    # Build failed - allow silently
    exit 0
}

# Run the post-tool-use hook, passing stdin through
exec "$BINARY" post-tool-use
