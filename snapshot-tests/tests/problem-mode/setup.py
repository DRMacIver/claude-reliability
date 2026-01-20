#!/usr/bin/env python3
"""Setup for problem-mode test.

Creates a clean git repository for testing problem mode.
"""

import subprocess
from pathlib import Path


def main():
    """Set up the test environment."""
    cwd = Path.cwd()

    # Create initial file and commit
    test_file = cwd / "test.txt"
    test_file.write_text("test file\n")

    subprocess.run(["git", "add", "test.txt"], check=True)
    subprocess.run(
        ["git", "commit", "-m", "Initial commit", "--quiet"],
        check=True,
    )

    # Create problem mode marker to simulate entering problem mode
    claude_dir = cwd / ".claude"
    claude_dir.mkdir(exist_ok=True)
    (claude_dir / "problem-mode.local").write_text("Problem mode active\n")


if __name__ == "__main__":
    main()
