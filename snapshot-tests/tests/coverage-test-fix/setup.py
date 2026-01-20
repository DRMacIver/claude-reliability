#!/usr/bin/env python3
"""Setup for add-multiply-function test (in coverage-test-fix directory).

Creates a git repository with no origin that has:
1. A Python source file with an add_numbers function
2. A working test file for the add_numbers function
3. A justfile with `just check` that runs tests requiring 100% coverage

The claude-reliability plugin is installed by the test runner.
"""

import subprocess
from pathlib import Path


# Store original content for post-condition verification
ORIGINAL_MATH_UTILS = '''"""Math utility functions."""


def add_numbers(a: int, b: int) -> int:
    """Add two numbers together."""
    return a + b
'''

ORIGINAL_JUSTFILE = '''check:
\tpytest tests/ --cov=src --cov-report=term-missing --cov-fail-under=100
'''


def main():
    """Set up the test environment."""
    cwd = Path.cwd()

    # Install pytest and pytest-cov in the virtualenv
    subprocess.run(["pip", "install", "pytest", "pytest-cov"], check=True, capture_output=True)

    # Create source file with a function
    src_dir = cwd / "src"
    src_dir.mkdir(exist_ok=True)
    (src_dir / "__init__.py").write_text("")
    (src_dir / "math_utils.py").write_text(ORIGINAL_MATH_UTILS)

    # Create test file with a working test
    tests_dir = cwd / "tests"
    tests_dir.mkdir(exist_ok=True)
    (tests_dir / "__init__.py").write_text("")

    (tests_dir / "test_math_utils.py").write_text('''"""Tests for math_utils."""

from src.math_utils import add_numbers


def test_add_numbers():
    """Test that add_numbers works."""
    assert add_numbers(2, 3) == 5
    assert add_numbers(-1, 1) == 0
    assert add_numbers(0, 0) == 0
''')

    # Create justfile with check target requiring 100% coverage
    (cwd / "justfile").write_text(ORIGINAL_JUSTFILE)

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
