#!/usr/bin/env python3
"""Compile JSONL transcript to human-readable Markdown.

Converts Claude Code's native transcript.jsonl format into a readable
Markdown document for documentation and review purposes.

Usage:
    uv run compile-transcript.py transcript.jsonl
    uv run compile-transcript.py transcript.jsonl -o transcript.md
    uv run compile-transcript.py transcript.jsonl --verbose
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from snapshot_tests.transcript import TranscriptEntry, parse_transcript, get_conversation_turns


def format_tool_use(tool_use) -> str:
    """Format a tool use block as Markdown."""
    lines = [f"### Tool: {tool_use.name}"]

    # Format input based on tool type
    if tool_use.name == "Bash":
        command = tool_use.input.get("command", "")
        lines.append("```bash")
        lines.append(command)
        lines.append("```")
    elif tool_use.name in ("Write", "Edit"):
        file_path = tool_use.input.get("file_path", "")
        lines.append(f"**File:** `{file_path}`")
        if tool_use.name == "Edit":
            old = tool_use.input.get("old_string", "")[:100]
            new = tool_use.input.get("new_string", "")[:100]
            lines.append(f"**Old:** `{old}...`")
            lines.append(f"**New:** `{new}...`")
        else:
            content = tool_use.input.get("content", "")
            if len(content) > 500:
                content = content[:500] + "\n... (truncated)"
            lines.append("```")
            lines.append(content)
            lines.append("```")
    elif tool_use.name == "Read":
        file_path = tool_use.input.get("file_path", "")
        lines.append(f"**File:** `{file_path}`")
    elif tool_use.name == "Glob":
        pattern = tool_use.input.get("pattern", "")
        lines.append(f"**Pattern:** `{pattern}`")
    elif tool_use.name == "Grep":
        pattern = tool_use.input.get("pattern", "")
        path = tool_use.input.get("path", ".")
        lines.append(f"**Pattern:** `{pattern}` in `{path}`")
    elif tool_use.name == "Task":
        prompt = tool_use.input.get("prompt", "")[:200]
        subagent = tool_use.input.get("subagent_type", "")
        lines.append(f"**Subagent:** {subagent}")
        lines.append(f"**Prompt:** {prompt}...")
    else:
        # Generic formatting
        lines.append("```json")
        lines.append(json.dumps(tool_use.input, indent=2)[:500])
        lines.append("```")

    return "\n".join(lines)


def format_tool_result(result: str) -> str:
    """Format a tool result as Markdown."""
    lines = ["**Result:**"]

    # Truncate long results
    if len(result) > 1000:
        result = result[:1000] + "\n... (truncated)"

    lines.append("```")
    lines.append(result)
    lines.append("```")

    return "\n".join(lines)


def compile_transcript(transcript: list[TranscriptEntry], verbose: bool = False) -> str:
    """Compile a parsed transcript to Markdown.

    Args:
        transcript: List of parsed transcript entries
        verbose: Include thinking blocks and extra details

    Returns:
        Markdown formatted string
    """
    lines = ["# Transcript", ""]

    turns = get_conversation_turns(transcript)

    for turn_num, (user_entry, assistant_entries) in enumerate(turns, start=1):
        # User section
        lines.append(f"## Turn {turn_num}")
        lines.append("")
        lines.append("### User")
        lines.append("")

        # Get user message content
        user_text = user_entry.text_content
        if user_text:
            lines.append(user_text)
            lines.append("")

        # Assistant section
        lines.append("### Assistant")
        lines.append("")

        for entry in assistant_entries:
            # Text content
            text = entry.text_content
            if text:
                lines.append(text)
                lines.append("")

            # Thinking blocks (if verbose)
            if verbose:
                for block in entry.content:
                    if block.type == "thinking":
                        thinking = block.data.get("thinking", "")[:500]
                        if thinking:
                            lines.append("<details>")
                            lines.append("<summary>Thinking</summary>")
                            lines.append("")
                            lines.append(thinking)
                            lines.append("")
                            lines.append("</details>")
                            lines.append("")

            # Tool uses
            for tool_use in entry.tool_uses:
                lines.append(format_tool_use(tool_use))
                lines.append("")

        # Tool results (from subsequent user entries)
        for entry in assistant_entries:
            # Check if this entry has tool results in the next user message
            pass

        # Find tool results that match tool uses
        tool_use_ids = set()
        for entry in assistant_entries:
            for tu in entry.tool_uses:
                tool_use_ids.add(tu.id)

        # Look for results in the transcript
        for entry in transcript:
            for result in entry.tool_results:
                if result.tool_use_id in tool_use_ids:
                    lines.append(format_tool_result(result.content))
                    lines.append("")

        lines.append("---")
        lines.append("")

    return "\n".join(lines)


def main():
    """Main entry point."""
    parser = argparse.ArgumentParser(
        description="Compile JSONL transcript to Markdown"
    )
    parser.add_argument("input", type=Path, help="Input transcript.jsonl file")
    parser.add_argument(
        "-o", "--output", type=Path, help="Output file (default: input with .md extension)"
    )
    parser.add_argument(
        "--verbose", "-v", action="store_true", help="Include thinking blocks"
    )

    args = parser.parse_args()

    if not args.input.exists():
        print(f"Error: Input file not found: {args.input}", file=sys.stderr)
        sys.exit(1)

    # Default output path
    output_path = args.output or args.input.with_suffix(".md")

    # Parse and compile
    transcript = parse_transcript(args.input)
    markdown = compile_transcript(transcript, verbose=args.verbose)

    # Write output
    output_path.write_text(markdown)
    print(f"Compiled {len(transcript)} entries to {output_path}")


if __name__ == "__main__":
    main()
