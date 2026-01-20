"""PTY-based Claude Code runner for snapshot tests.

Runs Claude Code in an interactive PTY session to ensure proper session
lifecycle, including Stop hooks. This replaces --print mode which bypasses
session lifecycle events.
"""

from __future__ import annotations

import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path

import pexpect


@dataclass
class PTYRunResult:
    """Result from running Claude Code via PTY."""

    success: bool
    exit_code: int | None
    output: str
    error: str | None = None


def run_claude_pty(
    prompt: str,
    cwd: Path,
    session_id: str,
    plugin_dir: Path,
    timeout: int = 300,
    env: dict[str, str] | None = None,
    verbose: bool = False,
) -> PTYRunResult:
    """Run Claude Code in a PTY session.

    This runs Claude Code interactively (not --print mode) so that the full
    session lifecycle runs, including Stop hooks on exit.

    Args:
        prompt: The prompt to send to Claude
        cwd: Working directory for Claude
        session_id: Session ID for transcript identification
        plugin_dir: Directory containing the plugin to load
        timeout: Maximum time to wait in seconds
        env: Optional environment variables
        verbose: Print debug output

    Returns:
        PTYRunResult with exit code and output
    """
    # Build command - use -p for prompt but NOT --print
    # This sends the prompt non-interactively but runs full session lifecycle
    cmd = [
        "claude",
        "--session-id", session_id,
        "--dangerously-skip-permissions",
        "--plugin-dir", str(plugin_dir),
        "--model", "opus",
        "-p", prompt,
    ]

    if verbose:
        print(f"  PTY command: {' '.join(cmd[:5])}...")

    # Set up environment
    spawn_env = env.copy() if env else os.environ.copy()
    # Ensure TERM is set for pexpect
    spawn_env["TERM"] = "dumb"

    try:
        # Spawn Claude Code in a PTY
        child = pexpect.spawn(
            cmd[0],
            cmd[1:],
            cwd=str(cwd),
            env=spawn_env,
            timeout=timeout,
            encoding="utf-8",
            codec_errors="replace",
        )

        # Collect all output
        output_lines = []

        # Read output until EOF or timeout
        # Claude will output its response and then exit
        while True:
            try:
                # Read line by line
                line = child.readline()
                if not line:
                    break
                # Strip ANSI codes for cleaner output
                clean_line = strip_ansi(line.rstrip())
                output_lines.append(clean_line)
                if verbose:
                    print(f"    > {clean_line}")
            except pexpect.TIMEOUT:
                if verbose:
                    print("  PTY timeout waiting for output")
                break
            except pexpect.EOF:
                break

        # Wait for process to fully exit
        child.wait()
        exit_code = child.exitstatus

        if verbose:
            print(f"  PTY exit code: {exit_code}")

        return PTYRunResult(
            success=(exit_code == 0),
            exit_code=exit_code,
            output="\n".join(output_lines),
        )

    except pexpect.ExceptionPexpect as e:
        return PTYRunResult(
            success=False,
            exit_code=None,
            output="",
            error=str(e),
        )


def strip_ansi(text: str) -> str:
    """Remove ANSI escape codes from text."""
    ansi_pattern = re.compile(r'\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07')
    return ansi_pattern.sub('', text)
