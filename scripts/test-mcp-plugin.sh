#!/usr/bin/env bash
# Test script to verify MCP server works when plugin is loaded
#
# Usage: ./scripts/test-mcp-plugin.sh
#
# This script verifies that:
# 1. The tasks-mcp binary exists
# 2. The binary responds to MCP protocol messages
# 3. Claude Code loads the plugin's MCP server correctly

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

pass() { echo -e "${GREEN}PASS${NC}: $1"; }
fail() { echo -e "${RED}FAIL${NC}: $1"; exit 1; }

echo "Testing MCP plugin configuration"
echo "Plugin directory: ${PLUGIN_DIR}"
echo ""

# Test 1: Binary exists
echo "=== Test 1: tasks-mcp binary exists ==="
if [[ -f "${PLUGIN_DIR}/.claude/bin/tasks-mcp" ]]; then
    pass "Binary found at .claude/bin/tasks-mcp"
else
    fail "Binary not found. Run: cargo build --release --features mcp --bin tasks-mcp && cp target/release/tasks-mcp .claude/bin/"
fi

# Test 2: Binary responds to MCP initialize
echo ""
echo "=== Test 2: tasks-mcp responds to MCP protocol ==="
INIT_REQUEST='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}'
RESPONSE=$(echo "$INIT_REQUEST" | timeout 5 "${PLUGIN_DIR}/.claude/bin/tasks-mcp" 2>/dev/null || true)

if echo "$RESPONSE" | grep -q '"protocolVersion":"2024-11-05"'; then
    pass "Binary responds correctly to MCP initialize"
else
    fail "Binary did not respond correctly. Response: $RESPONSE"
fi

# Test 3: .mcp.json has correct format
echo ""
echo "=== Test 3: .mcp.json format ==="
if [[ -f "${PLUGIN_DIR}/.mcp.json" ]]; then
    # Check for server definition without mcpServers wrapper (plugin root format)
    if grep -q '"tasks"' "${PLUGIN_DIR}/.mcp.json" && grep -q 'CLAUDE_PLUGIN_ROOT' "${PLUGIN_DIR}/.mcp.json"; then
        pass ".mcp.json has correct format"
        echo "Contents:"
        cat "${PLUGIN_DIR}/.mcp.json"
    else
        fail ".mcp.json missing required fields"
    fi
else
    fail ".mcp.json not found at plugin root"
fi

# Test 4: Plugin loads MCP server in Claude Code
echo ""
echo "=== Test 4: Claude Code loads plugin MCP server ==="

# Create temp directory for test
TEMP_DIR=$(mktemp -d)
trap "rm -rf '$TEMP_DIR'" EXIT

cd "$TEMP_DIR"
git init -q
echo "# Test" > README.md
git add README.md
git commit -q -m "Initial commit"

# Run claude with plugin and check for MCP server
OUTPUT=$(timeout 30 claude --plugin-dir "${PLUGIN_DIR}" -p "List the names of any MCP servers you have access to. Just respond with the server names, nothing else." 2>&1 || true)

if echo "$OUTPUT" | grep -qi "tasks"; then
    pass "Claude Code loaded plugin MCP server"
    echo "Response: $OUTPUT"
else
    echo "Response: $OUTPUT"
    fail "Claude Code did not find the tasks MCP server"
fi

echo ""
echo "=== All tests passed ==="
