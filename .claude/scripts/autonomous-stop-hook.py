#!/usr/bin/env python3
"""
Autonomous mode stop hook.

This hook runs when Claude attempts to stop/exit. It implements the autonomous
development loop by:
1. Checking if autonomous mode is active
2. Evaluating stopping conditions
3. Blocking exit if work should continue
4. Providing guidance on what to do next
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any, cast

import yaml


@dataclass
class BeadsInteractionStatus:
    """Status of agent's interaction with beads during this session."""

    has_interaction: bool  # True if .beads/ was modified (staged, unstaged, or commits)
    already_warned: bool  # True if we've already warned about missing interaction


# Magic string that allows stopping when human input is required
HUMAN_INPUT_REQUIRED = (
    "I have completed all work that I can and require human input to proceed."
)

# Number of iterations without issue changes before stopping
STALENESS_THRESHOLD = 5

SESSION_FILE = Path(".claude/autonomous-session.local.md")
BUILD_FAILING_MARKER = Path(".claude/build-already-failing.local")
BEADS_WARNING_MARKER = Path(".claude/beads-warning-given.local")

# Time window for considering user as "recently active"
USER_RECENCY_MINUTES = 5

# Patterns that suggest the assistant is asking a question
QUESTION_PATTERNS = [
    r"\?\s*$",  # Ends with question mark
    r"\?\s*\n",  # Question mark before newline
    r"(?i)\bwould you like\b",
    r"(?i)\bdo you want\b",
    r"(?i)\bshould I\b",
    r"(?i)\bcan you\b.*\?",
    r"(?i)\bwhat do you think\b",
    r"(?i)\blet me know\b",
    r"(?i)\bplease confirm\b",
    r"(?i)\bplease clarify\b",
    r"(?i)\bwhich (?:one|option)\b",
    r"(?i)\bhow would you like\b",
    r"(?i)\bwhat would you prefer\b",
]

# Patterns for "should I continue?" questions - auto-answered without sub-agent
# These are questions where the agent is asking if it should keep working
CONTINUE_QUESTION_PATTERNS = [
    r"(?i)\bwould you like me to continue\b",
    r"(?i)\bshould I continue\b",
    r"(?i)\bdo you want me to continue\b",
    r"(?i)\bshall I continue\b",
    r"(?i)\bshould I proceed\b",
    r"(?i)\bdo you want me to proceed\b",
    r"(?i)\bwould you like me to proceed\b",
    r"(?i)\bshall I proceed\b",
    r"(?i)\bdo you want me to keep going\b",
    r"(?i)\bshould I keep going\b",
    r"(?i)\bdo you want me to do the rest\b",
    r"(?i)\bshould I do the rest\b",
    r"(?i)\bwant me to continue\b",
    r"(?i)\bwant me to proceed\b",
    r"(?i)\bwant me to keep\b",
]


def cleanup_session_file() -> None:
    """Delete the session file on successful exit."""
    try:
        if SESSION_FILE.exists():
            SESSION_FILE.unlink()
    except OSError:
        pass  # Ignore errors during cleanup


def eprint(*args: object) -> None:
    """Print to stderr."""
    print(*args, file=sys.stderr)


def get_last_assistant_output(transcript_path: str) -> str:
    """Read the last assistant message from the transcript file."""
    import json as json_module

    try:
        transcript = Path(transcript_path)
        if not transcript.exists():
            return ""

        last_output = ""
        with transcript.open() as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    entry: dict[str, Any] = json_module.loads(line)
                    # Look for assistant messages
                    if entry.get("type") == "assistant":
                        message = cast(dict[str, Any], entry.get("message", {}))
                        content = cast(list[Any], message.get("content", []))
                        # Get text from content blocks
                        for block in content:
                            if isinstance(block, dict):
                                block_dict = cast(dict[str, Any], block)
                                if block_dict.get("type") == "text":
                                    text = block_dict.get("text", "")
                                    if isinstance(text, str):
                                        last_output = text
                            elif isinstance(block, str):
                                last_output = block
                except json_module.JSONDecodeError:
                    continue
        return last_output
    except Exception:
        return ""


def get_last_user_message_time(transcript_path: str) -> datetime | None:
    """Get the timestamp of the last user message from the transcript."""
    import json as json_module

    try:
        transcript = Path(transcript_path)
        if not transcript.exists():
            return None

        last_user_time: datetime | None = None
        with transcript.open() as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    entry: dict[str, Any] = json_module.loads(line)
                    if entry.get("type") == "user":
                        timestamp_str = entry.get("timestamp", "")
                        if timestamp_str:
                            # Parse ISO format timestamp
                            ts = timestamp_str.replace("Z", "+00:00")
                            last_user_time = datetime.fromisoformat(ts)
                except (json_module.JSONDecodeError, ValueError):
                    continue
        return last_user_time
    except Exception:
        return None


def is_user_recently_active(transcript_path: str) -> bool:
    """Check if the user sent a message within the recency window."""
    last_user_time = get_last_user_message_time(transcript_path)
    if last_user_time is None:
        return False

    now = datetime.now(tz=UTC)
    cutoff = now - timedelta(minutes=USER_RECENCY_MINUTES)
    return last_user_time >= cutoff


def looks_like_question(text: str) -> bool:
    """Check if the text appears to be asking the user a question."""
    return any(re.search(pattern, text) for pattern in QUESTION_PATTERNS)


def is_continue_question(text: str) -> bool:
    """Check if the text is asking whether to continue/proceed.

    These questions can be auto-answered without consulting a sub-agent.
    """
    return any(re.search(pattern, text) for pattern in CONTINUE_QUESTION_PATTERNS)


def run_subagent_decision(
    assistant_output: str, transcript_path: str
) -> tuple[bool, str | None]:
    """Run a sub-agent to decide whether to allow stop for user interaction.

    Uses headless Claude Code to make the decision, which doesn't consume
    extra API tokens since it uses the same session infrastructure.

    Returns (allow_stop, response) where:
    - allow_stop: True if we should allow the stop for user to respond
    - response: Optional response text if sub-agent wants to answer directly
    """
    # Extract the last ~2000 chars of assistant output for context
    if len(assistant_output) > 2000:
        context = assistant_output[-2000:]
    else:
        context = assistant_output

    # Fast path: Auto-answer "should I continue?" questions without sub-agent
    # This avoids the latency and potential timeout of calling the sub-agent
    if is_continue_question(context):
        eprint("# Fast path: Auto-answering continue question")
        eprint()
        return False, "Yes, please continue."

    prompt = f"""You are a sub-agent helping to manage an autonomous session.

The main agent has stopped and its last output appears to contain a question.
The user has been active recently (within the last {USER_RECENCY_MINUTES} minutes).

Your task is to decide:
1. Should we allow the main agent to stop so the user can respond to the question?
2. Or can you answer the question directly based on the context?

Here is the end of the main agent's last output:

<assistant_output>
{context}
</assistant_output>

Respond with EXACTLY one of these formats:

ALLOW_STOP: <brief reason why user should respond>

or

ANSWER: <your direct answer to the question>

or

CONTINUE: <reason why this doesn't seem like a real question for the user>

Choose ALLOW_STOP if:
- The question requires user preference or decision
- The question asks for clarification about requirements
- The question offers options the user should choose from

Choose ANSWER if:
- You can provide a reasonable default answer
- The question is about process/approach and you can decide
- IMPORTANT: If the question is "Do you want me to continue?", "Should I
  proceed?", "Do you want me to do the rest?", or any variation asking
  whether to keep working, ALWAYS answer "Yes, please continue."

Choose CONTINUE if:
- This doesn't look like a real question for the user
- It's a rhetorical question
- The agent should keep working without user input"""

    try:
        # Use headless Claude Code to make the decision
        result = subprocess.run(
            ["claude", "--print", "--model", "haiku", "-p", prompt],
            capture_output=True,
            text=True,
            timeout=60,  # 1 minute timeout (increased from 30s)
        )

        if result.returncode != 0:
            return False, None

        response_text = result.stdout.strip()

        if response_text.startswith("ALLOW_STOP:"):
            reason = response_text[len("ALLOW_STOP:"):].strip()
            eprint("# Sub-agent: Allowing stop for user response")
            eprint(f"Reason: {reason}")
            eprint()
            return True, None

        if response_text.startswith("ANSWER:"):
            answer = response_text[len("ANSWER:"):].strip()
            eprint("# Sub-agent: Providing answer")
            eprint()
            return False, answer

        # Default: CONTINUE or unrecognized format
        return False, None

    except (subprocess.TimeoutExpired, FileNotFoundError) as e:
        eprint(f"# Sub-agent error: {e}")
        return False, None


def check_interactive_question(transcript_path: str) -> tuple[bool, str | None]:
    """Check if the agent is asking a question and user is active.

    Returns (allow_stop, sub_agent_response) where:
    - allow_stop: True if stop should be allowed for user interaction
    - sub_agent_response: Optional response from sub-agent if it decided to answer
    """
    if not transcript_path:
        return False, None

    # Get last assistant output
    assistant_output = get_last_assistant_output(transcript_path)
    if not assistant_output:
        return False, None

    # Check if it looks like a question
    if not looks_like_question(assistant_output):
        return False, None

    # Check if user is recently active
    if not is_user_recently_active(transcript_path):
        return False, None

    # User is active and agent is asking a question - run sub-agent
    return run_subagent_decision(assistant_output, transcript_path)


def get_open_issues_count() -> int:
    """Get the count of open beads issues."""
    import json as json_module

    exit_code, output = run_command(["bd", "list", "--status=open", "--format=json"])
    if exit_code != 0:
        return 0

    try:
        issues = json_module.loads(output)
        if isinstance(issues, list):
            return len(cast(list[Any], issues))
    except (json_module.JSONDecodeError, TypeError):
        pass

    # Fallback: count lines from bd ready
    exit_code, output = run_command(["bd", "ready"])
    if exit_code == 0 and output.strip():
        # Count lines that look like issue entries (start with number and bracket)
        count = 0
        for line in output.strip().split("\n"):
            line = line.strip()
            if line and line[0].isdigit() and "[" in line:
                count += 1
        return count
    return 0


def check_for_bypass(transcript_path: str) -> tuple[bool, str | None]:
    """Check if Claude's last output contains the bypass string.

    Returns (allowed, rejection_reason) where allowed is True if bypass is
    permitted, and rejection_reason explains why bypass was rejected.
    """
    try:
        if not transcript_path:
            return False, None

        # Get Claude's last output from the transcript
        last_output = get_last_assistant_output(transcript_path)

        if HUMAN_INPUT_REQUIRED in last_output:
            # Check if there are remaining issues
            open_count = get_open_issues_count()
            if open_count > 0:
                return False, f"There are {open_count} open issue(s) remaining"
            return True, None

    except Exception:
        pass
    return False, None


def run_command(cmd: list[str]) -> tuple[int, str]:
    """Run a command and return (exit_code, output)."""
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        return result.returncode, (result.stdout or "") + (result.stderr or "")
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return 1, ""


def parse_session_file() -> dict[str, Any] | None:
    """Parse the autonomous session configuration file."""
    if not SESSION_FILE.exists():
        return None

    content = SESSION_FILE.read_text()
    if not content.startswith("---"):
        return None

    # Extract YAML frontmatter
    parts = content.split("---", 2)
    if len(parts) < 3:
        return None

    try:
        result: object = yaml.safe_load(parts[1])
        if isinstance(result, dict):
            return cast(dict[str, Any], result)
        return None
    except Exception:
        # Fallback to basic parsing
        config: dict[str, Any] = {}
        for line in parts[1].strip().split("\n"):
            if ":" in line:
                key, value_str = line.split(":", 1)
                key = key.strip()
                value_str = value_str.strip()
                value: Any = int(value_str) if value_str.isdigit() else value_str
                config[key] = value
        return config


def update_session_file(config: dict[str, Any]) -> None:
    """Update the session file with new config."""
    content = f"""---
{yaml.dump(config, default_flow_style=False)}---

# Autonomous Session Log

This file tracks the autonomous development session.
"""
    SESSION_FILE.write_text(content)


def get_issue_ids(output: str) -> set[str]:
    """Extract issue IDs from bd output."""
    ids: set[str] = set()
    for line in output.splitlines():
        # Look for issue ID patterns (e.g., "project-123")
        parts = line.split()
        for part in parts:
            if "-" in part and any(c.isdigit() for c in part):
                # Likely an issue ID
                ids.add(part.split()[0] if " " in part else part)
                break
    return ids


def get_current_issues() -> tuple[set[str], set[str]]:
    """Get current open and in-progress issues."""
    _, open_output = run_command(["bd", "list", "--status=open"])
    _, in_progress_output = run_command(["bd", "list", "--status=in_progress"])

    open_ids = get_issue_ids(open_output)
    in_progress_ids = get_issue_ids(in_progress_output)

    return open_ids, in_progress_ids


def run_quality_check() -> tuple[bool, str]:
    """Run the quality check script.

    Returns (passed, output) where passed is True if checks passed,
    and output contains the command output (useful for debugging failures).
    """
    script = Path(".claude/scripts/quality-check.sh")
    if not script.exists():
        # Try justfile
        exit_code, output = run_command(["just", "check"])
        return exit_code == 0, output

    exit_code, output = run_command(["bash", str(script)])
    return exit_code == 0, output


def check_github_build_status() -> tuple[bool, str | None]:
    """Check GitHub Actions build status for the current branch.

    Returns (should_block, message) where should_block is True if builds
    failed or are still running and we should wait.
    """
    import json

    # Get current branch
    exit_code, branch = run_command(["git", "rev-parse", "--abbrev-ref", "HEAD"])
    if exit_code != 0 or not branch.strip():
        return False, None  # Can't determine branch, don't block

    branch = branch.strip()

    # Check if gh is available and authenticated
    exit_code, _ = run_command(["gh", "auth", "status"])
    if exit_code != 0:
        # gh not authenticated, skip this check silently
        return False, None

    # Get workflow runs for current branch
    exit_code, output = run_command([
        "gh", "run", "list",
        "--branch", branch,
        "--limit", "5",
        "--json", "status,conclusion,name,headSha",
    ])

    if exit_code != 0:
        return False, None  # API error, don't block

    try:
        runs: list[dict[str, str]] = json.loads(output)
    except (json.JSONDecodeError, ValueError):
        return False, None

    if not runs:
        return False, None  # No runs found

    # Get current commit to check if runs are for our code
    exit_code, current_sha = run_command(["git", "rev-parse", "HEAD"])
    current_sha = current_sha.strip() if exit_code == 0 else ""

    # Check for in-progress or failed runs
    in_progress: list[str] = []
    failed: list[str] = []

    for run in runs:
        status = run.get("status", "")
        conclusion = run.get("conclusion", "")
        name = run.get("name", "unknown")
        run_sha = run.get("headSha", "")

        # Only consider runs for the current commit
        if current_sha and run_sha and not run_sha.startswith(current_sha[:7]):
            continue

        if status in ("in_progress", "queued", "waiting", "pending"):
            in_progress.append(name)
        elif conclusion in ("failure", "cancelled", "timed_out"):
            failed.append(name)

    if failed:
        msg = f"GitHub Actions failed: {', '.join(failed)}"
        return True, msg

    if in_progress:
        msg = f"GitHub Actions running: {', '.join(in_progress)}"
        return True, msg

    return False, None


def check_uncommitted_changes() -> tuple[bool, str]:
    """Check for uncommitted git changes.

    Returns (has_changes, description) where has_changes is True if there
    are uncommitted changes that should block exit.
    """
    # Check for unstaged changes
    exit_code, diff_output = run_command(["git", "diff", "--stat"])
    if exit_code != 0:
        # Not a git repo or git error, allow exit
        return False, ""

    # Check for staged changes
    _, staged_output = run_command(["git", "diff", "--cached", "--stat"])

    # Check for untracked files (but not ignored ones)
    untracked_cmd = ["git", "ls-files", "--others", "--exclude-standard"]
    _, untracked_output = run_command(untracked_cmd)

    changes: list[str] = []
    if diff_output.strip():
        changes.append("unstaged changes")
    if staged_output.strip():
        changes.append("staged changes")
    if untracked_output.strip():
        changes.append("untracked files")

    if changes:
        return True, ", ".join(changes)
    return False, ""


def check_beads_interaction() -> BeadsInteractionStatus:
    """Check if agent has interacted with beads during this session."""
    already_warned = BEADS_WARNING_MARKER.exists()

    # Check for uncommitted beads changes (staged or unstaged)
    _, beads_diff = run_command(["git", "diff", "--name-only", "--", ".beads/"])
    _, beads_staged = run_command(
        ["git", "diff", "--cached", "--name-only", "--", ".beads/"]
    )

    if beads_diff.strip() or beads_staged.strip():
        # Beads was modified - clear any warning marker
        if BEADS_WARNING_MARKER.exists():
            BEADS_WARNING_MARKER.unlink()
        return BeadsInteractionStatus(has_interaction=True, already_warned=already_warned)

    # Check recent commits for beads changes (last 10 commits)
    _, recent_beads = run_command(
        ["git", "log", "--oneline", "-10", "--name-only", "--", ".beads/"]
    )
    if recent_beads.strip():
        # Recent beads activity found - clear any warning marker
        if BEADS_WARNING_MARKER.exists():
            BEADS_WARNING_MARKER.unlink()
        return BeadsInteractionStatus(has_interaction=True, already_warned=already_warned)

    return BeadsInteractionStatus(has_interaction=False, already_warned=already_warned)


def get_diff_added_lines() -> list[tuple[str, int, str]]:
    """Get added lines from git diff.

    Returns list of (filename, line_number, line_content) tuples.
    """
    # Get unified diff with line numbers
    _, diff = run_command(["git", "diff", "-U0"])
    _, staged_diff = run_command(["git", "diff", "--cached", "-U0"])

    added_lines: list[tuple[str, int, str]] = []
    current_file = ""
    line_num = 0

    for diff_content in [diff, staged_diff]:
        for line in diff_content.splitlines():
            if line.startswith("diff --git"):
                # Extract filename from diff header
                parts = line.split()
                if len(parts) >= 4:
                    current_file = parts[3][2:]  # Remove 'b/' prefix
            elif line.startswith("@@"):
                # Parse hunk header: @@ -old,count +new,count @@
                match = re.search(r"\+([0-9]+)", line)
                if match:
                    line_num = int(match.group(1))
            elif line.startswith("+") and not line.startswith("+++"):
                # This is an added line
                added_lines.append((current_file, line_num, line[1:]))
                line_num += 1
            elif not line.startswith("-"):
                line_num += 1

    return added_lines


def check_error_suppression() -> list[str]:
    """Check for error suppression patterns in added lines.

    Returns list of warning messages if suppression patterns found.
    Only flags patterns that appear in actual end-of-line comments.
    """
    # Regex patterns for detecting error suppression directives
    suppression_regexes = [
        # Python suppression directives
        (re.compile(r"#\s*type:\s*ignore"), "type: ignore comment"),
        (re.compile(r"#\s*noqa"), "noqa comment"),
        (re.compile(r"#\s*pylint:\s*disable"), "pylint disable comment"),
        # TypeScript/JavaScript suppression directives
        (re.compile(r"//\s*@ts-ignore"), "TypeScript @ts-ignore comment"),
        (re.compile(r"//\s*@ts-expect-error"), "TypeScript @ts-expect-error comment"),
        # ESLint suppression directives
        (re.compile(r"(/\*|//)\s*eslint-disable"), "ESLint disable comment"),
    ]

    added_lines = get_diff_added_lines()
    violations: list[str] = []

    for filename, line_num, content in added_lines:
        # Skip lines that are inside string literals (basic heuristic)
        # If line has more quotes before # than after, # is likely in string
        for regex, description in suppression_regexes:
            if regex.search(content):
                violations.append(f"  {filename}:{line_num}: {description}")
                break  # One violation per line is enough

    return violations


def check_empty_except() -> list[str]:
    """Check for empty or pass-only except blocks in added lines.

    Returns list of warning messages if empty except blocks found.
    """
    added_lines = get_diff_added_lines()
    violations: list[str] = []

    for filename, line_num, content in added_lines:
        stripped = content.strip()
        # Check for bare except or except with just pass/...
        if "except" in stripped:
            # Look for patterns like "except: pass" or "except Exception: ..."
            if stripped.endswith(": pass") or stripped.endswith(": ..."):
                violations.append(f"  {filename}:{line_num}: empty except block")
            elif stripped == "except:" or stripped == "except Exception:":
                # Check next line would need context, flag as suspicious
                violations.append(f"  {filename}:{line_num}: bare except clause")

    return violations


def check_todo_without_issue() -> list[str]:
    """Check for work item markers in comments without issue references.

    Returns list of warning messages (not blocking).
    Only flags markers in actual comments (after comment start characters).
    """
    # Regex to find work markers in comments without issue references
    todo_in_comment = re.compile(
        r"(#|//)\s*(TODO|FIXME|HACK|XXX)\b(?!.*#[a-zA-Z]+-[a-zA-Z0-9]+)",
        re.IGNORECASE,
    )

    added_lines = get_diff_added_lines()
    warnings: list[str] = []

    for filename, line_num, content in added_lines:
        match = todo_in_comment.search(content)
        if match:
            marker = match.group(2).upper()
            warnings.append(
                f"  {filename}:{line_num}: {marker} without issue reference"
            )

    return warnings


# Token patterns that indicate hardcoded secrets
# Patterns are split/constructed to avoid self-detection
SECRET_PATTERNS = [
    # GitHub tokens (installation, personal, oauth, user, refresh)
    (re.compile(r"gh[spo]_[A-Za-z0-9]{36,}"), "GitHub token"),
    (re.compile(r"ghu_[A-Za-z0-9]{36,}"), "GitHub user token"),
    (re.compile(r"ghr_[A-Za-z0-9]{36,}"), "GitHub refresh token"),
    (re.compile(r"github_pat_[A-Za-z0-9_]{22,}"), "GitHub fine-grained PAT"),
    # AWS credentials
    (re.compile(r"AKIA[0-9A-Z]{16}"), "AWS access key"),
]


def check_hardcoded_secrets() -> list[str]:
    """Check for hardcoded secrets/tokens in added lines.

    Returns list of violation messages if secrets found.
    This is a BLOCKING check - secrets should never be committed.
    """
    added_lines = get_diff_added_lines()
    violations: list[str] = []

    for filename, line_num, content in added_lines:
        # Skip files in .credentials directory
        if ".credentials" in filename:
            continue

        for regex, description in SECRET_PATTERNS:
            if regex.search(content):
                violations.append(f"  {filename}:{line_num}: {description}")
                break  # One violation per line

    return violations


# Large file size threshold (500KB - reasonable for source code)
LARGE_FILE_THRESHOLD_BYTES = 500 * 1024


def check_large_files() -> list[str]:
    """Check for large files being committed.

    Returns list of warning messages for files over the threshold.
    """
    warnings: list[str] = []

    # Check staged files
    _, staged = run_command(["git", "diff", "--cached", "--name-only"])

    # Check untracked files
    _, untracked = run_command(
        ["git", "ls-files", "--others", "--exclude-standard"]
    )

    files_to_check: list[str] = []
    if staged.strip():
        files_to_check.extend(staged.strip().split("\n"))
    if untracked.strip():
        files_to_check.extend(untracked.strip().split("\n"))

    for filepath in files_to_check:
        if not filepath:
            continue
        try:
            path = Path(filepath)
            if path.exists() and path.is_file():
                size = path.stat().st_size
                if size > LARGE_FILE_THRESHOLD_BYTES:
                    size_kb = size / 1024
                    size_mb = size / (1024 * 1024)
                    if size_mb >= 1:
                        size_str = f"{size_mb:.1f}MB"
                    else:
                        size_str = f"{size_kb:.0f}KB"
                    warnings.append(f"  {filepath}: {size_str}")
        except (OSError, PermissionError):
            pass

    return warnings


def main() -> int:
    # Bypass the stop hook during repo critique
    if os.environ.get("CLAUDE_REPO_CRITIQUE"):
        return 0

    # Get transcript path from stdin (hook input)
    transcript_path = ""
    try:
        import json as json_module

        if not sys.stdin.isatty():
            stdin_content = sys.stdin.read()
            if stdin_content.strip():
                hook_input = json_module.loads(stdin_content)
                transcript_path = hook_input.get("transcript_path", "")
    except Exception:
        pass

    # Fast path: if no autonomous session and no git changes, allow immediate exit
    # This must come before any subagent calls to avoid slowness
    if not SESSION_FILE.exists():
        has_changes, _ = check_uncommitted_changes()
        if not has_changes:
            return 0  # Nothing to check, allow immediate exit

    # Check for bypass string in Claude's last output
    bypass_allowed, rejection_reason = check_for_bypass(transcript_path)
    if bypass_allowed:
        eprint("Human input required acknowledged. Allowing stop.")
        cleanup_session_file()
        return 0
    elif rejection_reason:
        # Bypass phrase was used but rejected
        eprint("# Exit Phrase Rejected")
        eprint()
        eprint(f"{rejection_reason}.")
        eprint()
        eprint("Please work on the remaining issues before exiting.")
        eprint("Run `bd ready` to see available work.")
        eprint()
        return 2

    # Always check for uncommitted changes (even outside autonomous mode)
    has_changes, change_desc = check_uncommitted_changes()

    if has_changes:
        # Check if agent has engaged with beads
        beads_status = check_beads_interaction()
        if not beads_status.has_interaction and not beads_status.already_warned:
            # First time warning about missing beads interaction
            BEADS_WARNING_MARKER.touch()
            eprint("# Beads Interaction Required")
            eprint()
            eprint("You have uncommitted changes but haven't interacted with beads.")
            eprint()
            eprint("In Claude Code sessions, work should be tracked in beads:")
            eprint("  - Create an issue: `bd create \"Issue title\"`")
            eprint("  - Claim work: `bd update <id> --status in_progress`")
            eprint("  - Complete work: `bd close <id>`")
            eprint()
            eprint("If this work genuinely doesn't need tracking, try stopping again.")
            eprint()
            return 2  # Block first attempt

        # Check for code quality issues in the diff
        suppression_violations = check_error_suppression()
        empty_except_violations = check_empty_except()
        secret_violations = check_hardcoded_secrets()
        todo_warnings = check_todo_without_issue()
        large_file_warnings = check_large_files()

        # Run quality checks to ensure they pass
        eprint("# Running Quality Checks...")
        eprint()
        quality_passed, quality_output = run_quality_check()
        eprint()

        eprint("# Uncommitted Changes Detected")
        eprint()
        eprint(f"Cannot exit with {change_desc}.")
        eprint()

        if not quality_passed:
            eprint("## Quality Checks Failed")
            eprint()
            eprint("Quality gates did not pass. Fix issues before committing.")
            eprint()
            if quality_output.strip():
                eprint("### Output:")
                eprint()
                # Show last 50 lines of output to avoid overwhelming
                lines = quality_output.strip().split("\n")
                if len(lines) > 50:
                    eprint(f"... (showing last 50 of {len(lines)} lines)")
                    lines = lines[-50:]
                for line in lines:
                    eprint(f"  {line}")
                eprint()

        # Show blocking violations first
        if suppression_violations:
            eprint("## Error Suppression Detected")
            eprint()
            eprint("The following error suppressions were added:")
            eprint()
            for v in suppression_violations:
                eprint(v)
            eprint()
            eprint("Fix the underlying issues instead of suppressing errors.")
            eprint()

        if empty_except_violations:
            eprint("## Empty Exception Handlers Detected")
            eprint()
            eprint("The following empty except blocks were added:")
            eprint()
            for v in empty_except_violations:
                eprint(v)
            eprint()
            eprint("Handle exceptions properly or re-raise them.")
            eprint()

        if secret_violations:
            eprint("## SECURITY: Hardcoded Secrets Detected")
            eprint()
            eprint("The following secrets/tokens were found in staged changes:")
            eprint()
            for v in secret_violations:
                eprint(v)
            eprint()
            eprint("NEVER commit secrets. Use environment variables instead.")
            eprint("If this was accidental, the secret may need to be rotated.")
            eprint()

        # Show non-blocking warnings
        if todo_warnings:
            eprint("## Untracked Work Items")
            eprint()
            eprint("Consider linking these items to beads issues:")
            eprint()
            for w in todo_warnings:
                eprint(w)
            eprint()
            eprint("Format: # ITEM(#project-abc): description".replace("ITEM", "TODO"))
            eprint()

        if large_file_warnings:
            eprint("## Large Files Detected")
            eprint()
            eprint("The following files exceed 500KB:")
            eprint()
            for w in large_file_warnings:
                eprint(w)
            eprint()
            eprint("Consider using Git LFS or adding to .gitignore if appropriate.")
            eprint()

        # Check for untracked files - everything should be tracked or gitignored
        untracked_cmd = ["git", "ls-files", "--others", "--exclude-standard"]
        _, untracked = run_command(untracked_cmd)
        if untracked.strip():
            untracked_files = untracked.strip().split("\n")
            eprint("## Untracked Files")
            eprint()
            eprint("The following files are not tracked by git:")
            eprint()
            for f in untracked_files[:10]:  # Limit to first 10
                eprint(f"  {f}")
            if len(untracked_files) > 10:
                eprint(f"  ... and {len(untracked_files) - 10} more")
            eprint()
            eprint("Either `git add` them or add them to .gitignore")
            eprint()

        eprint("Before stopping, please:")
        eprint()
        eprint("1. Run `git status` to check for files that should be gitignored")
        eprint("2. Run `just check` to verify quality gates pass")
        eprint("3. For substantial implementations, run `/goal-verify`")
        eprint("4. Stage your changes: `git add <files>`")
        eprint("5. Commit with a descriptive message: `git commit -m '...'`")
        eprint("6. Push to remote: `git push`")
        eprint()
        eprint("Work is incomplete until `git push` succeeds.")
        eprint()
        return 2  # Block exit

    # Check GitHub build status (runs after all other checks pass)
    # Skip if build was already failing at session start
    if BUILD_FAILING_MARKER.exists():
        build_should_block = False
        build_message = None
    else:
        build_should_block, build_message = check_github_build_status()
    if build_should_block and build_message:
        eprint("# GitHub Actions Check")
        eprint()
        eprint(build_message)
        eprint()
        if "running" in build_message.lower():
            eprint("Waiting for builds to complete...")
            eprint("Run `gh run list` to check status.")
        else:
            eprint("Fix the failing builds before stopping.")
            eprint("Run `gh run view` to see details.")
        eprint()
        return 2  # Block exit

    # Check if agent is asking a question and user is recently active
    # This allows interactive conversations to proceed naturally
    allow_for_question, sub_agent_response = check_interactive_question(transcript_path)
    if allow_for_question:
        eprint("# Allowing stop for user interaction")
        eprint()
        eprint("The agent appears to be asking a question and you were active.")
        eprint("Please respond to continue the conversation.")
        eprint()
        return 0  # Allow stop
    if sub_agent_response:
        # Sub-agent decided to answer the question directly
        eprint("# Sub-agent Response")
        eprint()
        eprint(sub_agent_response)
        eprint()
        eprint("---")
        eprint("Continuing autonomous work...")
        eprint()
        # Don't return - continue with autonomous mode

    # Check if autonomous mode is active
    config = parse_session_file()
    if config is None:
        # Not in autonomous mode - run quality checks before allowing exit
        # This ensures coverage and other quality gates pass
        eprint("# Pre-exit Quality Check")
        eprint()
        passed, output = run_quality_check()
        if not passed:
            eprint()
            eprint("## Quality Gates Failed")
            eprint()
            eprint("Quality checks must pass before exiting.")
            eprint()
            if output.strip():
                eprint("### Output:")
                eprint()
                # Show last 50 lines of output to avoid overwhelming
                lines = output.strip().split("\n")
                if len(lines) > 50:
                    eprint(f"... (showing last 50 of {len(lines)} lines)")
                    lines = lines[-50:]
                for line in lines:
                    eprint(f"  {line}")
                eprint()
            eprint("Fix the issues shown above before exiting.")
            eprint()
            return 2  # Block exit
        eprint()
        eprint("Quality gates passed. Allowing exit.")
        return 0

    iteration: int = int(config.get("iteration", 0)) + 1
    last_change_iteration: int = int(config.get("last_issue_change_iteration", 0))
    snapshot_list: list[str] = config.get("issue_snapshot", [])
    previous_snapshot: set[str] = set(snapshot_list)

    # Get current issue state
    open_ids, in_progress_ids = get_current_issues()
    current_snapshot = open_ids | in_progress_ids
    total_outstanding = len(current_snapshot)

    # Check if issues changed
    if current_snapshot != previous_snapshot:
        last_change_iteration = iteration
        eprint(f"Issue state changed at iteration {iteration}")

    # Update session file
    config["iteration"] = iteration
    config["last_issue_change_iteration"] = last_change_iteration
    config["issue_snapshot"] = list(current_snapshot)
    update_session_file(config)

    # Check staleness
    iterations_since_change = iteration - last_change_iteration
    if iterations_since_change >= STALENESS_THRESHOLD:
        eprint("# Staleness Detected")
        eprint()
        eprint(f"No issue changes for {iterations_since_change} iterations.")
        eprint("Autonomous mode is stopping due to lack of progress.")
        eprint()
        eprint("This could mean:")
        eprint("- The remaining work requires human decisions")
        eprint("- There's a blocker that needs manual intervention")
        eprint("- The loop is stuck in an unproductive pattern")
        eprint()
        eprint("Run `/autonomous-mode` to start a new session with fresh goals.")
        cleanup_session_file()
        return 0  # Allow exit

    # Check if all work is done
    if total_outstanding == 0:
        eprint("# Checking Completion")
        eprint()
        eprint("No outstanding issues. Running quality gates...")
        eprint()

        passed, output = run_quality_check()
        if passed:
            eprint("All quality gates passed!")
            eprint("No open issues remain.")
            eprint()
            eprint("## Options")
            eprint()
            eprint("1. Run `/ideate` to generate new work items")
            eprint("2. Say the exit phrase to end the session")
            eprint()
            eprint("To exit, include this exact phrase in your response:")
            eprint()
            eprint(f'  "{HUMAN_INPUT_REQUIRED}"')
            eprint()
            return 2  # Block exit, require explicit choice
        else:
            eprint("## Quality Gates Failed")
            if output.strip():
                eprint("### Output:")
                lines = output.strip().split("\n")
                if len(lines) > 50:
                    eprint(f"... (showing last 50 of {len(lines)} lines)")
                    lines = lines[-50:]
                for line in lines:
                    eprint(f"  {line}")
            eprint()
            eprint("Fix issues before completing.")
            return 2  # Block exit

    # Work remains
    eprint("# Autonomous Mode Active")
    eprint()
    eprint(f"**Iteration {iteration}** | Outstanding issues: {total_outstanding}")
    eprint(f"Iterations since last issue change: {iterations_since_change}")
    eprint()
    eprint("## Current State")
    eprint(f"- Open issues: {len(open_ids)}")
    eprint(f"- In progress: {len(in_progress_ids)}")
    eprint()
    eprint("## Action Required")
    eprint()
    eprint("Continue working on outstanding issues:")
    eprint()
    eprint("1. Run `bd ready` to see available work")
    eprint("2. Pick an issue and work on it")
    eprint("3. Run quality checks after completing work")
    eprint("4. For substantial work, run `/goal-verify`")
    eprint("5. Close completed issues with `bd close <id>`")
    eprint()

    if iterations_since_change > 2:
        eprint(f"**Warning**: No issue changes for {iterations_since_change} loops.")
        eprint(f"Staleness threshold: {STALENESS_THRESHOLD}")
        eprint()

    eprint("---")
    eprint()
    eprint("If you cannot proceed without human input, include this exact string:")
    eprint()
    eprint(f'  "{HUMAN_INPUT_REQUIRED}"')
    eprint()

    return 2  # Block exit


if __name__ == "__main__":
    sys.exit(main())
