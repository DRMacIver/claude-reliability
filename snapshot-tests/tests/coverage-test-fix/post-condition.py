#!/usr/bin/env python3
"""Post-condition assertions for coverage-test-fix test.

Verifies that:
1. The project config has require_push=false
2. The test file properly tests the function
3. Running `just check` passes
4. Replacing the test function with `assert False` causes failure
"""

import subprocess
import sys
from pathlib import Path


def main():
    """Run post-condition assertions."""
    # Check 1: require_push should be false (no origin)
    config_path = Path(".claude/project-config.yaml")
    if config_path.exists():
        config_content = config_path.read_text()
        if "require_push: true" in config_content:
            print("FAIL: require_push should be false (no origin)", file=sys.stderr)
            sys.exit(1)
        print("PASS: require_push is not true in config")
    else:
        # Config doesn't exist, which is fine - default should be false for no origin
        print("PASS: No config file (default require_push should be false)")

    # Check 2: just check passes
    result = subprocess.run(
        ["just", "check"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print("FAIL: just check does not pass", file=sys.stderr)
        print(f"stdout: {result.stdout}", file=sys.stderr)
        print(f"stderr: {result.stderr}", file=sys.stderr)
        sys.exit(1)
    print("PASS: just check passes")

    # Check 3: Test file is non-trivial (calls the function)
    test_file = Path("tests/test_math_utils.py")
    if not test_file.exists():
        print("FAIL: tests/test_math_utils.py does not exist", file=sys.stderr)
        sys.exit(1)

    test_content = test_file.read_text()
    if "add_numbers(" not in test_content or "assert add_numbers" not in test_content:
        print("FAIL: Test file doesn't appear to call add_numbers", file=sys.stderr)
        sys.exit(1)
    print("PASS: Test file calls add_numbers")

    # Check 4: Replacing test with assert False causes failure
    original_content = test_content

    # Find and replace the test function
    broken_test = '''"""Tests for math_utils."""

from src.math_utils import add_numbers


def test_add_numbers():
    """Test that add_numbers works."""
    assert False
'''
    test_file.write_text(broken_test)

    result = subprocess.run(
        ["just", "check"],
        capture_output=True,
        text=True,
    )

    # Restore original
    test_file.write_text(original_content)

    if result.returncode == 0:
        print("FAIL: Test should fail when replaced with assert False", file=sys.stderr)
        sys.exit(1)
    print("PASS: Test fails when replaced with assert False")

    print("\nAll post-conditions passed!")


if __name__ == "__main__":
    main()
