#!/usr/bin/env bash
# pre-tool-use-problem-mode.sh - PreToolUse hook to block tools in problem mode
#
# Blocks tool use when problem mode is active.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
BINARY="$("$ENSURE_BINARY")"

# Run the problem-mode hook, passing stdin through
exec "$BINARY" pre-tool-use problem-mode
