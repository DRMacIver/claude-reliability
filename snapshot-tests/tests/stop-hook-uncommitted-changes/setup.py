#!/usr/bin/env python3
"""Set up test environment for uncommitted changes blocking."""
from pathlib import Path

# Create source file that will be modified
Path("src").mkdir(exist_ok=True)
Path("src/main.py").write_text('print("hello")\n')

print("Setup complete")
