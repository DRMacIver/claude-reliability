#!/usr/bin/env bash
# pre-tool-use-no-verify.sh - PreToolUse hook to check for --no-verify
#
# Ensures the binary is available and runs the no-verify check.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
# Discard stderr (build status messages) - we only need the binary path
BINARY=$("$ENSURE_BINARY" 2>/dev/null) || {
    # Build failed - allow the tool use silently
    exit 0
}

# Run the no-verify hook, passing stdin through
exec "$BINARY" pre-tool-use no-verify
