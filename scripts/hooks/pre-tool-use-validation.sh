#!/usr/bin/env bash
# pre-tool-use-validation.sh - PreToolUse hook to track when validation is needed
#
# Ensures the binary is available and runs the validation hook.

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
    echo "Validation hook is disabled." >&2
    exit 0
}

# Run the validation hook, passing stdin through
exec "$BINARY" pre-tool-use validation
