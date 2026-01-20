#!/usr/bin/env bash
# pre-tool-use-protect-config.sh - PreToolUse hook to protect reliability config
#
# Blocks Write, Edit, and delete operations on the reliability config file
# to prevent accidental modifications.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
BINARY="$("$ENSURE_BINARY")"

# Run the protect-config hook, passing stdin through
exec "$BINARY" pre-tool-use protect-config
