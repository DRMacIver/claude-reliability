#!/usr/bin/env python3
"""Release script for claude-reliability.

Updates version to calver (YY.M.D.N), where N is the release number for the day.
This script is called by the release workflow.

Usage:
    python scripts/release.py              # Update version only (no tag)
    python scripts/release.py --dry-run    # Preview changes
    python scripts/release.py --version-only  # Just print next version
"""

import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path


def get_current_cargo_version() -> str | None:
    """Get the current version from Cargo.toml."""
    cargo_path = Path(__file__).parent.parent / "Cargo.toml"
    if not cargo_path.exists():
        return None

    content = cargo_path.read_text()
    # Match version in [package] section
    pattern = r'^version = "([^"]+)"'
    match = re.search(pattern, content, flags=re.MULTILINE)
    return match.group(1) if match else None


def parse_version_release_number(version: str, base_version: str) -> int | None:
    """Parse the release number from a version string.

    Returns the release number if the version matches base_version.N format,
    or 0 if it matches base_version exactly (old versioning scheme).
    Returns None if the version doesn't match the base_version date.
    """
    # Check for new format: base_version.N
    match = re.match(rf"^{re.escape(base_version)}\.(\d+)$", version)
    if match:
        return int(match.group(1))

    # Check for old format: base_version (without .N suffix)
    if version == base_version:
        return 0

    return None


def run_command(cmd: list[str], check: bool = True) -> subprocess.CompletedProcess[str]:
    """Run a shell command and return the result."""
    print(f"Running: {' '.join(cmd)}")
    result = subprocess.run(cmd, check=False, capture_output=True, text=True)

    if result.returncode != 0:
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        if result.stdout:
            print(result.stdout)
        if check:
            sys.exit(result.returncode)

    return result


def get_calver() -> str:
    """Generate calver version string in YY.M.D.N format.

    N is the release number for the day (0 for first, 1 for second, etc.).
    """
    now = datetime.now()
    year = now.strftime("%y")
    month = str(now.month)  # No leading zero
    day = str(now.day)  # No leading zero
    base_version = f"{year}.{month}.{day}"

    release_numbers: list[int] = []

    # Check Cargo.toml for current version
    current_version = get_current_cargo_version()
    if current_version:
        cargo_release = parse_version_release_number(current_version, base_version)
        if cargo_release is not None:
            release_numbers.append(cargo_release)

    # Find existing tags for today (format v{base}.N)
    result = run_command(["git", "tag", "-l", f"v{base_version}.*"], check=False)
    if result.returncode == 0 and result.stdout.strip():
        for tag in result.stdout.strip().split("\n"):
            match = re.match(rf"^v{re.escape(base_version)}\.(\d+)$", tag)
            if match:
                release_numbers.append(int(match.group(1)))

    # Also check for old format tag (v{base} without .N)
    result = run_command(["git", "tag", "-l", f"v{base_version}"], check=False)
    if result.returncode == 0 and result.stdout.strip() == f"v{base_version}":
        release_numbers.append(0)

    if not release_numbers:
        # No releases today yet, start at 0
        return f"{base_version}.0"

    # Next release number is max + 1
    next_release = max(release_numbers) + 1
    return f"{base_version}.{next_release}"


def update_version(new_version: str) -> None:
    """Update version in Cargo.toml."""
    cargo_path = Path(__file__).parent.parent / "Cargo.toml"
    content = cargo_path.read_text()

    # Safety check: verify this is the right Cargo.toml
    name_pattern = r'^name = "([^"]+)"'
    name_match = re.search(name_pattern, content, flags=re.MULTILINE)
    if not name_match or name_match.group(1) != "claude-reliability":
        print(f"Error: Cargo.toml does not belong to claude-reliability", file=sys.stderr)
        sys.exit(1)

    # Update version in [package] section
    # Match version line that appears after [package]
    content = re.sub(
        r'^version = "[^"]+"',
        f'version = "{new_version}"',
        content,
        count=1,
        flags=re.MULTILINE,
    )
    cargo_path.write_text(content)


def main() -> None:
    """Main release script."""
    version_only = "--version-only" in sys.argv
    dry_run = "--dry-run" in sys.argv

    # Generate calver version
    new_version = get_calver()

    if version_only:
        print(new_version)
        return

    if dry_run:
        print(f"Would update to version: {new_version}")
        return

    # Update version files
    update_version(new_version)
    print(f"Updated to version {new_version}")


if __name__ == "__main__":
    main()
