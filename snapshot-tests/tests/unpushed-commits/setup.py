#!/usr/bin/env python3
"""Setup for unpushed-commits test.

Creates a git repository with a remote tracking branch,
then makes a local commit without pushing.
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

    # Set up a fake remote (using local directory as "remote")
    fake_remote = cwd.parent / "fake-remote"
    fake_remote.mkdir(exist_ok=True)

    # Remove existing repo if present
    repo_git = fake_remote / "repo.git"
    if repo_git.exists():
        import shutil
        shutil.rmtree(repo_git)

    subprocess.run(
        ["git", "clone", "--bare", ".", str(repo_git), "--quiet"],
        check=True,
    )
    subprocess.run(
        ["git", "remote", "add", "origin", str(fake_remote / "repo.git")],
        check=True,
    )

    # Set up tracking
    subprocess.run(
        ["git", "fetch", "origin", "--quiet"],
        check=True,
        capture_output=True,
    )

    # Try to set upstream (may fail depending on branch name)
    try:
        subprocess.run(
            ["git", "branch", "--set-upstream-to=origin/main", "main"],
            check=True,
            capture_output=True,
        )
    except subprocess.CalledProcessError:
        try:
            subprocess.run(
                ["git", "branch", "--set-upstream-to=origin/master", "master"],
                check=True,
                capture_output=True,
            )
        except subprocess.CalledProcessError:
            pass  # Ignore if setting upstream fails

    # Make a new commit (unpushed)
    test_file.write_text("new content\n")
    subprocess.run(["git", "add", "test.txt"], check=True)
    subprocess.run(
        ["git", "commit", "-m", "New commit (unpushed)", "--quiet"],
        check=True,
    )


if __name__ == "__main__":
    main()
