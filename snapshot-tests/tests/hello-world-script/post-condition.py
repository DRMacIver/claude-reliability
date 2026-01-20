#!/usr/bin/env python3
"""Post-condition assertions for hello-world-script test.

Verifies that:
1. hello.sh exists
2. hello.sh is executable
3. Running hello.sh prints "Hello world"
"""

import os
import stat
import subprocess
import sys
from pathlib import Path


def main():
    """Run post-condition assertions."""
    script_path = Path("hello.sh")

    print("=" * 60)
    print("Hello World Script Post-Condition Check")
    print("=" * 60)

    # Check script exists
    print(f"\n1. Checking if {script_path} exists...")
    if not script_path.exists():
        print(f"   FAIL: {script_path} does not exist", file=sys.stderr)
        sys.exit(1)
    print(f"   PASS: {script_path} exists")

    # Show file info
    file_stat = script_path.stat()
    mode = stat.filemode(file_stat.st_mode)
    print(f"   File mode: {mode}")
    print(f"   File size: {file_stat.st_size} bytes")

    # Check script is executable
    print(f"\n2. Checking if {script_path} is executable...")
    if not os.access(script_path, os.X_OK):
        print(f"   FAIL: {script_path} is not executable", file=sys.stderr)
        sys.exit(1)
    print(f"   PASS: {script_path} is executable")

    # Show script contents
    print(f"\n3. Script contents:")
    content = script_path.read_text()
    for i, line in enumerate(content.split('\n'), 1):
        print(f"   {i:3}: {line}")

    # Run script and check output
    print(f"\n4. Running {script_path}...")
    result = subprocess.run(
        ["./hello.sh"],
        capture_output=True,
        text=True,
    )

    print(f"   Exit code: {result.returncode}")
    print(f"   stdout: {result.stdout.strip()!r}")
    if result.stderr:
        print(f"   stderr: {result.stderr.strip()!r}")

    expected_output = "Hello world"
    actual_output = result.stdout.strip()

    print(f"\n5. Checking output contains '{expected_output}'...")
    if expected_output not in actual_output:
        print(f"   FAIL: Expected output containing '{expected_output}'", file=sys.stderr)
        print(f"         Got: '{actual_output}'", file=sys.stderr)
        sys.exit(1)
    print(f"   PASS: Output contains '{expected_output}'")

    print("\n" + "=" * 60)
    print("All post-conditions PASSED")
    print("=" * 60)


if __name__ == "__main__":
    main()
