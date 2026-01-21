#!/usr/bin/env python3
"""
Find all tests and create beads issues to investigate global state dependencies.

Creates an issue for each test to investigate whether it depends on:
1. Current working directory
2. Environment variables
3. Global state
4. File system state outside temp directories
"""

import re
import subprocess
import sys
from pathlib import Path


def find_rust_test_files():
    """Find all Rust source files with tests."""
    src_dir = Path("src")
    return list(src_dir.rglob("*.rs"))


def extract_tests(file_path: Path) -> list[dict]:
    """Extract test functions and their attributes from a Rust file."""
    content = file_path.read_text()
    tests = []

    # Pattern to match test functions with their attributes
    # Look for #[test] followed by optional attributes and fn test_name
    pattern = r'((?:#\[[^\]]+\]\s*)*#\[test\]\s*(?:#\[[^\]]+\]\s*)*)(fn\s+(test_\w+))'

    for match in re.finditer(pattern, content):
        attrs = match.group(1)
        fn_line = match.group(2)
        test_name = match.group(3)

        # Find the test body
        start = match.end()
        brace_count = 0
        body_start = None
        body_end = None

        for i, char in enumerate(content[start:], start):
            if char == '{':
                if body_start is None:
                    body_start = i
                brace_count += 1
            elif char == '}':
                brace_count -= 1
                if brace_count == 0:
                    body_end = i + 1
                    break

        if body_start and body_end:
            body = content[body_start:body_end]
            tests.append({
                'file': str(file_path),
                'name': test_name,
                'attrs': attrs,
                'body': body,
                'has_serial': '#[serial_test::serial]' in attrs or '#[serial]' in attrs,
            })

    return tests


def get_hints(test: dict) -> list[str]:
    """Get hints about potential issues for a test."""
    hints = []
    body = test['body']

    if 'set_current_dir' in body:
        if test['has_serial']:
            hints.append("Already uses set_current_dir with #[serial_test::serial]")
        else:
            hints.append("Uses set_current_dir - NEEDS #[serial_test::serial]")

    if 'env::var(' in body or 'env::var_os(' in body:
        hints.append("Uses env::var - check if properly isolated")

    if '/workspaces/' in body or 'CLAUDE_PROJECT_DIR' in body:
        hints.append("References workspace-specific paths")

    if 'current_dir()' in body and 'set_current_dir' not in body:
        hints.append("Reads current_dir()")

    if 'TempDir' in body or 'tempfile' in body:
        hints.append("Uses TempDir (good!)")

    if test['has_serial']:
        hints.append("Already marked #[serial_test::serial]")

    return hints


def issue_exists(test_name: str) -> bool:
    """Check if an issue already exists for this test."""
    try:
        result = subprocess.run(
            ['bd', 'list', '--status=open', '--limit', '0'],
            capture_output=True,
            text=True,
            timeout=30
        )
        return f"Audit test isolation: {test_name}" in result.stdout
    except Exception:
        return False


def create_beads_issue(test: dict) -> bool:
    """Create a beads issue to investigate a test."""
    # Skip if issue already exists
    if issue_exists(test['name']):
        print(f"  Skipped (exists): {test['name']}")
        return False

    hints = get_hints(test)
    hints_text = chr(10).join(f'- {hint}' for hint in hints) if hints else "- No obvious hints"

    title = f"Audit test isolation: {test['name']}"
    description = f"""Investigate test `{test['name']}` in `{test['file']}` for global state dependencies.

## Hints from static analysis
{hints_text}

## Investigation Checklist
- [ ] Does this test depend on current working directory?
- [ ] Does this test depend on environment variables?
- [ ] Does this test use file paths outside of TempDir?
- [ ] Can this test fail if run in parallel with other tests?

## Action Required
If the test depends on global state:
1. **Preferred**: Fix the test to be isolated (use TempDir, mock env vars)
2. **If unfixable**: Add `#[serial_test::serial]` attribute

If the test is already isolated:
- Close this issue as "no action needed"

## Acceptance Criteria
- Test is verified to be isolated from global state, OR
- Test is marked with #[serial_test::serial] if isolation is not possible
"""

    try:
        result = subprocess.run(
            ['bd', 'create', '--title', title, '--type', 'task', '--priority', '4'],
            input=description,
            capture_output=True,
            text=True
        )
        if result.returncode == 0:
            issue_id = result.stdout.strip().split()[-1] if result.stdout.strip() else "unknown"
            print(f"  Created: {issue_id} - {test['name']}")
            return True
        else:
            print(f"  Failed: {test['name']} - {result.stderr.strip()}")
            return False
    except FileNotFoundError:
        print("bd command not found - beads not installed")
        return False


def main():
    """Main function."""
    print("Scanning for all tests...\n")

    all_tests = []

    for file_path in find_rust_test_files():
        tests = extract_tests(file_path)
        all_tests.extend(tests)

    # Sort by file then name
    all_tests.sort(key=lambda t: (t['file'], t['name']))

    print(f"Found {len(all_tests)} tests total\n")

    # Group by file for display
    by_file = {}
    for test in all_tests:
        by_file.setdefault(test['file'], []).append(test)

    for file_path, tests in sorted(by_file.items()):
        print(f"{file_path}: {len(tests)} tests")

    print()

    if '--create-issues' in sys.argv:
        print("Creating beads issues for all tests...\n")
        created = 0
        for test in all_tests:
            if create_beads_issue(test):
                created += 1
        print(f"\nCreated {created} issues for {len(all_tests)} tests")
    else:
        print("Run with --create-issues to create beads issues for all tests")

    return 0


if __name__ == '__main__':
    sys.exit(main())
