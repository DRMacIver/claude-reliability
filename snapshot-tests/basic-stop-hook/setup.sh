#!/usr/bin/env bash
# Setup for basic-stop-hook test
#
# Creates a git repository with an initial file, then modifies it
# to create uncommitted changes.

set -euo pipefail

# Create initial file and commit
echo "initial content" > test.txt
git add test.txt
git commit -m "Initial commit" --quiet

# Modify the file to create uncommitted changes
echo "modified content" > test.txt
