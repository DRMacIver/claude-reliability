#!/usr/bin/env python3
"""Verify reflection first stop postconditions."""
import sys
from pathlib import Path

# Check that the file was modified
test_file = Path("test.txt")
if not test_file.exists():
    print("FAIL: test.txt not found")
    sys.exit(1)

content = test_file.read_text()
if "modified content" not in content:
    print(f"FAIL: File content not modified as expected: {content}")
    sys.exit(1)

# The reflection marker should be cleared after the second stop
# (If the test includes the agent stopping twice)
marker = Path(".claude/must-reflect.local")
if marker.exists():
    print("INFO: Reflection marker still exists (agent may not have stopped twice)")

print("PASS: File was modified correctly")
