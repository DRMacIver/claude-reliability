#!/usr/bin/env python3
"""Setup for basic-stop-hook test.

Creates a git repository with an initial file, then modifies it
to create uncommitted changes.
"""

import subprocess
from pathlib import Path


def main():
    """Set up the test environment."""
    cwd = Path.cwd()

    # Create initial file and commit
    test_file = cwd / "test.txt"
    test_file.write_text("initial content\n")

    subprocess.run(["git", "add", "test.txt"], check=True)
    subprocess.run(
        ["git", "commit", "-m", "Initial commit", "--quiet"],
        check=True,
    )

    # Modify the file to create uncommitted changes
    test_file.write_text("modified content\n")


if __name__ == "__main__":
    main()
