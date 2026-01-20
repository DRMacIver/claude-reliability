#!/usr/bin/env python3
"""Snapshot test runner for claude-reliability hooks.

Runs snapshot tests that verify hook behavior by replaying recorded
Claude Code sessions and comparing tool outputs.

Usage:
    uv run run-snapshots.py                    # Run all tests in replay mode
    uv run run-snapshots.py --mode=replay      # Explicit replay mode
    uv run run-snapshots.py --mode=record      # Record new transcripts
    uv run run-snapshots.py basic-stop-hook    # Run specific test
    uv run run-snapshots.py --verbose          # Show detailed output

Each test is a directory under tests/ containing:
    - setup.py: Creates test environment (run in temp git repo)
    - transcript.jsonl: Claude Code native format transcript
    - transcript.md: Human-readable (generated from jsonl)
    - post-condition.py: Optional assertions after replay
    - config.yaml: Optional test configuration
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from snapshot_tests.placeholder import PlaceholderRegistry
from snapshot_tests.transcript import parse_transcript, extract_tool_calls
from snapshot_tests.simulator import ToolSimulator
from snapshot_tests.compile_transcript import compile_transcript


@dataclass
class TestConfig:
    """Configuration for a single test."""

    name: str
    test_dir: Path
    setup_script: Path
    transcript_path: Path | None
    post_condition: Path | None
    config: dict[str, Any] = field(default_factory=dict)


@dataclass
class TestResult:
    """Result from running a single test."""

    name: str
    passed: bool
    error: str | None = None
    output: str = ""


def find_tests(tests_dir: Path, selected: list[str] | None = None) -> list[TestConfig]:
    """Find all test directories.

    Args:
        tests_dir: Directory containing test subdirectories
        selected: Optional list of test names to run

    Returns:
        List of TestConfig for each test
    """
    tests = []

    for test_dir in sorted(tests_dir.iterdir()):
        if not test_dir.is_dir():
            continue

        name = test_dir.name
        if selected and name not in selected:
            continue

        # Check for required setup script
        setup_script = test_dir / "setup.py"
        if not setup_script.exists():
            # Try shell script fallback
            setup_script = test_dir / "setup.sh"
            if not setup_script.exists():
                continue

        # Check for transcript
        transcript_path = test_dir / "transcript.jsonl"
        if not transcript_path.exists():
            transcript_path = None

        # Check for post-condition
        post_condition = test_dir / "post-condition.py"
        if not post_condition.exists():
            post_condition = None

        # Load config if present
        config = {}
        config_path = test_dir / "config.yaml"
        if config_path.exists():
            import yaml

            config = yaml.safe_load(config_path.read_text()) or {}

        tests.append(
            TestConfig(
                name=name,
                test_dir=test_dir,
                setup_script=setup_script,
                transcript_path=transcript_path,
                post_condition=post_condition,
                config=config,
            )
        )

    return tests


def setup_test_environment(temp_dir: Path, test: TestConfig, project_dir: Path) -> None:
    """Set up the test environment in a temp directory.

    Args:
        temp_dir: Temporary directory for test
        test: Test configuration
        project_dir: Project root directory
    """
    os.chdir(temp_dir)

    # Initialize git repo
    subprocess.run(["git", "init", "--quiet"], check=True)
    subprocess.run(
        ["git", "config", "user.email", "test@example.com"], check=True
    )
    subprocess.run(["git", "config", "user.name", "Test User"], check=True)

    # Copy project binary if it exists
    binary_path = project_dir / "target" / "release" / "claude-reliability"
    if binary_path.exists():
        (temp_dir / ".claude" / "bin").mkdir(parents=True, exist_ok=True)
        shutil.copy(binary_path, temp_dir / ".claude" / "bin" / "claude-reliability")

    # Run setup script
    if test.setup_script.suffix == ".py":
        subprocess.run(
            ["python3", str(test.setup_script)],
            check=True,
            cwd=temp_dir,
        )
    else:
        subprocess.run(
            ["bash", str(test.setup_script)],
            check=True,
            cwd=temp_dir,
        )

    # Create initial commit if needed
    result = subprocess.run(
        ["git", "status", "--porcelain"],
        capture_output=True,
        text=True,
    )
    if result.stdout.strip():
        subprocess.run(["git", "add", "-A"], check=True)
        subprocess.run(
            ["git", "commit", "-m", "Initial setup", "--quiet", "--allow-empty"],
            check=True,
        )


def run_replay(
    temp_dir: Path,
    test: TestConfig,
    project_dir: Path,
    verbose: bool = False,
) -> TestResult:
    """Run a test in replay mode.

    Simulates tool calls from the transcript and verifies outputs match.

    Args:
        temp_dir: Temporary directory for test
        test: Test configuration
        project_dir: Project root directory
        verbose: Show detailed output

    Returns:
        TestResult with pass/fail status
    """
    if not test.transcript_path:
        return TestResult(
            name=test.name,
            passed=False,
            error="No transcript.jsonl found",
        )

    # Parse transcript
    transcript = parse_transcript(test.transcript_path)
    tool_calls = extract_tool_calls(transcript)

    if verbose:
        print(f"  Found {len(tool_calls)} tool calls in transcript")

    # Set up simulator
    registry = PlaceholderRegistry()
    simulator = ToolSimulator(registry=registry, cwd=temp_dir)

    # Simulate each tool call
    errors = []
    for i, (tool_use, expected_result) in enumerate(tool_calls):
        if verbose:
            print(f"  [{i+1}/{len(tool_calls)}] {tool_use.name}: {_summarize_input(tool_use)}")

        expected_output = expected_result.content if expected_result else None

        result = simulator.simulate(
            tool_name=tool_use.name,
            tool_input=tool_use.input,
            expected_result=expected_output,
        )

        if not result.success:
            errors.append(f"Tool {tool_use.name} failed: {result.error}")
            if verbose:
                print(f"    ERROR: {result.error}")
        elif not result.matched_expected and expected_output is not None:
            errors.append(
                f"Tool {tool_use.name} output mismatch:\n"
                f"  Expected: {expected_output[:200]}\n"
                f"  Actual: {result.output[:200]}"
            )
            if verbose:
                print(f"    MISMATCH")

    if errors:
        return TestResult(
            name=test.name,
            passed=False,
            error="\n".join(errors),
        )

    # Run post-condition if present
    if test.post_condition:
        if verbose:
            print("  Running post-condition...")
        try:
            subprocess.run(
                ["python3", str(test.post_condition)],
                check=True,
                cwd=temp_dir,
                capture_output=True,
                text=True,
            )
        except subprocess.CalledProcessError as e:
            return TestResult(
                name=test.name,
                passed=False,
                error=f"Post-condition failed: {e.stderr}",
            )

    return TestResult(name=test.name, passed=True)


def _summarize_input(tool_use) -> str:
    """Create a short summary of tool input for display."""
    if tool_use.name == "Bash":
        cmd = tool_use.input.get("command", "")
        return cmd[:50] + "..." if len(cmd) > 50 else cmd
    elif tool_use.name in ("Write", "Edit", "Read"):
        return tool_use.input.get("file_path", "")[:50]
    elif tool_use.name == "Glob":
        return tool_use.input.get("pattern", "")
    elif tool_use.name == "Grep":
        return tool_use.input.get("pattern", "")[:30]
    else:
        return json.dumps(tool_use.input)[:50]


def run_test(
    test: TestConfig,
    mode: str,
    project_dir: Path,
    verbose: bool = False,
) -> TestResult:
    """Run a single test.

    Args:
        test: Test configuration
        mode: 'replay' or 'record'
        project_dir: Project root directory
        verbose: Show detailed output

    Returns:
        TestResult with pass/fail status
    """
    with tempfile.TemporaryDirectory() as temp_dir_str:
        temp_dir = Path(temp_dir_str)

        try:
            setup_test_environment(temp_dir, test, project_dir)

            if mode == "replay":
                return run_replay(temp_dir, test, project_dir, verbose)
            elif mode == "record":
                return TestResult(
                    name=test.name,
                    passed=False,
                    error="Record mode not yet implemented",
                )
            else:
                return TestResult(
                    name=test.name,
                    passed=False,
                    error=f"Unknown mode: {mode}",
                )

        except Exception as e:
            return TestResult(
                name=test.name,
                passed=False,
                error=str(e),
            )


def compile_all_transcripts(tests: list[TestConfig], verbose: bool = False) -> None:
    """Compile all transcript.jsonl files to transcript.md.

    Args:
        tests: List of test configurations
        verbose: Show detailed output
    """
    compiled = 0
    for test in tests:
        if not test.transcript_path or not test.transcript_path.exists():
            continue

        md_path = test.transcript_path.with_suffix(".md")
        transcript = parse_transcript(test.transcript_path)
        markdown = compile_transcript(transcript, verbose=False)
        md_path.write_text(markdown)
        compiled += 1

        if verbose:
            print(f"  Compiled {test.name}/transcript.md")

    print(f"Compiled {compiled} transcripts to markdown")


def main():
    """Main entry point."""
    parser = argparse.ArgumentParser(description="Run snapshot tests")
    parser.add_argument(
        "tests",
        nargs="*",
        help="Specific tests to run (default: all)",
    )
    parser.add_argument(
        "--mode",
        choices=["replay", "record"],
        default="replay",
        help="Test mode (default: replay)",
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Show detailed output",
    )
    parser.add_argument(
        "--tests-dir",
        type=Path,
        default=None,
        help="Directory containing tests (default: tests/)",
    )

    args = parser.parse_args()

    # Determine directories
    package_dir = Path(__file__).parent
    snapshot_tests_dir = package_dir.parent
    project_dir = snapshot_tests_dir.parent
    tests_dir = args.tests_dir or snapshot_tests_dir / "tests"

    if not tests_dir.exists():
        print(f"Error: Tests directory not found: {tests_dir}", file=sys.stderr)
        sys.exit(1)

    # Find tests
    selected = args.tests if args.tests else None
    tests = find_tests(tests_dir, selected)

    if not tests:
        print("No tests found")
        sys.exit(0)

    print(f"Running {len(tests)} snapshot tests in {args.mode} mode...")
    print()

    # Run tests
    results: list[TestResult] = []
    for test in tests:
        print(f"Test: {test.name}")

        result = run_test(test, args.mode, project_dir, args.verbose)
        results.append(result)

        if result.passed:
            print("  PASS")
        else:
            print(f"  FAIL: {result.error}")

        print()

    # Summary
    passed = sum(1 for r in results if r.passed)
    failed = len(results) - passed

    print(f"Results: {passed} passed, {failed} failed")
    print()

    # Compile transcripts to markdown
    compile_all_transcripts(tests, args.verbose)

    if failed > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
