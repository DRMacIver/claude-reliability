# Claude Code Transcript Format

This document describes the JSONL transcript format used by Claude Code, based on analysis of recorded sessions.

## File Format

Transcripts are stored as JSON Lines (`.jsonl`) files where each line is a valid JSON object representing a message or event in the conversation.

## Message Structure

### Common Fields

All messages share these base fields:

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Message type: `"queue-operation"`, `"user"`, `"assistant"` |
| `uuid` | string | Unique identifier for this message |
| `parentUuid` | string \| null | UUID of parent message (for threading) |
| `timestamp` | string | ISO 8601 timestamp |
| `sessionId` | string | Session identifier |
| `cwd` | string | Current working directory |
| `version` | string | Claude Code version (e.g., `"2.1.12"`) |
| `gitBranch` | string | Git branch name (empty if not in repo) |

### Queue Operations

First message in a transcript, indicates session dequeue:

```json
{
  "type": "queue-operation",
  "operation": "dequeue",
  "timestamp": "2026-01-20T11:57:17.716Z",
  "sessionId": "95e6f011-ec33-4b37-91ad-5d242b40ae3d"
}
```

### User Messages

User input and tool results:

```json
{
  "type": "user",
  "userType": "external",
  "isSidechain": false,
  "message": {
    "role": "user",
    "content": "Plain text message or tool_result array"
  }
}
```

#### Tool Results

When a user message contains tool results:

```json
{
  "message": {
    "role": "user",
    "content": [
      {
        "tool_use_id": "toolu_01NzjV452AGVQR51StZtarfB",
        "type": "tool_result",
        "content": "     1→\"\"\"Math utility functions.\"\"\"\n     2→..."
      }
    ]
  },
  "toolUseResult": {
    "type": "text",
    "file": {
      "filePath": "/tmp/tmpamdu7ye8/src/math_utils.py",
      "content": "actual file content...",
      "numLines": 7,
      "startLine": 1,
      "totalLines": 7
    }
  }
}
```

### Assistant Messages

Model responses with tool calls or text:

```json
{
  "type": "assistant",
  "requestId": "req_011CXJb8F3Bb3xDrKBeYga4T",
  "message": {
    "model": "claude-opus-4-5-20251101",
    "id": "msg_01GvmQZAHi5NTJwtuYDNkbLY",
    "type": "message",
    "role": "assistant",
    "content": [
      {
        "type": "tool_use",
        "id": "toolu_01NzjV452AGVQR51StZtarfB",
        "name": "Read",
        "input": {
          "file_path": "/tmp/tmpamdu7ye8/src/math_utils.py"
        }
      }
    ],
    "stop_reason": null,
    "usage": {
      "input_tokens": 1,
      "cache_creation_input_tokens": 187,
      "cache_read_input_tokens": 19024,
      "output_tokens": 25
    }
  }
}
```

## Tool-Specific Formats

### Read Tool Result

Content format uses arrow separator (NOT tab):
- 6-character right-justified line number
- Arrow character (`→`, U+2192)
- Line content

```
     1→first line content
     2→second line content
```

The `toolUseResult` object contains structured data:
```json
{
  "type": "text",
  "file": {
    "filePath": "/path/to/file",
    "content": "raw file content",
    "numLines": 7,
    "startLine": 1,
    "totalLines": 7
  }
}
```

### Write Tool Result

```json
{
  "toolUseResult": {
    "type": "create",
    "filePath": "/path/to/file",
    "content": "file content",
    "structuredPatch": [],
    "originalFile": null
  }
}
```

For updates (file existed):
```json
{
  "toolUseResult": {
    "type": "update",
    "filePath": "/path/to/file",
    "content": "new content",
    "originalFile": "old content"
  }
}
```

### Edit Tool Result

```json
{
  "toolUseResult": {
    "filePath": "/path/to/file",
    "oldString": "text to replace",
    "newString": "replacement text",
    "originalFile": "full original content",
    "structuredPatch": [
      {
        "oldStart": 4,
        "oldLines": 3,
        "newStart": 4,
        "newLines": 8,
        "lines": [
          " unchanged line",
          "-removed line",
          "+added line"
        ]
      }
    ],
    "userModified": false,
    "replaceAll": false
  }
}
```

### Bash Tool Result

```json
{
  "toolUseResult": {
    "stdout": "command output",
    "stderr": "",
    "interrupted": false,
    "isImage": false
  }
}
```

### Glob Tool Result

```json
{
  "toolUseResult": {
    "filenames": ["/path/to/file1.py", "/path/to/file2.py"],
    "durationMs": 275,
    "numFiles": 2,
    "truncated": false
  }
}
```

## Message Threading

Messages are threaded via `parentUuid`:
- User message has `parentUuid` pointing to previous assistant message
- Assistant message has `parentUuid` pointing to previous user message
- First user message has `parentUuid: null`

When multiple tool calls are made in parallel:
- Single assistant message contains multiple `tool_use` blocks
- Multiple user messages follow, each with same `parentUuid` pointing to the assistant
- Each user message has `sourceToolAssistantUUID` pointing to the specific tool call

## Meta Messages

Stop hook feedback and other system events use `isMeta: true`:

```json
{
  "type": "user",
  "isMeta": true,
  "message": {
    "role": "user",
    "content": "Stop hook feedback:\n[hook/path]: message"
  }
}
```

## Session Metadata

Additional session metadata fields:
- `slug`: Human-readable session identifier (e.g., `"soft-inventing-zebra"`)
- `isSidechain`: Boolean, true for parallel/background operations
- `userType`: `"external"` for user input, may differ for system messages

## Version Notes

Format documented here is from Claude Code version 2.1.12. The format may evolve in future versions.
