#!/usr/bin/env python3
"""Post-condition assertions for add-multiply-function test.

Verifies that:
1. The new multiply_numbers function exists and is correct
2. The original add_numbers function is unchanged
3. The justfile is unchanged
4. Running `just check` passes
"""

import subprocess
import sys
from pathlib import Path


# Expected original content (must match setup.py)
ORIGINAL_ADD_NUMBERS = '''def add_numbers(a: int, b: int) -> int:
    """Add two numbers together."""
    return a + b'''

ORIGINAL_JUSTFILE = '''check:
\tpytest tests/ --cov=src --cov-report=term-missing --cov-fail-under=100
'''


def main():
    """Run post-condition assertions."""
    math_utils_file = Path("src/math_utils.py")
    test_file = Path("tests/test_math_utils.py")
    justfile = Path("justfile")

    # Check 1: multiply_numbers function exists and is correct
    if not math_utils_file.exists():
        print("FAIL: src/math_utils.py does not exist", file=sys.stderr)
        sys.exit(1)

    math_content = math_utils_file.read_text()
    if "def multiply_numbers" not in math_content:
        print("FAIL: multiply_numbers function not found", file=sys.stderr)
        sys.exit(1)
    print("PASS: multiply_numbers function exists")

    # Verify multiply_numbers works by importing and testing it
    sys.path.insert(0, str(Path.cwd()))
    try:
        from src.math_utils import multiply_numbers
        assert multiply_numbers(2, 3) == 6, "multiply_numbers(2, 3) should equal 6"
        assert multiply_numbers(0, 5) == 0, "multiply_numbers(0, 5) should equal 0"
        assert multiply_numbers(-2, 3) == -6, "multiply_numbers(-2, 3) should equal -6"
        print("PASS: multiply_numbers function works correctly")
    except ImportError as e:
        print(f"FAIL: Could not import multiply_numbers: {e}", file=sys.stderr)
        sys.exit(1)
    except AssertionError as e:
        print(f"FAIL: multiply_numbers incorrect: {e}", file=sys.stderr)
        sys.exit(1)

    # Check 2: Original add_numbers function is unchanged
    if ORIGINAL_ADD_NUMBERS not in math_content:
        print("FAIL: Original add_numbers function was modified", file=sys.stderr)
        print(f"Expected to find: {ORIGINAL_ADD_NUMBERS}", file=sys.stderr)
        sys.exit(1)
    print("PASS: Original add_numbers function is unchanged")

    # Check 3: Justfile is unchanged
    if not justfile.exists():
        print("FAIL: justfile does not exist", file=sys.stderr)
        sys.exit(1)

    justfile_content = justfile.read_text()
    if justfile_content != ORIGINAL_JUSTFILE:
        print("FAIL: justfile was modified", file=sys.stderr)
        print(f"Expected: {repr(ORIGINAL_JUSTFILE)}", file=sys.stderr)
        print(f"Actual: {repr(justfile_content)}", file=sys.stderr)
        sys.exit(1)
    print("PASS: justfile is unchanged")

    # Check 4: just check passes
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

    # Check 5: Test file tests multiply_numbers
    if not test_file.exists():
        print("FAIL: tests/test_math_utils.py does not exist", file=sys.stderr)
        sys.exit(1)

    test_content = test_file.read_text()
    if "multiply_numbers" not in test_content:
        print("FAIL: Test file doesn't test multiply_numbers", file=sys.stderr)
        sys.exit(1)
    print("PASS: Test file includes multiply_numbers tests")

    print("\nAll post-conditions passed!")


if __name__ == "__main__":
    main()
