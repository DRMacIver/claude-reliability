#!/usr/bin/env bash
# List all MCP tools with their schemas

set -euo pipefail

cd "$(dirname "$0")/.."

{
    # Initialize request
    echo '{"jsonrpc":"2.0","method":"initialize","id":0,"params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}'
    sleep 0.1

    # Initialized notification
    echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'
    sleep 0.1

    # Tools list request
    echo '{"jsonrpc":"2.0","method":"tools/list","id":1}'
    sleep 0.3
} | cargo run --bin tasks-mcp --features mcp 2>/dev/null | tail -1 | jq .
