#!/usr/bin/env python3
"""Set up a Python project with 5 modules for porting to Rust."""

import os
import subprocess
from pathlib import Path


def main():
    # Initialize git repo
    subprocess.run(["git", "init"], check=True, capture_output=True)
    subprocess.run(
        ["git", "config", "user.email", "test@example.com"],
        check=True,
        capture_output=True,
    )
    subprocess.run(
        ["git", "config", "user.name", "Test User"],
        check=True,
        capture_output=True,
    )

    # Create directory structure
    Path("src").mkdir(exist_ok=True)
    Path("tests").mkdir(exist_ok=True)

    # Create src/__init__.py
    Path("src/__init__.py").write_text("")

    # Create tests/__init__.py
    Path("tests/__init__.py").write_text("")

    # Module 1: string_utils
    Path("src/string_utils.py").write_text('''"""String utility functions."""


def reverse_string(s: str) -> str:
    """Reverse a string."""
    return s[::-1]
''')

    Path("tests/test_string_utils.py").write_text('''"""Tests for string_utils."""

from src.string_utils import reverse_string


def test_reverse_string():
    assert reverse_string("hello") == "olleh"
    assert reverse_string("") == ""
    assert reverse_string("a") == "a"
''')

    # Module 2: math_utils
    Path("src/math_utils.py").write_text('''"""Math utility functions."""


def factorial(n: int) -> int:
    """Calculate factorial of n."""
    if n < 0:
        raise ValueError("n must be non-negative")
    if n <= 1:
        return 1
    return n * factorial(n - 1)
''')

    Path("tests/test_math_utils.py").write_text('''"""Tests for math_utils."""

import pytest
from src.math_utils import factorial


def test_factorial():
    assert factorial(0) == 1
    assert factorial(1) == 1
    assert factorial(5) == 120


def test_factorial_negative():
    with pytest.raises(ValueError):
        factorial(-1)
''')

    # Module 3: list_utils
    Path("src/list_utils.py").write_text('''"""List utility functions."""


def flatten(nested: list) -> list:
    """Flatten a nested list one level deep."""
    result = []
    for item in nested:
        if isinstance(item, list):
            result.extend(item)
        else:
            result.append(item)
    return result
''')

    Path("tests/test_list_utils.py").write_text('''"""Tests for list_utils."""

from src.list_utils import flatten


def test_flatten():
    assert flatten([[1, 2], [3, 4]]) == [1, 2, 3, 4]
    assert flatten([1, [2, 3], 4]) == [1, 2, 3, 4]
    assert flatten([]) == []
    assert flatten([[1], [], [2]]) == [1, 2]
''')

    # Module 4: dict_utils
    Path("src/dict_utils.py").write_text('''"""Dictionary utility functions."""


def invert_dict(d: dict) -> dict:
    """Invert a dictionary (swap keys and values)."""
    return {v: k for k, v in d.items()}
''')

    Path("tests/test_dict_utils.py").write_text('''"""Tests for dict_utils."""

from src.dict_utils import invert_dict


def test_invert_dict():
    assert invert_dict({"a": 1, "b": 2}) == {1: "a", 2: "b"}
    assert invert_dict({}) == {}
    assert invert_dict({1: "x"}) == {"x": 1}
''')

    # Module 5: validation
    Path("src/validation.py").write_text('''"""Input validation functions."""


def is_valid_email(email: str) -> bool:
    """Check if a string looks like a valid email (simple check)."""
    if not email or "@" not in email:
        return False
    parts = email.split("@")
    if len(parts) != 2:
        return False
    local, domain = parts
    if not local or not domain:
        return False
    if "." not in domain:
        return False
    return True
''')

    Path("tests/test_validation.py").write_text('''"""Tests for validation."""

from src.validation import is_valid_email


def test_is_valid_email():
    assert is_valid_email("user@example.com") is True
    assert is_valid_email("user@domain.org") is True


def test_is_valid_email_invalid():
    assert is_valid_email("") is False
    assert is_valid_email("noatsign") is False
    assert is_valid_email("@nodomain.com") is False
    assert is_valid_email("user@") is False
    assert is_valid_email("user@nodot") is False
    assert is_valid_email("user@@double.com") is False
''')

    # Create pyproject.toml
    Path("pyproject.toml").write_text('''[project]
name = "python-utils"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = []

[dependency-groups]
dev = [
    "pytest>=9.0",
    "pytest-cov>=7.0",
]

[tool.pytest.ini_options]
testpaths = ["tests"]

[tool.coverage.run]
source = ["src"]

[tool.coverage.report]
fail_under = 100
''')

    # Create Justfile
    Path("Justfile").write_text('''# Run all checks
check:
    uv run pytest --cov=src --cov-report=term-missing --cov-fail-under=100

# Run tests only
test:
    uv run pytest

# Format code
format:
    uv run ruff format .

# Lint code
lint:
    uv run ruff check .
''')

    # Create .gitignore
    Path(".gitignore").write_text('''__pycache__/
*.pyc
.pytest_cache/
.coverage
htmlcov/
.venv/
target/
''')

    # Install Python dependencies
    subprocess.run(["uv", "sync"], check=True, capture_output=True)

    # Initial commit
    subprocess.run(["git", "add", "."], check=True, capture_output=True)
    subprocess.run(
        ["git", "commit", "-m", "Initial Python project with 5 modules"],
        check=True,
        capture_output=True,
    )

    print("Setup complete: Python project with 5 modules created")


if __name__ == "__main__":
    main()
