#!/usr/bin/env python3
"""Set up test environment for reflection second stop (allow exit)."""
from pathlib import Path

# Create a simple file
Path("test.txt").write_text("content\n")

# Pre-create the reflection marker to simulate first stop already happened
claude_dir = Path(".claude")
claude_dir.mkdir(exist_ok=True)
(claude_dir / "must-reflect.local").write_text("Reflection prompted - waiting for next stop")

print("Setup complete")
