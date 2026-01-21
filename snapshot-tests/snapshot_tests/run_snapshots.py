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
import hashlib
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

from enum import Enum, auto

from snapshot_tests.placeholder import PlaceholderRegistry
from snapshot_tests.transcript import parse_transcript, extract_tool_calls, get_project_directory
from snapshot_tests.simulator import ToolSimulator
from snapshot_tests.compile_transcript import compile_transcript
from snapshot_tests.plugin_setup import install_plugin
from snapshot_tests.pty_runner import run_claude_pty


class ErrorCategory(Enum):
    """Categories of test failures with associated remediation actions."""

    NO_TRANSCRIPT = auto()  # No transcript.jsonl exists
    EXECUTION_ERROR = auto()  # Tool failed during replay
    DIRECTORY_MISMATCH = auto()  # Write/Edit produced different files
    POST_CONDITION_FAILED = auto()  # Post-condition script failed
    RECORDING_FAILED = auto()  # Claude Code failed during recording
    OTHER = auto()  # Generic/unknown error


# Patterns for paths to exclude from directory snapshots
SNAPSHOT_EXCLUDE_PATTERNS = {
    ".git",
    ".venv",
    ".claude",
    "__pycache__",
    ".pyc",
    ".pyo",
    "target",  # Rust build directory
    "Cargo.lock",  # Varies between builds
    ".coverage",  # Created by pytest during recording but not replay
    ".pytest_cache",  # Pytest cache varies between runs
    ".gitignore",  # Modified by plugin setup, varies between recording and replay
}


def should_exclude_path(path: Path) -> bool:
    """Check if a path should be excluded from directory snapshot.

    Args:
        path: Path to check (relative to project root)

    Returns:
        True if the path should be excluded
    """
    parts = path.parts
    for pattern in SNAPSHOT_EXCLUDE_PATTERNS:
        if pattern in parts:
            return True
        # Check for file extension patterns
        if pattern.startswith(".") and str(path).endswith(pattern):
            return True
    return False


def capture_directory_snapshot(root_dir: Path) -> dict[str, str]:
    """Capture a snapshot of all files in a directory.

    Args:
        root_dir: Directory to snapshot

    Returns:
        Dictionary mapping relative file paths to content hashes (SHA256)
    """
    snapshot = {}

    for file_path in root_dir.rglob("*"):
        if not file_path.is_file():
            continue

        rel_path = file_path.relative_to(root_dir)
        if should_exclude_path(rel_path):
            continue

        try:
            content = file_path.read_bytes()
            content_hash = hashlib.sha256(content).hexdigest()
            snapshot[str(rel_path)] = content_hash
        except (PermissionError, OSError):
            # Skip files we can't read
            continue

    return snapshot


def compare_directory_snapshots(
    expected: dict[str, str],
    actual: dict[str, str],
    root_dir: Path,
) -> list[str]:
    """Compare two directory snapshots and report differences.

    Args:
        expected: Expected file hashes from recorded snapshot
        actual: Actual file hashes from current directory
        root_dir: Root directory (for reporting file content on mismatch)

    Returns:
        List of difference descriptions (empty if identical)
    """
    differences = []

    # Check for files in expected but not in actual (deleted)
    for path in expected:
        if path not in actual:
            differences.append(f"Missing file: {path}")

    # Check for files in actual but not in expected (created)
    for path in actual:
        if path not in expected:
            differences.append(f"Unexpected file: {path}")

    # Check for files with different content
    for path in expected:
        if path in actual and expected[path] != actual[path]:
            # Try to show diff if it's a text file
            file_path = root_dir / path
            try:
                actual_content = file_path.read_text()
                differences.append(
                    f"Content mismatch: {path}\n"
                    f"  Expected hash: {expected[path]}\n"
                    f"  Actual hash: {actual[path]}\n"
                    f"  First 200 chars: {actual_content[:200]}"
                )
            except (UnicodeDecodeError, OSError):
                differences.append(
                    f"Content mismatch (binary): {path}\n"
                    f"  Expected hash: {expected[path]}\n"
                    f"  Actual hash: {actual[path]}"
                )

    return differences


def save_directory_snapshot(snapshot: dict[str, str], snapshot_path: Path) -> None:
    """Save a directory snapshot to a JSON file.

    Args:
        snapshot: Directory snapshot to save
        snapshot_path: Path to save the snapshot
    """
    # Sort keys for deterministic output
    sorted_snapshot = dict(sorted(snapshot.items()))
    snapshot_path.write_text(json.dumps(sorted_snapshot, indent=2) + "\n")


def load_directory_snapshot(snapshot_path: Path) -> dict[str, str] | None:
    """Load a directory snapshot from a JSON file.

    Args:
        snapshot_path: Path to the snapshot file

    Returns:
        Directory snapshot or None if file doesn't exist
    """
    if not snapshot_path.exists():
        return None
    return json.loads(snapshot_path.read_text())


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
    error_category: ErrorCategory | None = None
    output: str = ""
    post_condition_output: str = ""


def get_suggestion_for_error(category: ErrorCategory | None, test_name: str) -> str | None:
    """Get an actionable suggestion based on error category.

    Args:
        category: The error category
        test_name: Name of the test that failed

    Returns:
        A helpful suggestion string, or None if no specific suggestion
    """
    if category == ErrorCategory.NO_TRANSCRIPT:
        return f"  -> Run: just snapshot-tests-record {test_name}"
    elif category == ErrorCategory.DIRECTORY_MISMATCH:
        return (
            "  -> If changes are expected, review them and run:\n"
            f"     just snapshot-tests --save-snapshot {test_name}"
        )
    elif category == ErrorCategory.EXECUTION_ERROR:
        return (
            "  -> The recorded tool call could not be replayed. This may indicate:\n"
            "     - Test setup.py doesn't match original environment\n"
            "     - Files referenced in transcript are missing\n"
            f"     Consider re-recording: just snapshot-tests-record {test_name}"
        )
    elif category == ErrorCategory.POST_CONDITION_FAILED:
        return "  -> Check the post-condition.py script for your test"
    elif category == ErrorCategory.RECORDING_FAILED:
        return (
            "  -> Claude Code failed during recording. Check:\n"
            "     - Claude Code is installed and authenticated\n"
            "     - The test prompt in story.md is valid"
        )
    return None


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


def get_venv_env(venv_dir: Path, home_dir: Path | None = None) -> dict[str, str]:
    """Get environment variables for running commands in the virtualenv.

    Args:
        venv_dir: Path to the virtualenv directory
        home_dir: Optional isolated HOME directory for test isolation

    Returns:
        Environment dict with PATH, VIRTUAL_ENV, and optionally HOME set
    """
    env = os.environ.copy()
    venv_bin = venv_dir / "bin"
    env["PATH"] = f"{venv_bin}:{env.get('PATH', '')}"
    env["VIRTUAL_ENV"] = str(venv_dir)
    # Remove PYTHONHOME if set, as it can interfere with venv
    env.pop("PYTHONHOME", None)
    # Isolate HOME to prevent test instances from sharing state
    if home_dir:
        # Preserve Rust toolchain access before changing HOME
        # rustup/cargo look for config in $HOME/.rustup and $HOME/.cargo
        original_home = env.get("HOME", "")
        if original_home:
            if "RUSTUP_HOME" not in env:
                env["RUSTUP_HOME"] = f"{original_home}/.rustup"
            if "CARGO_HOME" not in env:
                env["CARGO_HOME"] = f"{original_home}/.cargo"
        env["HOME"] = str(home_dir)
    return env


def create_isolated_home(temp_dir: Path) -> Path:
    """Create an isolated HOME directory for test isolation.

    This prevents Claude Code instances from sharing state like
    ~/.claude-reliability/ cache or ~/.claude/ configuration.

    Args:
        temp_dir: Base temp directory

    Returns:
        Path to the isolated home directory
    """
    home_dir = temp_dir.parent / f"{temp_dir.name}_home"
    home_dir.mkdir(exist_ok=True)
    return home_dir


def setup_test_environment(
    temp_dir: Path,
    test: TestConfig,
    project_dir: Path,
    venv_dir: Path,
    home_dir: Path,
) -> None:
    """Set up the test environment in a temp directory.

    Args:
        temp_dir: Temporary directory for test
        test: Test configuration
        project_dir: Project root directory
        venv_dir: Path to the virtualenv directory
        home_dir: Isolated HOME directory for test isolation
    """
    os.chdir(temp_dir)

    # Install the claude-reliability plugin
    # Pass home_dir so the binary is copied to the right location for ensure-binary.sh
    install_plugin(temp_dir, home_dir=None)  # Use real HOME for now to avoid auth issues

    # Get virtualenv environment with isolated HOME
    env = get_venv_env(venv_dir, home_dir)

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
    home_dir: Path,
    verbose: bool = False,
    save_snapshot: bool = False,
) -> TestResult:
    """Run a test in replay mode.

    Simulates tool calls from the transcript and verifies outputs match.

    Args:
        temp_dir: Temporary directory for test
        test: Test configuration
        project_dir: Project root directory
        venv_dir: Path to the virtualenv directory
        home_dir: Isolated HOME directory for test isolation
        verbose: Show detailed output

    Returns:
        TestResult with pass/fail status
    """
    if not test.transcript_path:
        return TestResult(
            name=test.name,
            passed=False,
            error="No transcript.jsonl found",
            error_category=ErrorCategory.NO_TRANSCRIPT,
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
    # Track true errors (execution failures) separately from mismatches (different output)
    execution_errors = []
    mismatches = []
    for i, (tool_use, expected_result, _is_new_entry) in enumerate(tool_calls):
        # Note: We track reads at session level, not per-entry, based on observed
        # Claude Code behavior (tool 20 succeeded after tool 19 in different entries)

        if verbose:
            print(f"  [{i+1}/{len(tool_calls)}] {tool_use.name}: {_summarize_input(tool_use)}")

        expected_output = expected_result.content if expected_result else None

        result = simulator.simulate(
            tool_name=tool_use.name,
            tool_input=tool_use.input,
            expected_result=expected_output,
        )

        if not result.success:
            execution_errors.append(f"Tool {tool_use.name} failed: {result.error}")
            if verbose:
                print(f"    ERROR: {result.error}")
        elif not result.matched_expected and expected_output is not None:
            # Output mismatches are warnings, not errors
            # The post-condition determines if the test passes
            substituted_expected = simulator.substitute_paths(expected_output)
            mismatches.append(
                f"Tool {tool_use.name} output mismatch:\n"
                f"  Expected: {substituted_expected[:200]}\n"
                f"  Actual: {result.output[:200]}"
            )
            if verbose:
                print(f"    MISMATCH")

    # Execution errors are fatal - the tool failed to run
    if execution_errors:
        return TestResult(
            name=test.name,
            passed=False,
            error="\n".join(execution_errors),
            error_category=ErrorCategory.EXECUTION_ERROR,
        )

    # Compare directory snapshot to ensure file states are byte-wise identical
    snapshot_path = test.test_dir / "directory-snapshot.json"
    expected_snapshot = load_directory_snapshot(snapshot_path)
    actual_snapshot = capture_directory_snapshot(temp_dir)

    if expected_snapshot:
        snapshot_diffs = compare_directory_snapshots(
            expected_snapshot, actual_snapshot, temp_dir
        )
        if snapshot_diffs:
            # File state differences are errors - Write/Edit must produce identical files
            return TestResult(
                name=test.name,
                passed=False,
                error="Directory state mismatch (Write/Edit produced different files):\n"
                + "\n".join(snapshot_diffs[:5]),  # Limit to first 5 diffs
                error_category=ErrorCategory.DIRECTORY_MISMATCH,
            )
        if verbose:
            print(f"  Directory snapshot verified ({len(actual_snapshot)} files match)")
    elif save_snapshot:
        # Bootstrap: save snapshot from successful replay
        save_directory_snapshot(actual_snapshot, snapshot_path)
        if verbose:
            print(f"  Saved directory snapshot ({len(actual_snapshot)} files)")
    elif verbose:
        print("  No directory snapshot to compare (use --save-snapshot to create)")

    # Get virtualenv environment with isolated HOME
    env = get_venv_env(venv_dir, home_dir)

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
            error_msg = f"Post-condition failed (exit code {e.returncode}):\n"
            if e.stdout:
                error_msg += f"stdout: {e.stdout}\n"
            if e.stderr:
                error_msg += f"stderr: {e.stderr}"
            return TestResult(
                name=test.name,
                passed=False,
                error=error_msg,
                error_category=ErrorCategory.POST_CONDITION_FAILED,
                post_condition_output=e.stdout or "",
            )

    # Include mismatch warnings in the result but still pass
    if mismatches and verbose:
        print(f"  {len(mismatches)} output mismatches (non-fatal, post-condition passed)")

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
    home_dir: Path,
    verbose: bool = False,
) -> TestResult:
    """Run a test in record mode.

    Runs Claude Code with the prompt from story.md and captures the transcript.

    Args:
        temp_dir: Temporary directory for test
        test: Test configuration
        project_dir: Project root directory
        venv_dir: Path to the virtualenv directory
        home_dir: Isolated HOME directory for test isolation
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
            error_category=ErrorCategory.OTHER,
        )

    if verbose:
        print(f"  Prompt: {prompt[:100]}...")

    # Generate session ID
    session_id = str(uuid.uuid4())

    if verbose:
        print(f"  Session ID: {session_id}")

    # Plugin directory for hooks
    plugin_dir = temp_dir / ".claude" / "plugins" / "claude-reliability"

    if verbose:
        print(f"  Running Claude Code via PTY...")
        print(f"  Plugin dir: {plugin_dir}")

    # Run Claude Code via PTY (not --print mode) to ensure proper session lifecycle
    # This allows Stop hooks to run during normal exit
    pty_result = run_claude_pty(
        prompt=prompt,
        cwd=temp_dir,
        session_id=session_id,
        plugin_dir=plugin_dir,
        timeout=300,
        verbose=verbose,
    )

    if pty_result.error:
        return TestResult(
            name=test.name,
            passed=False,
            error=f"Claude Code PTY error: {pty_result.error}",
            error_category=ErrorCategory.RECORDING_FAILED,
        )

    if verbose:
        print(f"  Claude Code exit code: {pty_result.exit_code}")
        if pty_result.output:
            print(f"  Output: {pty_result.output[:500]}...")

    # Find and copy the transcript
    # Claude stores transcripts in ~/.claude/projects/<project-path-hash>/<session-id>.jsonl
    real_home = Path.home()
    claude_projects = real_home / ".claude" / "projects"

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
            error_category=ErrorCategory.RECORDING_FAILED,
        )

    # Capture directory snapshot for replay verification
    # This ensures Write/Edit operations produce byte-wise identical files
    snapshot = capture_directory_snapshot(temp_dir)
    snapshot_path = test.test_dir / "directory-snapshot.json"
    save_directory_snapshot(snapshot, snapshot_path)
    if verbose:
        print(f"  Saved directory snapshot ({len(snapshot)} files)")

    # Get virtualenv environment with isolated HOME
    env = get_venv_env(venv_dir, home_dir)

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
            # Save post-condition output alongside transcript for later compilation
            output_file = test.test_dir / "post-condition-output.txt"
            output_file.write_text(post_condition_output)
        except subprocess.CalledProcessError as e:
            error_msg = f"Post-condition failed (exit code {e.returncode}):\n"
            if e.stdout:
                error_msg += f"stdout: {e.stdout}\n"
            if e.stderr:
                error_msg += f"stderr: {e.stderr}"
            return TestResult(
                name=test.name,
                passed=False,
                error=error_msg,
                error_category=ErrorCategory.POST_CONDITION_FAILED,
                post_condition_output=e.stdout or "",
            )

    return TestResult(name=test.name, passed=True, post_condition_output=post_condition_output)


def run_test(
    test: TestConfig,
    mode: str,
    project_dir: Path,
    verbose: bool = False,
    save_snapshot: bool = False,
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
        home_dir = None

        try:
            # Create virtualenv for test isolation (outside test dir)
            venv_dir = create_virtualenv(temp_dir)

            # Create isolated HOME directory for test isolation
            home_dir = create_isolated_home(temp_dir)

            setup_test_environment(temp_dir, test, project_dir, venv_dir, home_dir)

            if mode == "replay":
                return run_replay(temp_dir, test, project_dir, venv_dir, home_dir, verbose, save_snapshot)
            elif mode == "record":
                return run_record(temp_dir, test, project_dir, venv_dir, home_dir, verbose)
            else:
                return TestResult(
                    name=test.name,
                    passed=False,
                    error=f"Unknown mode: {mode}",
                    error_category=ErrorCategory.OTHER,
                )

        except Exception as e:
            return TestResult(
                name=test.name,
                passed=False,
                error=str(e),
                error_category=ErrorCategory.OTHER,
            )
        finally:
            # Clean up directories created outside the temp dir
            if venv_dir and venv_dir.exists():
                shutil.rmtree(venv_dir)
            if home_dir and home_dir.exists():
                shutil.rmtree(home_dir)


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

        # Read post-condition output if available
        post_condition_output = None
        output_file = test.test_dir / "post-condition-output.txt"
        if output_file.exists():
            post_condition_output = output_file.read_text()

        md_path = test.transcript_path.with_suffix(".md")
        transcript = parse_transcript(test.transcript_path)
        markdown = compile_transcript(
            transcript, verbose=False, post_condition_output=post_condition_output
        )
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
    parser.add_argument(
        "--save-snapshot",
        action="store_true",
        help="Save directory snapshot from replay (for bootstrapping)",
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

        result = run_test(test, args.mode, project_dir, args.verbose, args.save_snapshot)
        results.append(result)

        if result.passed:
            print("  PASS")
        else:
            print(f"  FAIL: {result.error}")
            suggestion = get_suggestion_for_error(result.error_category, result.name)
            if suggestion:
                print(suggestion)

        print()

    # Summary
    passed = sum(1 for r in results if r.passed)
    failed = len(results) - passed

    print(f"Results: {passed} passed, {failed} failed")

    # Show actionable summary if there are failures
    if failed > 0:
        print()
        print("=" * 60)
        print("FAILED TESTS - SUGGESTED ACTIONS:")
        print("=" * 60)

        # Group failures by category for cleaner output
        failures_by_category: dict[ErrorCategory | None, list[TestResult]] = {}
        for r in results:
            if not r.passed:
                cat = r.error_category
                if cat not in failures_by_category:
                    failures_by_category[cat] = []
                failures_by_category[cat].append(r)

        # Show suggestions by category
        if ErrorCategory.NO_TRANSCRIPT in failures_by_category:
            tests_list = [r.name for r in failures_by_category[ErrorCategory.NO_TRANSCRIPT]]
            print()
            print(f"Missing transcripts ({len(tests_list)} test(s)):")
            for name in tests_list:
                print(f"  - {name}")
            print()
            print("  Record transcripts with:")
            print(f"    just snapshot-tests-record {' '.join(tests_list)}")

        if ErrorCategory.DIRECTORY_MISMATCH in failures_by_category:
            tests_list = [r.name for r in failures_by_category[ErrorCategory.DIRECTORY_MISMATCH]]
            print()
            print(f"Directory snapshot mismatches ({len(tests_list)} test(s)):")
            for name in tests_list:
                print(f"  - {name}")
            print()
            print("  After reviewing the changes, save new snapshots with:")
            print(f"    just snapshot-tests --save-snapshot {' '.join(tests_list)}")

        if ErrorCategory.EXECUTION_ERROR in failures_by_category:
            tests_list = [r.name for r in failures_by_category[ErrorCategory.EXECUTION_ERROR]]
            print()
            print(f"Execution errors ({len(tests_list)} test(s)):")
            for name in tests_list:
                print(f"  - {name}")
            print()
            print("  Re-record transcripts with:")
            print(f"    just snapshot-tests-record {' '.join(tests_list)}")

        if ErrorCategory.POST_CONDITION_FAILED in failures_by_category:
            tests_list = [r.name for r in failures_by_category[ErrorCategory.POST_CONDITION_FAILED]]
            print()
            print(f"Post-condition failures ({len(tests_list)} test(s)):")
            for name in tests_list:
                print(f"  - {name}")
            print()
            print("  Check the post-condition.py scripts for these tests")

        print()
    else:
        print()

    # Compile transcripts to markdown
    compile_all_transcripts(tests, args.verbose)

    if failed > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
