#!/usr/bin/env python3
"""Hook that runs at Claude Code startup."""

import subprocess
import sys
from pathlib import Path


def ensure_config() -> None:
    """Ensure the reliability config file exists."""
    # Find the binary using ensure-local-binary.sh
    script_dir = Path(__file__).parent
    ensure_binary = script_dir / "ensure-local-binary.sh"

    if not ensure_binary.exists():
        return

    try:
        result = subprocess.run(
            [str(ensure_binary)],
            capture_output=True,
            text=True,
            check=True,
        )
        binary_path = result.stdout.strip()
        if binary_path and Path(binary_path).exists():
            # Run ensure-config
            subprocess.run(
                [binary_path, "ensure-config"],
                capture_output=True,
                check=False,
            )
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass  # Silently ignore if binary is not available


def main() -> int:
    # Ensure reliability config exists
    ensure_config()

    # Check for one-time setup prompt (newly created project)
    setup_prompt = Path(".claude/setup.local.md")
    if setup_prompt.exists():
        print("=" * 60)
        print("NEW PROJECT: Run /project-setup to complete initial configuration")
        print("=" * 60)
        print()
        print(setup_prompt.read_text())
        print()
        print("=" * 60)
        print("After setup is complete, delete this file:")
        print("  rm -f .claude/setup.local.md")
        print("=" * 60)

    # Check for build failures from previous session
    # Only unlink if setup prompt doesn't exist (to avoid clearing during new project)
    failure_marker = Path(".build-failure")
    if failure_marker.exists() and not setup_prompt.exists():
        print("WARNING: Previous build failed. Run quality checks.")
        failure_marker.unlink()

    return 0


if __name__ == "__main__":
    sys.exit(main())
