#!/usr/bin/env bash
# Check that commits include beads interaction (Claude Code sessions only)
#
# This hook enforces that every commit interacts with beads in some way.
# When problems are identified during a Claude Code session, they should
# be tracked immediately - not deferred until later when context may be lost.
#
# The check only runs in Claude Code sessions (CLAUDECODE=1 environment variable).
# Manual commits outside Claude Code are not subject to this restriction.

set -e

# Only enforce in Claude Code sessions
if [ -z "$CLAUDECODE" ]; then
    exit 0
fi

# Check if any .beads/ files are staged
beads_changes=$(git diff --cached --name-only -- .beads/ 2>/dev/null || true)

if [ -z "$beads_changes" ]; then
    echo "ERROR: No beads changes staged for this commit." >&2
    echo "" >&2
    echo "In Claude Code sessions, every commit should interact with beads:" >&2
    echo "  - Create an issue:  bd create \"Issue title\"" >&2
    echo "  - Update an issue:  bd update <id> --status in_progress" >&2
    echo "  - Close an issue:   bd close <id>" >&2
    echo "" >&2
    echo "Then add beads changes: git add .beads/" >&2
    exit 1
fi
