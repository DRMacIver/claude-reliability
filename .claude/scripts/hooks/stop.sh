#!/usr/bin/env bash
# stop.sh - Stop hook wrapper for claude-reliability
#
# Ensures the binary is available and runs the stop hook.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
# Capture stderr separately so we can show it on failure
BINARY_STDERR=$(mktemp)
trap "rm -f '$BINARY_STDERR'" EXIT

BINARY=$("$ENSURE_BINARY" 2>"$BINARY_STDERR") || {
    # Build/download failed - show the error but don't block the stop
    echo "# Plugin Build Failed"
    echo ""
    echo "The claude-reliability plugin could not be built:"
    echo ""
    echo '```'
    cat "$BINARY_STDERR"
    echo '```'
    echo ""
    echo "The stop hook is disabled until this is fixed."
    echo "Stop is allowed to proceed."
    exit 0
}

# Run the stop hook, passing stdin through
exec "$BINARY" stop
