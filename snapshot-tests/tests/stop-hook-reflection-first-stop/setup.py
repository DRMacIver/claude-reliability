#!/usr/bin/env python3
"""Set up test environment for reflection first stop blocking."""
from pathlib import Path

# Create a simple file to be modified
Path("test.txt").write_text("original content\n")

print("Setup complete")
