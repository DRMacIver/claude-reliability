#!/usr/bin/env bash
# pre-tool-use-no-verify.sh - PreToolUse hook to check for --no-verify
#
# Ensures the binary is available and runs the no-verify check.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
BINARY="$("$ENSURE_BINARY")"

# Run the no-verify hook, passing stdin through
exec "$BINARY" pre-tool-use no-verify
