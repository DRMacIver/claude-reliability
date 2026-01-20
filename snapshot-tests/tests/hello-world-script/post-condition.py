#!/usr/bin/env python3
"""Post-condition assertions for hello-world-script test.

Verifies that:
1. hello.sh exists
2. hello.sh is executable
3. Running hello.sh prints "Hello world"
"""

import os
import subprocess
import sys
from pathlib import Path


def main():
    """Run post-condition assertions."""
    script_path = Path("hello.sh")

    # Check script exists
    if not script_path.exists():
        print(f"FAIL: {script_path} does not exist", file=sys.stderr)
        sys.exit(1)

    # Check script is executable
    if not os.access(script_path, os.X_OK):
        print(f"FAIL: {script_path} is not executable", file=sys.stderr)
        sys.exit(1)

    # Run script and check output
    result = subprocess.run(
        ["./hello.sh"],
        capture_output=True,
        text=True,
    )

    expected_output = "Hello world"
    actual_output = result.stdout.strip()

    if expected_output not in actual_output:
        print(f"FAIL: Expected output containing '{expected_output}'", file=sys.stderr)
        print(f"      Got: '{actual_output}'", file=sys.stderr)
        sys.exit(1)

    print("PASS: hello.sh exists, is executable, and prints 'Hello world'")


if __name__ == "__main__":
    main()
