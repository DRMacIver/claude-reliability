#!/usr/bin/env bash
# pre-tool-use-jkw-setup.sh - PreToolUse hook to enforce JKW session setup
#
# Blocks non-session-file writes when JKW setup is required.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
BINARY="$("$ENSURE_BINARY")"

# Run the jkw-setup hook, passing stdin through
exec "$BINARY" pre-tool-use jkw-setup
