#!/usr/bin/env bash
# hook.sh - Unified hook wrapper for claude-reliability
#
# Usage: hook.sh <hook-name>
#
# Ensures the binary is available and runs the specified hook.
# For the stop hook, shows build errors; for others, fails silently.

set -uo pipefail

HOOK_NAME="${1:-}"

if [[ -z "$HOOK_NAME" ]]; then
    echo "Usage: hook.sh <hook-name>" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-binary.sh"

# For the stop hook, show build errors; for others, fail silently
if [[ "$HOOK_NAME" == "stop" ]]; then
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
else
    # For non-stop hooks, fail silently
    BINARY=$("$ENSURE_BINARY" 2>/dev/null) || exit 0
fi

# Run the hook, passing stdin through
exec "$BINARY" "$HOOK_NAME"
