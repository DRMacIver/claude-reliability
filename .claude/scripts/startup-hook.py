#!/usr/bin/env python3
"""Hook that runs at Claude Code startup."""

import sys
from pathlib import Path


def main() -> int:
    # Check for one-time setup prompt (newly created project)
    setup_prompt = Path(".claude/setup.local.md")
    if setup_prompt.exists():
        print("=" * 60)
        print("NEW PROJECT: Run /project-setup to complete initial configuration")
        print("=" * 60)
        print()
        print(setup_prompt.read_text())
        print()
        print("=" * 60)
        print("After setup is complete, delete this file:")
        print("  rm -f .claude/setup.local.md")
        print("=" * 60)

    # Check for build failures from previous session
    # Only unlink if setup prompt doesn't exist (to avoid clearing during new project)
    failure_marker = Path(".build-failure")
    if failure_marker.exists() and not setup_prompt.exists():
        print("WARNING: Previous build failed. Run quality checks.")
        failure_marker.unlink()

    return 0


if __name__ == "__main__":
    sys.exit(main())
