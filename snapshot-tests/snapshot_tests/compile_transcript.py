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

from snapshot_tests.transcript import TranscriptEntry, ToolUse, ToolResult, parse_transcript


def format_tool_use(tool_use: ToolUse) -> list[str]:
    """Format a tool use block as Markdown lines."""
    lines = [f"**Tool: {tool_use.name}**"]

    # Format input based on tool type
    if tool_use.name == "Bash":
        command = tool_use.input.get("command", "")
        lines.append("```bash")
        lines.append(command)
        lines.append("```")
    elif tool_use.name == "Write":
        file_path = tool_use.input.get("file_path", "")
        content = tool_use.input.get("content", "")
        lines.append(f"File: `{file_path}`")
        if len(content) > 1000:
            content = content[:1000] + "\n... (truncated)"
        lines.append("```")
        lines.append(content)
        lines.append("```")
    elif tool_use.name == "Edit":
        file_path = tool_use.input.get("file_path", "")
        old = tool_use.input.get("old_string", "")
        new = tool_use.input.get("new_string", "")
        lines.append(f"File: `{file_path}`")
        if len(old) > 200:
            old = old[:200] + "... (truncated)"
        if len(new) > 200:
            new = new[:200] + "... (truncated)"
        lines.append(f"Old: `{old}`")
        lines.append(f"New: `{new}`")
    elif tool_use.name == "Read":
        file_path = tool_use.input.get("file_path", "")
        lines.append(f"File: `{file_path}`")
    elif tool_use.name == "Glob":
        pattern = tool_use.input.get("pattern", "")
        lines.append(f"Pattern: `{pattern}`")
    elif tool_use.name == "Grep":
        pattern = tool_use.input.get("pattern", "")
        path = tool_use.input.get("path", ".")
        lines.append(f"Pattern: `{pattern}` in `{path}`")
    elif tool_use.name == "Task":
        prompt = tool_use.input.get("prompt", "")
        subagent = tool_use.input.get("subagent_type", "")
        if len(prompt) > 300:
            prompt = prompt[:300] + "... (truncated)"
        lines.append(f"Subagent: {subagent}")
        lines.append(f"Prompt: {prompt}")
    else:
        # Generic formatting
        input_str = json.dumps(tool_use.input, indent=2)
        if len(input_str) > 500:
            input_str = input_str[:500] + "... (truncated)"
        lines.append("```json")
        lines.append(input_str)
        lines.append("```")

    return lines


def format_tool_result(result: ToolResult) -> list[str]:
    """Format a tool result as Markdown lines."""
    content = result.content
    if len(content) > 2000:
        content = content[:2000] + "\n... (truncated)"

    lines = ["**Output:**", "```", content, "```"]
    return lines


def compile_transcript(transcript: list[TranscriptEntry], verbose: bool = False) -> str:
    """Compile a parsed transcript to Markdown.

    Args:
        transcript: List of parsed transcript entries
        verbose: Include thinking blocks and extra details

    Returns:
        Markdown formatted string
    """
    lines = ["# Transcript", ""]

    # Find the project directory from the first user entry
    project_dir = None
    for entry in transcript:
        if entry.raw.get("cwd"):
            project_dir = entry.raw["cwd"]
            break

    def substitute_project_dir(text: str) -> str:
        """Replace the project directory with $CLAUDE_PROJECT_DIR."""
        if project_dir and project_dir in text:
            return text.replace(project_dir, "$CLAUDE_PROJECT_DIR")
        return text

    # Build a map of tool_use_id -> ToolResult for quick lookup
    results_by_id: dict[str, ToolResult] = {}
    for entry in transcript:
        for result in entry.tool_results:
            results_by_id[result.tool_use_id] = result

    # Process entries in order
    for entry in transcript:
        # User message (not a tool result)
        if entry.type == "user" and entry.role == "user" and not entry.tool_results:
            user_text = entry.text_content
            if user_text:
                lines.append(f"**User:** {substitute_project_dir(user_text)}")
                lines.append("")

        # Assistant message
        elif entry.type == "assistant" and entry.role == "assistant":
            # Text content
            text = entry.text_content
            if text:
                lines.append(f"**Assistant:** {substitute_project_dir(text)}")
                lines.append("")

            # Thinking blocks (if verbose)
            if verbose:
                for block in entry.content:
                    if block.type == "thinking":
                        thinking = block.data.get("thinking", "")
                        if thinking:
                            if len(thinking) > 1000:
                                thinking = thinking[:1000] + "... (truncated)"
                            lines.append("<details>")
                            lines.append("<summary>Thinking</summary>")
                            lines.append("")
                            lines.append(substitute_project_dir(thinking))
                            lines.append("")
                            lines.append("</details>")
                            lines.append("")

            # Tool uses with their results
            for tool_use in entry.tool_uses:
                tool_lines = format_tool_use(tool_use)
                lines.extend([substitute_project_dir(line) for line in tool_lines])
                lines.append("")

                # Add the result if we have it
                if tool_use.id in results_by_id:
                    result = results_by_id[tool_use.id]
                    result_lines = format_tool_result(result)
                    lines.extend([substitute_project_dir(line) for line in result_lines])
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
