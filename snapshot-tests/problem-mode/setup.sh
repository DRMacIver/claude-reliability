#!/usr/bin/env bash
# Setup for problem-mode test
#
# Creates a clean git repository for testing problem mode.

set -euo pipefail

# Create initial file and commit
echo "test file" > test.txt
git add test.txt
git commit -m "Initial commit" --quiet

# Create problem mode marker to simulate entering problem mode
mkdir -p .claude
echo "Problem mode active" > .claude/problem-mode.local
