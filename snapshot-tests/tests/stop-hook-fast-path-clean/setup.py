#!/usr/bin/env python3
"""Set up test environment for fast-path clean exit."""
from pathlib import Path

# Create a simple file that's already committed
# The git repo is initialized by the test harness
Path("README.md").write_text("# Test Project\n")

print("Setup complete")
