#!/usr/bin/env python3
"""Pre-tool hook to reject git commit --no-verify unless explicitly allowed.

This hook runs before Bash commands and checks if the command is a git commit
with --no-verify (or -n) flag. If so, it rejects the command unless the
NO_VERIFY_OK environment variable is set with an acknowledgment.

To allow --no-verify, set:
  NO_VERIFY_OK="I promise the user has said I can use --no-verify here"
"""

from __future__ import annotations

import json
import os
import re
import sys


def main() -> int:
    # Only run in Claude Code sessions
    if not os.environ.get("CLAUDECODE"):
        return 0

    # Read hook input from stdin
    try:
        stdin_content = sys.stdin.read()
        if not stdin_content.strip():
            return 0
        hook_input = json.loads(stdin_content)
    except (json.JSONDecodeError, Exception):
        return 0

    # Check if this is a Bash tool call
    tool_name = hook_input.get("tool_name", "")
    if tool_name != "Bash":
        return 0

    # Get the command being executed
    tool_input = hook_input.get("tool_input", {})
    command = tool_input.get("command", "")

    # Check for git commit with --no-verify or -n
    # Match patterns like:
    # - git commit --no-verify
    # - git commit -n
    # - git commit -am "msg" --no-verify
    # - git commit -n -m "msg"
    no_verify_patterns = [
        "\\bgit\\s+commit\\b.*--no-verify\\b",
        "\\bgit\\s+commit\\b.*\\s-[a-zA-Z]*n",  # -n anywhere in flags
    ]

    has_no_verify = any(re.search(pattern, command) for pattern in no_verify_patterns)

    if not has_no_verify:
        return 0

    # Check for the acknowledgment environment variable
    no_verify_ok = os.environ.get("NO_VERIFY_OK", "")

    if "I promise the user has said I can use --no-verify here" in no_verify_ok:
        print("--no-verify acknowledged by NO_VERIFY_OK environment variable")
        return 0

    # Reject the command
    err = sys.stderr
    print("ERROR: Attempting to use git commit with --no-verify.", file=err)
    print("", file=err)
    print("The --no-verify flag skips pre-commit hooks, which are", file=err)
    print("important for:", file=err)
    print("- Running quality checks before commits", file=err)
    print("- Preventing secrets from being committed", file=err)
    print("- Ensuring beads are properly synced", file=err)
    print("", file=err)
    print("If the user has explicitly said you can skip hooks, set:", file=err)
    print("", file=err)
    print('  NO_VERIFY_OK="I promise the user has said I can use', file=err)
    print('  --no-verify here"', file=err)
    print("", file=err)

    return 2  # Block the command


if __name__ == "__main__":
    sys.exit(main())
