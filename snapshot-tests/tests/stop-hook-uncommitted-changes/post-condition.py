#!/usr/bin/env python3
"""Verify uncommitted changes blocking postconditions."""
import subprocess
import sys
from pathlib import Path

# Check that the file was modified
main_py = Path("src/main.py")
if not main_py.exists():
    print("FAIL: src/main.py not found")
    sys.exit(1)

content = main_py.read_text()
if '"""' not in content and "'''" not in content:
    print("FAIL: No docstring found in src/main.py")
    sys.exit(1)

# Check that changes were committed
result = subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True)
if result.stdout.strip():
    print(f"FAIL: Uncommitted changes remain: {result.stdout}")
    sys.exit(1)

print("PASS: Changes committed, working directory clean")
