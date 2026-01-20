#!/usr/bin/env bash
# Setup for unpushed-commits test
#
# Creates a git repository with a remote tracking branch,
# then makes a local commit without pushing.

set -euo pipefail

# Create initial file and commit
echo "initial content" > test.txt
git add test.txt
git commit -m "Initial commit" --quiet

# Set up a fake remote (using local directory as "remote")
mkdir -p ../fake-remote
git clone --bare . ../fake-remote/repo.git --quiet
git remote add origin ../fake-remote/repo.git

# Set up tracking
git fetch origin --quiet 2>/dev/null || true
git branch --set-upstream-to=origin/main main 2>/dev/null || git branch --set-upstream-to=origin/master master 2>/dev/null || true

# Make a new commit (unpushed)
echo "new content" > test.txt
git add test.txt
git commit -m "New commit (unpushed)" --quiet
