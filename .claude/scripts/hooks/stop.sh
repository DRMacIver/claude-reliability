#!/usr/bin/env bash
# stop.sh - Stop hook wrapper for claude-reliability
#
# Ensures the binary is available and runs the stop hook.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
BINARY="$("$ENSURE_BINARY")"

# Run the stop hook, passing stdin through
exec "$BINARY" stop
