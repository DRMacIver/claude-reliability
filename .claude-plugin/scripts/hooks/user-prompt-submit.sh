#!/usr/bin/env bash
# user-prompt-submit.sh - UserPromptSubmit hook

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

BINARY="$("$ENSURE_BINARY")"
exec "$BINARY" user-prompt-submit
