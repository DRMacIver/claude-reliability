#!/usr/bin/env python3
"""Verify reflection second stop postconditions."""
import sys
from pathlib import Path

# The reflection marker should be cleared after the stop
marker = Path(".claude/must-reflect.local")
if marker.exists():
    print("FAIL: Reflection marker should have been cleared")
    sys.exit(1)

print("PASS: Reflection marker cleared, exit allowed")
