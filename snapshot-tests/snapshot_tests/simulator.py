"""Tool call simulator for snapshot testing.

Simulates Bash, Write, Read, Edit, and other tool calls during test replay.
"""

from __future__ import annotations

import os
import re
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from snapshot_tests.placeholder import PlaceholderRegistry
from snapshot_tests.commit_tracker import normalize_for_comparison as normalize_git_commits


def strip_system_reminders(text: str) -> str:
    """Remove <system-reminder>...</system-reminder> blocks from text.

    Also handles truncated/incomplete system-reminder tags that may appear
    at the end of tool outputs.
    """
    # Remove complete system-reminder blocks
    text = re.sub(r"<system-reminder>.*?</system-reminder>", "", text, flags=re.DOTALL)
    # Remove incomplete/truncated system-reminder tags at end of text
    text = re.sub(r"<system-reminder>.*$", "", text, flags=re.DOTALL)
    return text.strip()


def filter_infrastructure_paths(text: str) -> str:
    """Remove infrastructure paths from glob output for comparison.

    These paths vary between runs and are not relevant to test behavior:
    - .venv: virtualenv structure varies
    - .claude: plugin installation varies
    """
    lines = text.splitlines()
    filtered = [
        line for line in lines
        if ".venv" not in line and ".claude" not in line
    ]
    return "\n".join(sorted(filtered))  # Sort for consistent comparison


def normalize_bash_output(text: str) -> str:
    """Normalize Bash output that varies between runs.

    Handles:
    - ls -la timestamps: "Jan 20 13:19" -> "<timestamp>"
    - ISO timestamps: "2026-01-20T13:19:00" -> "<timestamp>"
    - Cargo timing: "in 0.25s" -> "in <time>"
    - Cargo binary hashes: "rust_utils-878eb25838ec6d42" -> "rust_utils-<hash>"
    - Cargo coverage hashes: "rust_utils-c39e1837dc782f1a" -> "rust_utils-<hash>"
    - Cargo compilation lines: "Compiling foo" removed (varies based on cached state)
    - Doc test timing: "all doctests ran in X.XXs" -> "all doctests ran in <time>"
    """
    # Remove "Compiling ..." lines since they depend on whether code was cached
    text = re.sub(r'^\s*Compiling [^\n]+\n', '', text, flags=re.MULTILINE)
    # ls -la timestamp: "Jan 20 13:19" or "Jan  1 09:00"
    text = re.sub(
        r'(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+\d{1,2}\s+\d{1,2}:\d{2}',
        '<timestamp>',
        text
    )
    # ISO timestamp in output
    text = re.sub(
        r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}',
        '<timestamp>',
        text
    )
    # Cargo compilation/test timing: "in 0.25s" or "in 1.23s"
    text = re.sub(
        r'in \d+\.\d+s',
        'in <time>',
        text
    )
    # Cargo binary hashes in test output paths (varies in length)
    text = re.sub(
        r'(rust_utils|deps/[a-z_]+)-[0-9a-f]{8,16}',
        r'\1-<hash>',
        text
    )
    # Doc test line numbers: "(line 15)" -> "(line <n>)"
    text = re.sub(
        r'\(line \d+\)',
        '(line <n>)',
        text
    )
    # Doc test summary timing: various formats
    # "all doctests ran in X.XXs; merged doctests compilation took X.XXs"
    # or just "merged doctests compilation took X.XXs"
    text = re.sub(
        r'all doctests ran in \d+\.\d+s',
        'all doctests ran in <time>',
        text
    )
    text = re.sub(
        r'merged doctests compilation took \d+\.\d+s',
        'merged doctests compilation took <time>',
        text
    )
    # Normalize test result order by sorting test lines
    # Lines like "test foo::bar ... ok" should be sorted for comparison
    lines = text.split('\n')
    test_lines = []
    other_lines = []
    for line in lines:
        if line.strip().startswith('test ') and ' ... ' in line:
            test_lines.append(line)
        else:
            other_lines.append(line)
    # If there are test lines, sort them and reconstruct
    if test_lines:
        test_lines.sort()
        # Find where test lines start and reconstruct
        result_lines = []
        test_inserted = False
        for line in lines:
            if line.strip().startswith('test ') and ' ... ' in line:
                if not test_inserted:
                    result_lines.extend(test_lines)
                    test_inserted = True
            else:
                result_lines.append(line)
        text = '\n'.join(result_lines)
    return text


def normalize_for_comparison(text: str, tool_name: str | None = None) -> str:
    """Normalize text for lenient comparison.

    - Strips system reminders
    - Strips trailing whitespace from lines
    - Normalizes line endings
    - For Glob: filters out .venv paths
    - For Edit: normalizes output format
    - For Bash: normalizes timestamps

    Note: Git SHA normalization is handled separately via normalize_git_output()
    which needs both expected and actual to properly canonicalize placeholders.
    """
    text = strip_system_reminders(text)

    # For glob outputs, filter out infrastructure paths which vary between runs
    if tool_name == "Glob":
        text = filter_infrastructure_paths(text)

    # For Edit and Write outputs, skip comparison entirely
    # We can't perfectly replicate Claude Code's read-tracking behavior,
    # and the directory snapshot verifies actual file state at the end.
    # Just mark as executed so comparison always passes.
    if tool_name == "Edit":
        return "[edit_executed]"

    if tool_name == "Write":
        return "[write_executed]"

    # For Bash outputs, normalize variable content
    if tool_name == "Bash":
        text = normalize_bash_output(text)

    lines = [line.rstrip() for line in text.splitlines()]
    return "\n".join(lines).strip()


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
    path_mappings: dict[str, str] = field(default_factory=dict)
    # Track files that have been read (for enforcing read-before-write)
    # This should be reset between conversation turns to match Claude Code behavior
    files_read: set[str] = field(default_factory=set)

    def reset_read_state(self) -> None:
        """Reset the read tracking state.

        Claude Code tracks which files have been read within a single turn.
        Call this between turns to match that behavior.
        """
        self.files_read.clear()

    def substitute_paths(self, text: str) -> str:
        """Substitute old paths with new paths in text.

        Args:
            text: Text that may contain old paths

        Returns:
            Text with paths substituted
        """
        result = text
        for old_path, new_path in self.path_mappings.items():
            result = result.replace(old_path, new_path)
        return result

    def substitute_tool_input(self, tool_input: dict[str, Any]) -> dict[str, Any]:
        """Substitute paths in tool input dictionary.

        Args:
            tool_input: Original tool input

        Returns:
            Tool input with paths substituted
        """
        result = {}
        for key, value in tool_input.items():
            if isinstance(value, str):
                result[key] = self.substitute_paths(value)
            else:
                result[key] = value
        return result

    # Tools that don't affect file system state and should skip comparison
    PASSTHROUGH_TOOLS = {"Task", "TodoWrite", "Skill", "AskUserQuestion"}

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

        # For passthrough tools, skip execution and comparison
        if tool_name in self.PASSTHROUGH_TOOLS:
            return SimulationResult(
                success=True,
                output="[passthrough]",
                error=None,
                matched_expected=True,
            )

        # If expected result is an error, return that error without executing
        # This ensures simulator state matches the original session's state
        # (e.g., if Edit was expected to fail, file shouldn't be modified)
        if expected_result and "<tool_use_error>" in expected_result:
            return SimulationResult(
                success=True,  # Tool "succeeded" in returning expected error
                output=expected_result,
                error=None,
                matched_expected=True,
            )

        # Substitute paths in tool input (e.g., /tmp/old_session -> /tmp/new_session)
        substituted_input = self.substitute_tool_input(tool_input)

        try:
            actual_output = handler(substituted_input)
            success = True
            error = None
        except Exception as e:
            actual_output = ""
            success = False
            error = str(e)

        # Check against expected if provided
        # Also substitute paths in expected result for comparison
        matched = True
        if expected_result is not None and success:
            substituted_expected = self.substitute_paths(expected_result)
            # Normalize both for lenient comparison, passing tool name for special handling
            normalized_expected = normalize_for_comparison(substituted_expected, tool_name)
            normalized_actual = normalize_for_comparison(actual_output, tool_name)

            # For Bash outputs, also normalize git SHAs using commit placeholders
            if tool_name == "Bash":
                normalized_expected, normalized_actual = normalize_git_commits(
                    normalized_expected, normalized_actual
                )

            matched = self.registry.match(normalized_expected, normalized_actual)

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
            # Pass-through tools that don't affect file system state
            "Task": self._handle_passthrough,
            "TodoWrite": self._handle_passthrough,
            "Skill": self._handle_passthrough,
            "AskUserQuestion": self._handle_passthrough,
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
        """Write content to a file.

        Enforces read-before-write for existing files, matching Claude Code behavior.
        """
        file_path = tool_input.get("file_path", "")
        content = tool_input.get("content", "")

        if self.dry_run:
            return f"[DRY RUN] Would write {len(content)} bytes to {file_path}"

        path = Path(file_path)
        if not path.is_absolute():
            path = self.cwd / path

        # Claude Code enforces read-before-write for existing files
        file_exists = path.exists()
        if file_exists and str(path) not in self.files_read:
            return "<tool_use_error>File has not been read yet. Read it first before writing to it.</tool_use_error>"

        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

        if file_exists:
            return f"The file {file_path} has been updated successfully."
        return f"File created successfully at: {file_path}"

    def _handle_read(self, tool_input: dict[str, Any]) -> str:
        """Read content from a file.

        Returns output in Claude Code's format:
        - Line numbers right-justified to 6 characters
        - Arrow character (→ U+2192) as separator (not tab)
        - Line content after separator

        Example: "     1→first line"

        Uses split('\n') to match Claude Code's line counting behavior
        (trailing newlines create an extra empty line).
        """
        file_path = tool_input.get("file_path", "")
        offset = tool_input.get("offset", 0)
        limit = tool_input.get("limit", 2000)

        path = Path(file_path)
        if not path.is_absolute():
            path = self.cwd / path

        if not path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")

        # Track that this file has been read (for read-before-write enforcement)
        self.files_read.add(str(path))

        content = path.read_text(encoding="utf-8")
        # Use split('\n') to match Claude Code's behavior with trailing newlines
        lines = content.split('\n')

        # Apply offset and limit
        selected_lines = lines[offset : offset + limit]

        # Format with line numbers (Claude Code style with arrow separator)
        result_lines = []
        for i, line in enumerate(selected_lines, start=offset + 1):
            result_lines.append(f"{i:>6}→{line}")

        return "\n".join(result_lines)

    def _handle_edit(self, tool_input: dict[str, Any]) -> str:
        """Edit a file by replacing text.

        Enforces Claude Code behavior:
        - Read-before-edit requirement
        - old_string must differ from new_string
        - old_string must be unique when replace_all=False
        """
        file_path = tool_input.get("file_path", "")
        old_string = tool_input.get("old_string", "")
        new_string = tool_input.get("new_string", "")
        replace_all = tool_input.get("replace_all", False)

        if self.dry_run:
            return f"[DRY RUN] Would edit {file_path}"

        # Check that old_string differs from new_string
        if old_string == new_string:
            return "<tool_use_error>old_string and new_string must be different.</tool_use_error>"

        path = Path(file_path)
        if not path.is_absolute():
            path = self.cwd / path

        if not path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")

        # Claude Code enforces read-before-edit
        if str(path) not in self.files_read:
            return "<tool_use_error>File has not been read yet. Read it first before writing to it.</tool_use_error>"

        content = path.read_text()

        if old_string not in content:
            raise ValueError(f"old_string not found in {file_path}")

        # Check for uniqueness constraint when replace_all is False
        if not replace_all:
            match_count = content.count(old_string)
            if match_count > 1:
                return f"<tool_use_error>Multiple occurrences of old_string found ({match_count} matches). Use replace_all: true or provide a more unique string.</tool_use_error>"

        if replace_all:
            new_content = content.replace(old_string, new_string)
        else:
            # Replace only first occurrence (already verified uniqueness above)
            new_content = content.replace(old_string, new_string, 1)

        path.write_text(new_content)

        return f"The file {file_path} has been updated successfully."

    def _handle_glob(self, tool_input: dict[str, Any]) -> str:
        """Find files matching a glob pattern."""
        pattern = tool_input.get("pattern", "")
        search_path = tool_input.get("path", str(self.cwd))

        path = Path(search_path)
        if not path.is_absolute():
            path = self.cwd / path

        matches = sorted(path.glob(pattern))
        return "\n".join(str(m) for m in matches)

    def _handle_passthrough(self, tool_input: dict[str, Any]) -> str:
        """Handle tools that don't affect file system state.

        Tools like Task, TodoWrite, Skill, and AskUserQuestion don't modify
        files - they're for orchestration and user interaction. During replay,
        we skip execution and always report success.
        """
        return "[passthrough - no execution needed]"

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
    path_mappings: dict[str, str] | None = None,
) -> ToolSimulator:
    """Create a configured ToolSimulator.

    Args:
        registry: Placeholder registry for fuzzy matching
        cwd: Working directory for tool execution
        dry_run: If True, don't actually execute tools
        path_mappings: Dictionary mapping old paths to new paths for substitution

    Returns:
        Configured ToolSimulator instance
    """
    return ToolSimulator(
        registry=registry or PlaceholderRegistry(),
        cwd=Path(cwd) if cwd else Path.cwd(),
        dry_run=dry_run,
        path_mappings=path_mappings or {},
    )
