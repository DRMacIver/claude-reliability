#!/usr/bin/env python3
"""Fake Claude binary for snapshot testing.

This script is invoked when hooks call the Claude binary (e.g., for RealSubAgent).
It reads expected input/output from a transcript and returns the expected output.

Environment variables:
    FAKE_CLAUDE_TRANSCRIPT: Path to the transcript.jsonl file
    FAKE_CLAUDE_STATE: Path to a JSON file for tracking call state
    FAKE_CLAUDE_CWD: Working directory (defaults to current)

Usage:
    python fake_claude.py --print -p "prompt"
    python fake_claude.py [other claude args]
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

from snapshot_tests.placeholder import PlaceholderRegistry
from snapshot_tests.transcript import TranscriptEntry, parse_transcript


def load_state(state_path: Path) -> dict:
    """Load state from the state file."""
    if state_path.exists():
        return json.loads(state_path.read_text())
    return {"call_index": 0, "placeholders": {}}


def save_state(state_path: Path, state: dict) -> None:
    """Save state to the state file."""
    state_path.write_text(json.dumps(state, indent=2))


def find_subagent_calls(transcript: list[TranscriptEntry]) -> list[dict]:
    """Find all sub-agent calls in the transcript.

    These are identified by looking for Task tool uses that spawn
    claude processes.
    """
    subagent_calls = []

    for entry in transcript:
        for tool_use in entry.tool_uses:
            if tool_use.name == "Task":
                # This is a sub-agent call
                subagent_calls.append(
                    {
                        "id": tool_use.id,
                        "input": tool_use.input,
                        "entry": entry,
                    }
                )

    return subagent_calls


def find_subagent_result(transcript: list[TranscriptEntry], tool_use_id: str) -> str | None:
    """Find the result for a sub-agent call."""
    for entry in transcript:
        for result in entry.tool_results:
            if result.tool_use_id == tool_use_id:
                return result.content
    return None


def main():
    """Main entry point for fake claude."""
    parser = argparse.ArgumentParser(description="Fake Claude for testing")
    parser.add_argument("--print", "-P", action="store_true", help="Print mode")
    parser.add_argument("-p", dest="prompt", help="Prompt text")
    parser.add_argument("--output-format", default="text", help="Output format")
    parser.add_argument("--model", help="Model to use")
    parser.add_argument("--dangerously-skip-permissions", action="store_true")

    args, unknown = parser.parse_known_args()

    # Get environment configuration
    transcript_path = os.environ.get("FAKE_CLAUDE_TRANSCRIPT")
    state_path = os.environ.get("FAKE_CLAUDE_STATE")
    cwd = os.environ.get("FAKE_CLAUDE_CWD", os.getcwd())

    if not transcript_path:
        print("Error: FAKE_CLAUDE_TRANSCRIPT not set", file=sys.stderr)
        sys.exit(1)

    if not state_path:
        print("Error: FAKE_CLAUDE_STATE not set", file=sys.stderr)
        sys.exit(1)

    transcript_path = Path(transcript_path)
    state_path = Path(state_path)

    if not transcript_path.exists():
        print(f"Error: Transcript not found: {transcript_path}", file=sys.stderr)
        sys.exit(1)

    # Load transcript and state
    transcript = parse_transcript(transcript_path)
    state = load_state(state_path)

    # Initialize placeholder registry from state
    registry = PlaceholderRegistry()
    registry.values = state.get("placeholders", {})

    # Find sub-agent calls
    subagent_calls = find_subagent_calls(transcript)
    call_index = state.get("call_index", 0)

    if call_index >= len(subagent_calls):
        print("Error: No more sub-agent calls expected", file=sys.stderr)
        sys.exit(1)

    expected_call = subagent_calls[call_index]
    expected_prompt = expected_call["input"].get("prompt", "")

    # Verify prompt matches (with placeholder substitution)
    actual_prompt = args.prompt or ""

    if not registry.match(expected_prompt, actual_prompt):
        print(f"Error: Prompt mismatch", file=sys.stderr)
        print(f"Expected: {expected_prompt}", file=sys.stderr)
        print(f"Actual: {actual_prompt}", file=sys.stderr)
        sys.exit(1)

    # Get expected result
    result = find_subagent_result(transcript, expected_call["id"])
    if result is None:
        print("Warning: No result found for sub-agent call", file=sys.stderr)
        result = ""

    # Update state
    state["call_index"] = call_index + 1
    state["placeholders"] = registry.values
    save_state(state_path, state)

    # Output the expected result
    print(result)


if __name__ == "__main__":
    main()
