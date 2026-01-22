#!/usr/bin/env bash
# pre-tool-use.sh - Unified PreToolUse hook wrapper for claude-reliability plugin
#
# Dispatches to the appropriate handler based on tool_name internally.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE_BINARY="${SCRIPT_DIR}/../ensure-local-binary.sh"

BINARY="$("$ENSURE_BINARY")"
exec "$BINARY" pre-tool-use
