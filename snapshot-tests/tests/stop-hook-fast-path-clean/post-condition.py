#!/usr/bin/env python3
"""Verify fast-path clean exit postconditions."""
import sys
from pathlib import Path

# Check no session markers were created
claude_dir = Path(".claude")
if claude_dir.exists():
    markers = [
        "jkw-session.local.json",
        "must-reflect.local",
        "problem-mode.local",
    ]
    for marker in markers:
        marker_path = claude_dir / marker
        if marker_path.exists():
            print(f"FAIL: Unexpected marker file: {marker}")
            sys.exit(1)

print("PASS: Fast path clean exit - no markers created")
