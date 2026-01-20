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
import re
import shutil
import subprocess
import sys
import tempfile
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from snapshot_tests.placeholder import PlaceholderRegistry
from snapshot_tests.transcript import parse_transcript, extract_tool_calls, get_project_directory
from snapshot_tests.simulator import ToolSimulator
from snapshot_tests.compile_transcript import compile_transcript
from snapshot_tests.plugin_setup import install_plugin


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
    post_condition_output: str = ""


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


def create_virtualenv(temp_dir: Path) -> Path:
    """Create a virtualenv for the test using uv.

    The venv is created OUTSIDE the test directory (as a sibling) so that
    glob operations in the test directory won't find .venv files.

    Args:
        temp_dir: Temporary directory for test

    Returns:
        Path to the virtualenv directory
    """
    # Create venv as a sibling to the test directory, not inside it
    # This prevents globs from finding .venv files
    venv_dir = temp_dir.parent / f"{temp_dir.name}_venv"

    # Create virtualenv with uv, including pip
    subprocess.run(
        ["uv", "venv", str(venv_dir), "--seed"],
        check=True,
        cwd=temp_dir,
        capture_output=True,
    )

    return venv_dir


def get_venv_env(venv_dir: Path) -> dict[str, str]:
    """Get environment variables for running commands in the virtualenv.

    Args:
        venv_dir: Path to the virtualenv directory

    Returns:
        Environment dict with PATH and VIRTUAL_ENV set
    """
    env = os.environ.copy()
    venv_bin = venv_dir / "bin"
    env["PATH"] = f"{venv_bin}:{env.get('PATH', '')}"
    env["VIRTUAL_ENV"] = str(venv_dir)
    # Remove PYTHONHOME if set, as it can interfere with venv
    env.pop("PYTHONHOME", None)
    return env


def setup_test_environment(
    temp_dir: Path,
    test: TestConfig,
    project_dir: Path,
    venv_dir: Path,
) -> None:
    """Set up the test environment in a temp directory.

    Args:
        temp_dir: Temporary directory for test
        test: Test configuration
        project_dir: Project root directory
        venv_dir: Path to the virtualenv directory
    """
    os.chdir(temp_dir)

    # Install the claude-reliability plugin
    install_plugin(temp_dir)

    # Get virtualenv environment
    env = get_venv_env(venv_dir)

    # Run setup script - this handles git init if needed
    if test.setup_script.suffix == ".py":
        subprocess.run(
            ["python", str(test.setup_script)],
            check=True,
            cwd=temp_dir,
            env=env,
        )
    else:
        subprocess.run(
            ["bash", str(test.setup_script)],
            check=True,
            cwd=temp_dir,
            env=env,
        )


def run_replay(
    temp_dir: Path,
    test: TestConfig,
    project_dir: Path,
    venv_dir: Path,
    verbose: bool = False,
) -> TestResult:
    """Run a test in replay mode.

    Simulates tool calls from the transcript and verifies outputs match.

    Args:
        temp_dir: Temporary directory for test
        test: Test configuration
        project_dir: Project root directory
        venv_dir: Path to the virtualenv directory
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

    # Get the original project directory from the transcript
    original_project_dir = get_project_directory(transcript)
    path_mappings = {}
    if original_project_dir:
        # Map original path to current temp dir
        path_mappings[original_project_dir] = str(temp_dir)
        if verbose:
            print(f"  Path mapping: {original_project_dir} -> {temp_dir}")

    # Set up simulator with path mappings
    registry = PlaceholderRegistry()
    simulator = ToolSimulator(registry=registry, cwd=temp_dir, path_mappings=path_mappings)

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
            # Show the substituted expected for debugging
            substituted_expected = simulator.substitute_paths(expected_output)
            errors.append(
                f"Tool {tool_use.name} output mismatch:\n"
                f"  Expected: {substituted_expected[:200]}\n"
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

    # Get virtualenv environment
    env = get_venv_env(venv_dir)

    # Run post-condition if present
    post_condition_output = ""
    if test.post_condition:
        if verbose:
            print("  Running post-condition...")
        try:
            result = subprocess.run(
                ["python", str(test.post_condition)],
                check=True,
                cwd=temp_dir,
                capture_output=True,
                text=True,
                env=env,
            )
            post_condition_output = result.stdout
            if verbose and post_condition_output:
                print(f"  Post-condition output:\n{post_condition_output}")
        except subprocess.CalledProcessError as e:
            return TestResult(
                name=test.name,
                passed=False,
                error=f"Post-condition failed: {e.stderr}",
                post_condition_output=e.stdout,
            )

    return TestResult(name=test.name, passed=True, post_condition_output=post_condition_output)


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


def extract_prompt_from_story(story_path: Path) -> str | None:
    """Extract the prompt from story.md.

    Looks for a ```-fenced code block after a "## Prompt" heading.
    """
    if not story_path.exists():
        return None

    content = story_path.read_text()

    # Find the Prompt section
    prompt_match = re.search(r"##\s*Prompt\s*\n+```[^\n]*\n(.*?)```", content, re.DOTALL)
    if prompt_match:
        return prompt_match.group(1).strip()

    return None


def run_record(
    temp_dir: Path,
    test: TestConfig,
    project_dir: Path,
    venv_dir: Path,
    verbose: bool = False,
) -> TestResult:
    """Run a test in record mode.

    Runs Claude Code with the prompt from story.md and captures the transcript.

    Args:
        temp_dir: Temporary directory for test
        test: Test configuration
        project_dir: Project root directory
        verbose: Show detailed output

    Returns:
        TestResult with pass/fail status
    """
    # Extract prompt from story.md
    story_path = test.test_dir / "story.md"
    prompt = extract_prompt_from_story(story_path)

    if not prompt:
        return TestResult(
            name=test.name,
            passed=False,
            error="No prompt found in story.md (need ## Prompt section with ```code block```)",
        )

    if verbose:
        print(f"  Prompt: {prompt[:100]}...")

    # Generate session ID
    session_id = str(uuid.uuid4())

    if verbose:
        print(f"  Session ID: {session_id}")

    # Run Claude Code
    cmd = [
        "claude",
        "--print",
        "--session-id", session_id,
        "--dangerously-skip-permissions",
        "--model", "opus",  # Use opus for first message
        "-p", prompt,
    ]

    if verbose:
        print(f"  Running Claude Code...")

    try:
        result = subprocess.run(
            cmd,
            cwd=temp_dir,
            capture_output=True,
            text=True,
            timeout=300,  # 5 minute timeout
        )
    except subprocess.TimeoutExpired:
        return TestResult(
            name=test.name,
            passed=False,
            error="Claude Code timed out after 5 minutes",
        )

    if verbose:
        print(f"  Claude Code exit code: {result.returncode}")
        if result.stdout:
            print(f"  Output: {result.stdout[:500]}...")

    # Find and copy the transcript
    # Claude stores transcripts in ~/.claude/projects/<project-path-hash>/<session-id>.jsonl
    home = Path.home()
    claude_projects = home / ".claude" / "projects"

    transcript_found = False
    for project_dir_hash in claude_projects.iterdir():
        if not project_dir_hash.is_dir():
            continue
        transcript_file = project_dir_hash / f"{session_id}.jsonl"
        if transcript_file.exists():
            # Copy transcript to test directory
            dest_path = test.test_dir / "transcript.jsonl"
            shutil.copy(transcript_file, dest_path)
            transcript_found = True
            if verbose:
                print(f"  Copied transcript to {dest_path}")
            break

    if not transcript_found:
        return TestResult(
            name=test.name,
            passed=False,
            error=f"Transcript not found for session {session_id}",
        )

    # Get virtualenv environment
    env = get_venv_env(venv_dir)

    # Run post-condition if present
    post_condition_output = ""
    if test.post_condition:
        if verbose:
            print("  Running post-condition...")
        try:
            result = subprocess.run(
                ["python", str(test.post_condition)],
                check=True,
                cwd=temp_dir,
                capture_output=True,
                text=True,
                env=env,
            )
            post_condition_output = result.stdout
            if verbose and post_condition_output:
                print(f"  Post-condition output:\n{post_condition_output}")
        except subprocess.CalledProcessError as e:
            return TestResult(
                name=test.name,
                passed=False,
                error=f"Post-condition failed: {e.stderr}",
                post_condition_output=e.stdout,
            )

    return TestResult(name=test.name, passed=True, post_condition_output=post_condition_output)


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
        venv_dir = None

        try:
            # Create virtualenv for test isolation (outside test dir)
            venv_dir = create_virtualenv(temp_dir)

            setup_test_environment(temp_dir, test, project_dir, venv_dir)

            if mode == "replay":
                return run_replay(temp_dir, test, project_dir, venv_dir, verbose)
            elif mode == "record":
                return run_record(temp_dir, test, project_dir, venv_dir, verbose)
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
        finally:
            # Clean up venv directory (it's outside the temp dir)
            if venv_dir and venv_dir.exists():
                shutil.rmtree(venv_dir)


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
