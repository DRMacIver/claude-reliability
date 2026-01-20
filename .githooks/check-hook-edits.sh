#!/usr/bin/env bash
# Protect .claude/scripts/ hook edits from accidental modification
#
# This hook requires explicit acknowledgment before allowing edits to hook files.
# This prevents Claude from accidentally truncating or overwriting important hooks
# during updates or other operations.
#
# To modify hooks, set the MODIFY_HOOK environment variable:
#   MODIFY_HOOK="Yes I really meant to edit the jkw-stop-hook.py hook(s) the user has said it is OK" git commit ...
#
# The value must include the names of the hooks being modified.

set -e

# Only enforce in Claude Code sessions
if [ -z "$CLAUDECODE" ]; then
    exit 0
fi

# Check if any .claude/scripts/ files are staged
hook_changes=$(git diff --cached --name-only -- .claude/scripts/ 2>/dev/null || true)

if [ -z "$hook_changes" ]; then
    exit 0
fi

# Extract the hook names that are being modified
modified_hooks=""
for file in $hook_changes; do
    hook_name=$(basename "$file")
    if [ -n "$modified_hooks" ]; then
        modified_hooks="$modified_hooks, $hook_name"
    else
        modified_hooks="$hook_name"
    fi
done

# Check if MODIFY_HOOK is set and contains the hook names
if [ -z "$MODIFY_HOOK" ]; then
    echo "ERROR: Attempting to modify hook files without explicit acknowledgment." >&2
    echo "" >&2
    echo "Modified hooks: $modified_hooks" >&2
    echo "" >&2
    echo "Hook files (.claude/scripts/) are protected because they contain critical" >&2
    echo "functionality that can be accidentally truncated or overwritten." >&2
    echo "" >&2
    echo "If you really meant to modify these hooks, set the MODIFY_HOOK environment variable:" >&2
    echo "" >&2
    echo "  MODIFY_HOOK=\"Yes I really meant to edit the $modified_hooks hook(s) the user has said it is OK\" git commit ..." >&2
    echo "" >&2
    exit 1
fi

# Verify MODIFY_HOOK contains all the hook names
missing_hooks=""
for file in $hook_changes; do
    hook_name=$(basename "$file")
    if ! echo "$MODIFY_HOOK" | grep -q "$hook_name"; then
        if [ -n "$missing_hooks" ]; then
            missing_hooks="$missing_hooks, $hook_name"
        else
            missing_hooks="$hook_name"
        fi
    fi
done

if [ -n "$missing_hooks" ]; then
    echo "ERROR: MODIFY_HOOK does not acknowledge all modified hooks." >&2
    echo "" >&2
    echo "Missing acknowledgment for: $missing_hooks" >&2
    echo "" >&2
    echo "Your MODIFY_HOOK value: $MODIFY_HOOK" >&2
    echo "" >&2
    echo "Please include all hook names in MODIFY_HOOK." >&2
    exit 1
fi

# All hooks are acknowledged
echo "Hook edit acknowledged: $modified_hooks"
