#!/usr/bin/env bash
# Script to safely pull with rebase, handling the "local changes would be overwritten" error
#
# Usage: ./.githooks/pre-pull-rebase.sh

set -e

echo "=== Pre-pull checks ==="

# Check for uncommitted changes
if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "ERROR: Uncommitted changes detected. Please commit or stash first."
    git status --short
    exit 1
fi

# Check for untracked files that might interfere
untracked=$(git ls-files --others --exclude-standard)
if [ -n "$untracked" ]; then
    echo "WARNING: Untracked files present (shouldn't cause issues):"
    echo "$untracked" | head -5
fi

# Ensure index is in sync with HEAD
echo "Resetting index to HEAD..."
git reset HEAD --quiet

# Kill any background cargo processes that might modify files
if pgrep -x cargo >/dev/null 2>&1; then
    echo "WARNING: cargo processes running, they may interfere with rebase"
fi

# Try rebase, fall back to merge
echo "=== Attempting git pull --rebase ==="
if git pull --rebase; then
    echo "=== Rebase successful ==="
else
    echo "=== Rebase failed, trying merge instead ==="
    # Abort failed rebase if in progress
    if [ -d "$(git rev-parse --git-dir)/rebase-merge" ] || [ -d "$(git rev-parse --git-dir)/rebase-apply" ]; then
        git rebase --abort
    fi
    git pull --no-rebase
    echo "=== Merge completed ==="
fi
