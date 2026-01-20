"""Tool call simulator for snapshot testing.

Simulates Bash, Write, Read, Edit, and other tool calls during test replay.
"""

from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from snapshot_tests.placeholder import PlaceholderRegistry


@dataclass
class SimulationResult:
    """Result from simulating a tool call."""

    success: bool
    output: str
    error: str | None = None
    matched_expected: bool = True


@dataclass
class ToolSimulator:
    """Simulates tool calls during test replay.

    In replay mode:
    - Executes the actual tool
    - Compares result against expected (with placeholders)
    - Returns expected result for consistency

    This allows verifying that tools produce expected output while
    maintaining deterministic test behavior.
    """

    registry: PlaceholderRegistry = field(default_factory=PlaceholderRegistry)
    cwd: Path = field(default_factory=Path.cwd)
    dry_run: bool = False

    def simulate(
        self,
        tool_name: str,
        tool_input: dict[str, Any],
        expected_result: str | None = None,
    ) -> SimulationResult:
        """Simulate a tool call and optionally verify against expected output.

        Args:
            tool_name: Name of the tool (Bash, Write, Read, Edit, Glob, Grep)
            tool_input: Tool input parameters
            expected_result: Expected output (may contain placeholders)

        Returns:
            SimulationResult with actual output and match status
        """
        handler = self._get_handler(tool_name)
        if not handler:
            return SimulationResult(
                success=False,
                output="",
                error=f"Unknown tool: {tool_name}",
                matched_expected=False,
            )

        try:
            actual_output = handler(tool_input)
            success = True
            error = None
        except Exception as e:
            actual_output = ""
            success = False
            error = str(e)

        # Check against expected if provided
        matched = True
        if expected_result is not None and success:
            matched = self.registry.match(expected_result, actual_output)

        return SimulationResult(
            success=success,
            output=actual_output,
            error=error,
            matched_expected=matched,
        )

    def _get_handler(self, tool_name: str):
        """Get the handler function for a tool."""
        handlers = {
            "Bash": self._handle_bash,
            "Write": self._handle_write,
            "Read": self._handle_read,
            "Edit": self._handle_edit,
            "Glob": self._handle_glob,
            "Grep": self._handle_grep,
        }
        return handlers.get(tool_name)

    def _handle_bash(self, tool_input: dict[str, Any]) -> str:
        """Execute a bash command."""
        command = tool_input.get("command", "")
        timeout = tool_input.get("timeout", 120000) / 1000  # Convert ms to seconds

        if self.dry_run:
            return f"[DRY RUN] Would execute: {command}"

        result = subprocess.run(
            command,
            shell=True,
            cwd=self.cwd,
            capture_output=True,
            text=True,
            timeout=timeout,
        )

        output = result.stdout
        if result.stderr:
            output += result.stderr

        return output.rstrip()

    def _handle_write(self, tool_input: dict[str, Any]) -> str:
        """Write content to a file."""
        file_path = tool_input.get("file_path", "")
        content = tool_input.get("content", "")

        if self.dry_run:
            return f"[DRY RUN] Would write {len(content)} bytes to {file_path}"

        path = Path(file_path)
        if not path.is_absolute():
            path = self.cwd / path

        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

        return f"File written: {file_path}"

    def _handle_read(self, tool_input: dict[str, Any]) -> str:
        """Read content from a file."""
        file_path = tool_input.get("file_path", "")
        offset = tool_input.get("offset", 0)
        limit = tool_input.get("limit", 2000)

        path = Path(file_path)
        if not path.is_absolute():
            path = self.cwd / path

        if not path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")

        with open(path, encoding="utf-8") as f:
            lines = f.readlines()

        # Apply offset and limit
        selected_lines = lines[offset : offset + limit]

        # Format with line numbers (cat -n style)
        result_lines = []
        for i, line in enumerate(selected_lines, start=offset + 1):
            result_lines.append(f"{i:>6}\t{line.rstrip()}")

        return "\n".join(result_lines)

    def _handle_edit(self, tool_input: dict[str, Any]) -> str:
        """Edit a file by replacing text."""
        file_path = tool_input.get("file_path", "")
        old_string = tool_input.get("old_string", "")
        new_string = tool_input.get("new_string", "")
        replace_all = tool_input.get("replace_all", False)

        if self.dry_run:
            return f"[DRY RUN] Would edit {file_path}"

        path = Path(file_path)
        if not path.is_absolute():
            path = self.cwd / path

        if not path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")

        content = path.read_text()

        if old_string not in content:
            raise ValueError(f"old_string not found in {file_path}")

        if replace_all:
            new_content = content.replace(old_string, new_string)
        else:
            # Replace only first occurrence
            new_content = content.replace(old_string, new_string, 1)

        path.write_text(new_content)

        return f"File edited: {file_path}"

    def _handle_glob(self, tool_input: dict[str, Any]) -> str:
        """Find files matching a glob pattern."""
        pattern = tool_input.get("pattern", "")
        search_path = tool_input.get("path", str(self.cwd))

        path = Path(search_path)
        if not path.is_absolute():
            path = self.cwd / path

        matches = sorted(path.glob(pattern))
        return "\n".join(str(m) for m in matches)

    def _handle_grep(self, tool_input: dict[str, Any]) -> str:
        """Search for patterns in files."""
        pattern = tool_input.get("pattern", "")
        search_path = tool_input.get("path", str(self.cwd))
        output_mode = tool_input.get("output_mode", "files_with_matches")

        if self.dry_run:
            return f"[DRY RUN] Would grep for {pattern} in {search_path}"

        # Use ripgrep if available, fall back to grep
        rg_args = ["rg", "--no-heading"]

        if output_mode == "files_with_matches":
            rg_args.append("-l")
        elif output_mode == "count":
            rg_args.append("-c")
        # For "content" mode, default output works

        rg_args.extend([pattern, search_path])

        try:
            result = subprocess.run(
                rg_args,
                capture_output=True,
                text=True,
                cwd=self.cwd,
            )
            return result.stdout.rstrip()
        except FileNotFoundError:
            # Fall back to grep
            grep_args = ["grep", "-r"]
            if output_mode == "files_with_matches":
                grep_args.append("-l")
            elif output_mode == "count":
                grep_args.append("-c")
            grep_args.extend([pattern, search_path])

            result = subprocess.run(
                grep_args,
                capture_output=True,
                text=True,
                cwd=self.cwd,
            )
            return result.stdout.rstrip()


def create_simulator(
    registry: PlaceholderRegistry | None = None,
    cwd: Path | str | None = None,
    dry_run: bool = False,
) -> ToolSimulator:
    """Create a configured ToolSimulator.

    Args:
        registry: Placeholder registry for fuzzy matching
        cwd: Working directory for tool execution
        dry_run: If True, don't actually execute tools

    Returns:
        Configured ToolSimulator instance
    """
    return ToolSimulator(
        registry=registry or PlaceholderRegistry(),
        cwd=Path(cwd) if cwd else Path.cwd(),
        dry_run=dry_run,
    )
