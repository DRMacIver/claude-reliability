#!/usr/bin/env bash
# Session start hook - runs at the beginning of each Claude Code session

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# Get the binary path
BINARY=$("$SCRIPT_DIR/../ensure-local-binary.sh" 2>/dev/null) || true

if [[ -n "$BINARY" && -x "$BINARY" ]]; then
    # Ensure config exists
    "$BINARY" ensure-config >/dev/null 2>&1 || true

    # Print intro message
    "$BINARY" intro 2>/dev/null || true
fi

# Check for one-time setup prompt (newly created project)
SETUP_PROMPT="$PROJECT_DIR/.claude/setup.local.md"
if [[ -f "$SETUP_PROMPT" ]]; then
    echo "============================================================"
    echo "NEW PROJECT: Run /project-setup to complete initial configuration"
    echo "============================================================"
    echo
    cat "$SETUP_PROMPT"
    echo
    echo "============================================================"
    echo "After setup is complete, delete this file:"
    echo "  rm -f .claude/setup.local.md"
    echo "============================================================"
fi

# Check for build failures from previous session
FAILURE_MARKER="$PROJECT_DIR/.build-failure"
if [[ -f "$FAILURE_MARKER" && ! -f "$SETUP_PROMPT" ]]; then
    echo "WARNING: Previous build failed. Run quality checks."
    rm -f "$FAILURE_MARKER"
fi

exit 0
