#!/usr/bin/env bash
# ensure-local-binary.sh - Ensures the claude-reliability binary is available locally
#
# This script:
# 1. Checks for binary at .claude/bin/claude-reliability
# 2. If missing, builds it with `just update-my-hooks`
# 3. Prints the path to the binary on success, exits non-zero on failure

set -euo pipefail

# Get the project root (where CLAUDE.md and justfile live)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BINARY_PATH="${PROJECT_ROOT}/.claude/bin/claude-reliability"

# Check if binary exists and is executable
if [[ -x "$BINARY_PATH" ]]; then
    # Verify it works
    if "$BINARY_PATH" version >/dev/null 2>&1; then
        echo "$BINARY_PATH"
        exit 0
    fi
    # Binary is broken, rebuild
    rm -f "$BINARY_PATH"
fi

# Binary missing or broken - build it
echo "Building claude-reliability binary..." >&2

cd "$PROJECT_ROOT"

# Check if just is available
if command -v just >/dev/null 2>&1; then
    just update-my-hooks >&2
elif command -v cargo >/dev/null 2>&1; then
    # Fall back to direct cargo build
    cargo build --release --features cli >&2
    mkdir -p .claude/bin
    cp target/release/claude-reliability .claude/bin/
    chmod +x .claude/bin/claude-reliability
else
    echo "ERROR: Neither 'just' nor 'cargo' available to build binary" >&2
    exit 1
fi

# Verify the binary now exists and works
if [[ -x "$BINARY_PATH" ]] && "$BINARY_PATH" version >/dev/null 2>&1; then
    echo "$BINARY_PATH"
    exit 0
fi

echo "ERROR: Failed to build claude-reliability binary" >&2
exit 1
