"""Snapshot testing library for claude-reliability hooks."""

from .placeholder import PlaceholderRegistry
from .transcript import TranscriptEntry, ToolUse, ToolResult, parse_transcript, extract_tool_calls
from .simulator import ToolSimulator
from .plugin_setup import install_plugin, install_plugin_binary_only

__all__ = [
    "PlaceholderRegistry",
    "TranscriptEntry",
    "ToolUse",
    "ToolResult",
    "parse_transcript",
    "extract_tool_calls",
    "ToolSimulator",
    "install_plugin",
    "install_plugin_binary_only",
]
