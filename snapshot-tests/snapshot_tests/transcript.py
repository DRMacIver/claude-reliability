"""Transcript parsing for Claude Code JSONL transcripts.

Parses the native JSONL format from Claude Code sessions and extracts
structured data for replay testing.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class ToolUse:
    """Represents a tool use from Claude."""

    id: str
    name: str
    input: dict[str, Any]


@dataclass
class ToolResult:
    """Represents a tool result from execution."""

    tool_use_id: str
    content: str
    is_error: bool = False


@dataclass
class ContentBlock:
    """A content block within a message."""

    type: str
    data: dict[str, Any]

    @property
    def text(self) -> str | None:
        """Get text content if this is a text block."""
        if self.type == "text":
            return self.data.get("text")
        return None

    @property
    def tool_use(self) -> ToolUse | None:
        """Get tool use if this is a tool_use block."""
        if self.type == "tool_use":
            return ToolUse(
                id=self.data.get("id", ""),
                name=self.data.get("name", ""),
                input=self.data.get("input", {}),
            )
        return None

    @property
    def tool_result(self) -> ToolResult | None:
        """Get tool result if this is a tool_result block."""
        if self.type == "tool_result":
            return ToolResult(
                tool_use_id=self.data.get("tool_use_id", ""),
                content=self.data.get("content", ""),
                is_error=self.data.get("is_error", False),
            )
        return None


@dataclass
class TranscriptEntry:
    """A single entry in a Claude Code transcript.

    Entry types:
    - "user": User message
    - "assistant": Assistant response (may contain text, tool_use, thinking)
    - "system": System message
    - "file-history-snapshot": File backup metadata
    - "summary": Conversation summary
    - "progress": Progress indicator
    - "hook_progress": Hook execution progress
    """

    type: str
    uuid: str
    timestamp: str
    parent_uuid: str | None
    content: list[ContentBlock]
    raw: dict[str, Any] = field(repr=False)

    @property
    def role(self) -> str | None:
        """Get the role if this is a message entry."""
        message = self.raw.get("message", {})
        return message.get("role")

    @property
    def text_content(self) -> str:
        """Get concatenated text content from all text blocks."""
        texts = []
        for block in self.content:
            if block.type == "text" and block.text:
                texts.append(block.text)
        return "\n".join(texts)

    @property
    def tool_uses(self) -> list[ToolUse]:
        """Get all tool uses from this entry."""
        uses = []
        for block in self.content:
            if tu := block.tool_use:
                uses.append(tu)
        return uses

    @property
    def tool_results(self) -> list[ToolResult]:
        """Get all tool results from this entry."""
        results = []
        for block in self.content:
            if tr := block.tool_result:
                results.append(tr)
        return results


def parse_content_blocks(content: Any) -> list[ContentBlock]:
    """Parse content into ContentBlock list.

    Content can be:
    - A string (converted to single text block)
    - A list of content block dicts
    """
    if isinstance(content, str):
        return [ContentBlock(type="text", data={"text": content})]

    if not isinstance(content, list):
        return []

    blocks = []
    for item in content:
        if isinstance(item, dict):
            block_type = item.get("type", "unknown")
            blocks.append(ContentBlock(type=block_type, data=item))
        elif isinstance(item, str):
            blocks.append(ContentBlock(type="text", data={"text": item}))

    return blocks


def parse_entry(data: dict[str, Any]) -> TranscriptEntry | None:
    """Parse a single JSONL line into a TranscriptEntry.

    Returns None for entries that aren't useful for replay testing.
    """
    entry_type = data.get("type")
    if not entry_type:
        return None

    # Skip file history and progress entries
    skip_types = {"file-history-snapshot", "progress", "hook_progress", "bash_progress"}
    if entry_type in skip_types:
        return None

    uuid = data.get("uuid", "")
    timestamp = data.get("timestamp", "")
    parent_uuid = data.get("parentUuid")

    # Extract content from message if present
    message = data.get("message", {})
    content = message.get("content", [])

    return TranscriptEntry(
        type=entry_type,
        uuid=uuid,
        timestamp=timestamp,
        parent_uuid=parent_uuid,
        content=parse_content_blocks(content),
        raw=data,
    )


def parse_transcript(path: Path) -> list[TranscriptEntry]:
    """Parse a JSONL transcript file into structured entries.

    Args:
        path: Path to .jsonl transcript file

    Returns:
        List of TranscriptEntry objects, filtered to useful entries
    """
    entries = []

    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue

            try:
                data = json.loads(line)
                if entry := parse_entry(data):
                    entries.append(entry)
            except json.JSONDecodeError:
                # Skip malformed lines
                continue

    return entries


def extract_tool_calls(transcript: list[TranscriptEntry]) -> list[tuple[ToolUse, ToolResult | None]]:
    """Extract tool use/result pairs from a transcript.

    Returns pairs of (ToolUse, ToolResult) where ToolResult may be None
    if the result wasn't captured.

    Args:
        transcript: Parsed transcript entries

    Returns:
        List of (tool_use, tool_result) tuples
    """
    # Build a map of tool_use_id -> ToolResult
    results_by_id: dict[str, ToolResult] = {}
    for entry in transcript:
        for result in entry.tool_results:
            results_by_id[result.tool_use_id] = result

    # Collect all tool uses with their results
    pairs: list[tuple[ToolUse, ToolResult | None]] = []
    for entry in transcript:
        for tool_use in entry.tool_uses:
            result = results_by_id.get(tool_use.id)
            pairs.append((tool_use, result))

    return pairs


def filter_by_role(transcript: list[TranscriptEntry], role: str) -> list[TranscriptEntry]:
    """Filter transcript entries by role (user, assistant, system)."""
    return [e for e in transcript if e.role == role]


def get_conversation_turns(transcript: list[TranscriptEntry]) -> list[tuple[TranscriptEntry, list[TranscriptEntry]]]:
    """Group transcript into conversation turns.

    Each turn is (user_entry, [assistant_entries]).

    Returns:
        List of (user_message, assistant_responses) tuples
    """
    turns: list[tuple[TranscriptEntry, list[TranscriptEntry]]] = []
    current_user: TranscriptEntry | None = None
    current_assistant: list[TranscriptEntry] = []

    for entry in transcript:
        if entry.role == "user" and entry.type == "user":
            # Check if this is a tool result (still part of assistant turn)
            if entry.tool_results:
                current_assistant.append(entry)
                continue

            # New user turn
            if current_user is not None:
                turns.append((current_user, current_assistant))
            current_user = entry
            current_assistant = []
        elif entry.role == "assistant":
            current_assistant.append(entry)

    # Add final turn
    if current_user is not None:
        turns.append((current_user, current_assistant))

    return turns
