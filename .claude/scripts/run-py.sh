#!/usr/bin/env bash
# Run a Python script with the project's dev dependencies.
# Usage: .claude/scripts/run-py.sh <script.py> [args...]
#
# This script determines the project directory from its own location,
# ensuring it works correctly regardless of the current working directory
# or how environment variables are expanded.

set -e

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Project root is two levels up from .claude/scripts/
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Check if this is a Python project with pyproject.toml
if [ -f "$PROJECT_DIR/pyproject.toml" ]; then
    # Ensure dependencies are synced (quick check, uv handles caching)
    uv sync --project "$PROJECT_DIR" --group dev --quiet 2>/dev/null || true

    # Run the Python script with dev dependencies
    # Use --quiet to prevent progress output to stderr (which Claude Code interprets as error)
    exec uv run --quiet --project "$PROJECT_DIR" --group dev python3 "$@"
else
    # No pyproject.toml - use uv to run with required hook dependencies
    # Claude Code hooks require pyyaml even in non-Python projects
    exec uv run --quiet --with pyyaml python3 "$@"
fi
