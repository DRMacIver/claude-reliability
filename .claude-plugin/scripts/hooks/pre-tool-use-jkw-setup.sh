#!/usr/bin/env bash
# pre-tool-use-jkw-setup.sh - PreToolUse hook to enforce JKW session setup

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

BINARY="$("$ENSURE_BINARY")"
exec "$BINARY" pre-tool-use jkw-setup
