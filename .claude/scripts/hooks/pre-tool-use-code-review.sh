#!/usr/bin/env bash
# pre-tool-use-code-review.sh - PreToolUse hook for code review
#
# Ensures the binary is available and runs the code review hook.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
BINARY="$("$ENSURE_BINARY")"

# Run the code review hook, passing stdin through
exec "$BINARY" pre-tool-use code-review
