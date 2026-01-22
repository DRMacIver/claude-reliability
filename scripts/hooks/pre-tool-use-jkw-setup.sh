#!/usr/bin/env bash
# pre-tool-use-jkw-setup.sh - PreToolUse hook to enforce JKW session setup
#
# Ensures the binary is available and runs the JKW setup hook.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-binary.sh"

# Get the binary path (this may build it if source changed)
BINARY_STDERR=$(mktemp)
trap "rm -f '$BINARY_STDERR'" EXIT

BINARY=$("$ENSURE_BINARY" 2>"$BINARY_STDERR") || {
    # Build/download failed - allow the operation but warn
    echo "# Plugin Build Failed" >&2
    echo "The claude-reliability plugin could not be built." >&2
    echo "JKW setup hook is disabled." >&2
    exit 0
}

# Run the JKW setup hook, passing stdin through
exec "$BINARY" pre-tool-use jkw-setup
