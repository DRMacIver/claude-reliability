#!/usr/bin/env python3
"""Setup for coverage-test-fix test.

Creates a git repository with no origin that has:
1. A Python source file with a function
2. A test file that imports but doesn't properly test the function
3. A justfile with `just check` that runs tests requiring 100% coverage
"""

import subprocess
from pathlib import Path


def main():
    """Set up the test environment."""
    cwd = Path.cwd()

    # Create source file with a function
    src_dir = cwd / "src"
    src_dir.mkdir(exist_ok=True)
    (src_dir / "__init__.py").write_text("")

    (src_dir / "math_utils.py").write_text('''"""Math utility functions."""


def add_numbers(a: int, b: int) -> int:
    """Add two numbers together."""
    return a + b
''')

    # Create test file that doesn't properly call the function
    tests_dir = cwd / "tests"
    tests_dir.mkdir(exist_ok=True)
    (tests_dir / "__init__.py").write_text("")

    (tests_dir / "test_math_utils.py").write_text('''"""Tests for math_utils."""

from src.math_utils import add_numbers


def test_add_numbers():
    """Test that add_numbers works."""
    # BUG: This test doesn't actually call the function!
    assert True
''')

    # Create justfile with check target requiring 100% coverage
    (cwd / "justfile").write_text('''check:
\tpytest tests/ --cov=src --cov-report=term-missing --cov-fail-under=100
''')

    # Create pyproject.toml for the test project
    (cwd / "pyproject.toml").write_text('''[project]
name = "test-project"
version = "0.1.0"
requires-python = ">=3.11"

[tool.pytest.ini_options]
pythonpath = ["."]
''')

    # Initialize git repo (no origin)
    subprocess.run(["git", "init", "--quiet"], check=True)
    subprocess.run(["git", "config", "user.email", "test@example.com"], check=True)
    subprocess.run(["git", "config", "user.name", "Test User"], check=True)
    subprocess.run(["git", "add", "-A"], check=True)
    subprocess.run(["git", "commit", "-m", "Initial commit", "--quiet"], check=True)


if __name__ == "__main__":
    main()
