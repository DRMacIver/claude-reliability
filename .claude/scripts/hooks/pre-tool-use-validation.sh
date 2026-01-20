#!/usr/bin/env bash
# pre-tool-use-validation.sh - PreToolUse hook to track when validation is needed
#
# Sets a marker when Edit/Write/NotebookEdit tools are used, indicating
# that validation must run before stopping.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

# Get the binary path (this may build it if missing)
BINARY="$("$ENSURE_BINARY")"

# Run the validation hook, passing stdin through
exec "$BINARY" pre-tool-use validation
