#!/usr/bin/env python3
"""PreCompact hook to identify untracked issues in the conversation.

This hook runs before context compaction and uses headless Claude Code
to analyze the conversation transcript, identifying issues that should
be tracked in beads but aren't.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def eprint(*args: object) -> None:
    """Print to stderr."""
    print(*args, file=sys.stderr)


def run_command(cmd: list[str], timeout: int = 60) -> tuple[int, str]:
    """Run a command and return (returncode, output)."""
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return result.returncode, result.stdout + result.stderr
    except subprocess.TimeoutExpired:
        return 1, "Command timed out"
    except Exception as e:
        return 1, str(e)


def get_existing_issues() -> set[str]:
    """Get titles of existing open issues in beads."""
    code, output = run_command(["bd", "list", "--status=open"], timeout=30)
    if code != 0:
        return set()

    titles: set[str] = set()
    for line in output.splitlines():
        # Parse lines like "○ project-123 [P2] [bug] - Issue title here"
        if " - " in line:
            title = line.split(" - ", 1)[1].strip().lower()
            titles.add(title)
    return titles


def extract_transcript_summary(transcript_path: str, max_chars: int = 50000) -> str:
    """Extract a summary of the conversation transcript.

    Focuses on user messages and assistant responses to identify
    issues, TODOs, and problems discussed.
    """
    try:
        path = Path(transcript_path)
        if not path.exists():
            return ""

        content = path.read_text()
        lines = content.strip().split("\n")

        summary_parts: list[str] = []
        total_chars = 0

        for line in lines:
            if total_chars >= max_chars:
                break
            try:
                msg = json.loads(line)
                role = msg.get("role", "")

                # Extract text content
                if role == "user":
                    text = msg.get("content", "")
                    if isinstance(text, str) and text.strip():
                        chunk = f"USER: {text[:2000]}"
                        summary_parts.append(chunk)
                        total_chars += len(chunk)
                elif role == "assistant":
                    content_list = msg.get("content", [])
                    if isinstance(content_list, list):
                        for item in content_list:
                            if isinstance(item, dict) and item.get("type") == "text":
                                text = item.get("text", "")
                                if text.strip():
                                    chunk = f"ASSISTANT: {text[:2000]}"
                                    summary_parts.append(chunk)
                                    total_chars += len(chunk)
                                    break  # Only first text block
            except json.JSONDecodeError:
                continue

        return "\n\n".join(summary_parts)
    except Exception as e:
        eprint(f"Error reading transcript: {e}")
        return ""


def analyze_with_claude(
    transcript_summary: str, existing_issues: set[str]
) -> list[dict[str, str]]:
    """Use headless Claude to analyze transcript and identify untracked issues."""
    if not transcript_summary:
        return []

    existing_list = (
        "\n".join(f"- {title}" for title in sorted(existing_issues)) or "(none)"
    )

    prompt = f"""Analyze this conversation transcript and identify any issues, \
bugs, feature requests, or TODOs that were discussed but may not be tracked.

EXISTING ISSUES (already tracked, do not duplicate):
{existing_list}

CONVERSATION TRANSCRIPT:
{transcript_summary}

Instructions:
1. Look for: bugs, feature requests, TODOs discussed, problems to fix later
2. Do NOT include issues already being tracked (check EXISTING ISSUES)
3. Do NOT include things that were completed in this session
4. Only include things that need future work

For each untracked issue found, output EXACTLY this format (one per line):
ISSUE|<type>|<priority>|<title>

Where:
- type is one of: bug, feature, task
- priority is 0-4 (0=critical, 2=medium, 4=backlog)
- title is a brief description (max 80 chars)

If no untracked issues are found, output only: NONE

Example output:
ISSUE|bug|2|Fix memory leak in database connection pool
ISSUE|feature|3|Add support for dark mode
NONE"""

    # Call headless Claude with the prompt
    code, output = run_command(
        [
            "claude",
            "-p", prompt,
            "--dangerously-skip-permissions",
            "--max-turns", "1",
        ],
        timeout=120,
    )

    if code != 0:
        eprint(f"Claude analysis failed: {output[:500]}")
        return []

    # Parse the output
    issues: list[dict[str, str]] = []
    for line in output.splitlines():
        line = line.strip()
        if line == "NONE":
            break
        if line.startswith("ISSUE|"):
            parts = line.split("|", 3)
            if len(parts) == 4:
                _, issue_type, priority, title = parts
                if issue_type in ("bug", "feature", "task") and priority.isdigit():
                    issues.append({
                        "type": issue_type,
                        "priority": priority,
                        "title": title.strip()[:80],
                    })

    return issues


def create_beads_issues(
    issues: list[dict[str, str]], existing_issues: set[str]
) -> int:
    """Create beads issues for untracked items."""
    created = 0
    for issue in issues:
        # Skip if title is too similar to existing
        title_lower = issue["title"].lower()
        if any(title_lower in existing or existing in title_lower
               for existing in existing_issues):
            continue

        cmd = [
            "bd", "create",
            f"--title={issue['title']}",
            f"--type={issue['type']}",
            f"--priority={issue['priority']}",
        ]
        code, output = run_command(cmd, timeout=30)
        if code == 0:
            created += 1
            eprint(f"Created issue: {issue['title']}")
        else:
            eprint(f"Failed to create issue: {output[:200]}")

    return created


def main() -> int:
    """Main entry point."""
    # Parse hook input from stdin
    try:
        hook_input = json.loads(sys.stdin.read())
    except json.JSONDecodeError:
        # No input or invalid JSON - exit silently
        return 0

    transcript_path = hook_input.get("transcript_path", "")
    if not transcript_path:
        # No transcript available - this is known to happen sometimes
        return 0

    # Check if transcript exists
    if not Path(transcript_path).exists():
        return 0

    eprint("Analyzing conversation for untracked issues...")

    # Get existing issues to avoid duplicates
    existing_issues = get_existing_issues()

    # Extract and summarize transcript
    transcript_summary = extract_transcript_summary(transcript_path)
    if not transcript_summary:
        return 0

    # Analyze with Claude
    issues = analyze_with_claude(transcript_summary, existing_issues)

    if not issues:
        eprint("No untracked issues found.")
        return 0

    # Create issues in beads
    created = create_beads_issues(issues, existing_issues)
    if created > 0:
        eprint(f"Created {created} new issue(s) in beads.")

    return 0


if __name__ == "__main__":
    sys.exit(main())
