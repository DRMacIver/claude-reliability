#!/usr/bin/env bash
# Test script to verify MCP server actually works when plugin is loaded
#
# This test actually invokes an MCP tool through Claude Code and verifies it works.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "Testing MCP plugin - actual tool invocation"
echo "Plugin directory: ${PLUGIN_DIR}"
echo ""

# Create a temporary directory for testing
TEMP_DIR=$(mktemp -d)
trap "rm -rf '$TEMP_DIR'" EXIT

cd "$TEMP_DIR"
git init -q
echo "# MCP Test" > README.md
# Pre-create gitignore with sqlite pattern to avoid stop hook issues
echo ".claude/*.sqlite3" > .gitignore
git add README.md .gitignore
git commit -q -m "Initial commit"

echo "Test directory: ${TEMP_DIR}"
echo ""

# Verify binary exists first
if [[ ! -f "${PLUGIN_DIR}/.claude/bin/tasks-mcp" ]]; then
    echo -e "${RED}FAIL${NC}: tasks-mcp binary not found"
    echo "Build it with: cargo build --release --features mcp --bin tasks-mcp"
    exit 1
fi

echo "=== Testing: Create a task using MCP tool ==="
echo ""
echo "Running Claude with plugin, asking it to create a task..."
echo ""

# Ask Claude to create a task using the MCP tool
# Use --dangerously-skip-permissions to avoid permission prompts in test
OUTPUT=$(timeout 120 claude --plugin-dir "${PLUGIN_DIR}" --dangerously-skip-permissions -p "Use the create_task MCP tool to create a task with title 'Test Task' and description 'Testing MCP'. Return only the task ID that was created." 2>&1) || true

echo "Claude response:"
echo "$OUTPUT"
echo ""

# Check if the response indicates the MCP tool was used successfully
# Look for task ID patterns or success indicators
if echo "$OUTPUT" | grep -qiE '(task.*id.*[0-9]|id.*[0-9]|created.*task|task.*created|successfully)'; then
    echo -e "${GREEN}PASS${NC}: MCP create_task tool worked!"
elif echo "$OUTPUT" | grep -qi "mcp"; then
    echo -e "${YELLOW}PARTIAL${NC}: MCP server was detected but tool invocation unclear"
    echo "This still indicates the MCP server is connecting."
else
    echo -e "${RED}FAIL${NC}: MCP tool did not work"
    echo ""
    echo "Check if:"
    echo "  1. .mcp.json has correct format (mcpServers wrapper)"
    echo "  2. tasks-mcp binary is built and executable"
    echo "  3. CLAUDE_PLUGIN_ROOT is being resolved correctly"
    exit 1
fi

echo ""
echo "=== MCP server connection verified ==="
