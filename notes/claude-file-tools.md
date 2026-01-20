# Claude Code File Tools: Implementation Specification

This document captures the behavioral specification of Claude Code's Read, Write, and Edit tools for implementing compatible test fixtures. Primary sources are the [Piebald-AI/claude-code-system-prompts](https://github.com/Piebald-AI/claude-code-system-prompts) repository and [bgauryy's internal tools analysis](https://gist.github.com/bgauryy/0cdb9aa337d01ae5bd0c803943aa36bd).

## Tool Schemas

### Read

```json
{
  "name": "Read",
  "parameters": {
    "file_path": { "type": "string", "required": true },
    "offset": { "type": "number", "required": false },
    "limit": { "type": "number", "required": false }
  }
}
```

### Write

```json
{
  "name": "Write", 
  "parameters": {
    "file_path": { "type": "string", "required": true },
    "content": { "type": "string", "required": true }
  }
}
```

### Edit

```json
{
  "name": "Edit",
  "parameters": {
    "file_path": { "type": "string", "required": true },
    "old_string": { "type": "string", "required": true },
    "new_string": { "type": "string", "required": true },
    "replace_all": { "type": "boolean", "required": false, "default": false }
  }
}
```

## Read Tool Behavior

### Output Format

Output uses `cat -n` style line numbering. Format per line:

```
{spaces}{line_number}→{content}
```

Where:
- `{spaces}` — right-justifies line number to a fixed width (observed: 6 characters total for number + spaces)
- `{line_number}` — 1-indexed
- `→` — Unicode arrow character (U+2192 RIGHTWARDS ARROW), **NOT a tab character** (earlier docs incorrectly stated tab)
- `{content}` — line content, truncated at **2,000 characters**

Example output for a 3-line file:
```
     1→first line
     2→second line
     3→third line
```

### Default Limits

- **Default limit**: 2,000 lines from start of file
- **Default offset**: 1 (first line)
- **Max line length**: 2,000 characters (truncated, not wrapped)

### State Tracking

The system maintains a set of files that have been read in the current session. This is checked by Write and Edit before allowing modifications.

**Important**: Reading a file adds it to this set. The set persists for the session but the file content is not cached—subsequent reads fetch current disk content.

### Special File Handling

| File Type | Behavior |
|-----------|----------|
| Images (PNG, JPG, JPEG, GIF, WEBP) | Returned as visual content to the model |
| PDF | Processed page-by-page, text + visual extraction |
| Jupyter notebooks (.ipynb) | Returns all cells with outputs (but dedicated `NotebookRead` tool exists) |
| Empty files | Returns empty string + system warning about empty file |
| Binary files | Unclear — likely returns raw bytes or errors |

### Error Conditions

| Condition | Error |
|-----------|-------|
| File does not exist | File not found error (exact message TBD) |
| Path is relative | Rejected — absolute paths required |
| Permission denied | OS-level permission error |

## Write Tool Behavior

### Semantics

- Creates file if it doesn't exist
- **Completely overwrites** if file exists
- Creates parent directories as needed (unconfirmed — needs testing)

### State Requirements

For **existing files**: must have been read in current session, otherwise fails with:
```
File has not been read yet. Read it first before writing to it.
```

For **new files**: no read requirement.

### Error Conditions

| Condition | Error |
|-----------|-------|
| Existing file not previously read | "File has not been read yet" |
| Path is relative | Rejected |
| File modified since last read | "File has been unexpectedly modified" |

## Edit Tool Behavior

### Matching Semantics

The `old_string` parameter uses **exact byte-for-byte matching**:

- NOT regex
- NOT fuzzy matching
- NOT whitespace-normalized
- Case-sensitive
- Whitespace-sensitive (tabs vs spaces matter)
- Unicode-sensitive (visually identical characters with different codepoints will NOT match)

### Uniqueness Constraint

When `replace_all: false` (the default):

| Match Count | Result |
|-------------|--------|
| 0 matches | Error: string not found |
| 1 match | Success: replacement made |
| 2+ matches | Error: ambiguous match |

When `replace_all: true`:

| Match Count | Result |
|-------------|--------|
| 0 matches | Error: string not found |
| 1+ matches | Success: all occurrences replaced |

### State Requirements

Same as Write — file must have been read in current session:
```
File has not been read yet. Read it first before editing it.
```

### Modification Detection

The system tracks file modification time. If the file has been modified since the last read (by any process), the edit fails:
```
File has been unexpectedly modified. Read it again before attempting to edit it.
```

**Known bug on Windows** ([GitHub #12891](https://github.com/anthropics/claude-code/issues/12891)): Windows updates Last Access Time on read by default, which can trigger false positives. The check appears to use `mtime` but Windows behavior conflates access and modification times unless `fsutil behavior set disablelastaccess 1` is run.

### Error Conditions

| Condition | Error Message |
|-----------|---------------|
| File not read yet | "File has not been read yet" |
| File modified since read | "File has been unexpectedly modified" |
| `old_string` not found | (needs exact message — likely "No match found" or similar) |
| Multiple matches, `replace_all: false` | (needs exact message — likely "Multiple matches found" or similar) |
| `old_string == new_string` | Likely rejected — schema says "must differ" |
| Path is relative | Rejected |

### Edge Cases From Bug Reports

**Unicode normalization** ([GitHub #1986](https://github.com/anthropics/claude-code/issues/1986)): Straight quotes (`"`) vs curly quotes (`"`) are different bytes. The model may hallucinate one when the file contains the other, causing "No changes to make" errors even when the text looks identical.

**Tab handling** ([GitHub #13152](https://github.com/anthropics/claude-code/issues/13152)): Files indented with tabs are problematic. The model sees the `cat -n` output which has a tab delimiter, and may confuse content tabs with the line-number delimiter tab.

**Partial replacement** ([GitHub #1657](https://github.com/anthropics/claude-code/issues/1657)): Reported cases where `replace_all: true` only replaced the first occurrence despite multiple matches in the file. Root cause unclear.

## Path Resolution

All three tools require **absolute paths**:

- ✅ `/Users/dev/project/src/index.ts`
- ✅ `C:/Users/dev/project/src/index.ts`
- ❌ `./src/index.ts`
- ❌ `src/index.ts`
- ❌ `~/project/src/index.ts` (unclear if ~ expansion happens)

## Session State Model

```
SessionState {
  read_files: Set<AbsolutePath>
  file_mtimes: Map<AbsolutePath, Timestamp>  // recorded at read time
}

on Read(path):
  content = read_file(path)
  state.read_files.add(path)
  state.file_mtimes[path] = get_mtime(path)
  return format_with_line_numbers(content)

on Write(path, content):
  if file_exists(path):
    if path not in state.read_files:
      error("File has not been read yet")
    if get_mtime(path) != state.file_mtimes[path]:
      error("File has been unexpectedly modified")
  write_file(path, content)
  state.file_mtimes[path] = get_mtime(path)

on Edit(path, old_string, new_string, replace_all):
  if path not in state.read_files:
    error("File has not been read yet")
  if get_mtime(path) != state.file_mtimes[path]:
    error("File has been unexpectedly modified")
  
  content = read_file(path)
  matches = find_all(content, old_string)  // exact byte match
  
  if len(matches) == 0:
    error("No match found")  // exact message TBD
  if len(matches) > 1 and not replace_all:
    error("Multiple matches found")  // exact message TBD
  
  new_content = replace(content, old_string, new_string, replace_all)
  write_file(path, new_content)
  state.file_mtimes[path] = get_mtime(path)
```

## Open Questions for Testing

1. **Exact error messages**: The precise wording of error messages is not fully documented. You may need to trigger each error condition against real Claude Code to capture exact strings.

2. **Line number formatting width**: Is it always 6 characters? Does it adapt to file length (e.g., 7 chars for files with 1M+ lines)?

3. **Binary file handling**: What happens when Read is called on a binary file?

4. **Directory creation**: Does Write create parent directories, or fail if they don't exist?

5. **Symlink handling**: Does Read follow symlinks? What path gets recorded in `read_files`?

6. **Encoding**: Is file content assumed UTF-8? What happens with other encodings?

7. **Line ending normalization**: Does Read normalize `\r\n` to `\n`? Does Write preserve or normalize line endings?

8. **Empty `old_string`**: What happens if `old_string` is empty string in Edit?

9. **Tilde expansion**: Is `~/file.txt` expanded or rejected as non-absolute?

10. **Trailing newline**: Does Write ensure files end with newline, or write content exactly as provided?

## Sources

- [Piebald-AI/claude-code-system-prompts](https://github.com/Piebald-AI/claude-code-system-prompts) — extracted system prompts, updated per release
- [bgauryy's gist](https://gist.github.com/bgauryy/0cdb9aa337d01ae5bd0c803943aa36bd) — internal tool implementation analysis  
- [wong2's gist](https://gist.github.com/wong2/e0f34aac66caf890a332f7b6f9e2ba8f) — earlier system prompt extraction
- [GitHub #12891](https://github.com/anthropics/claude-code/issues/12891) — Windows mtime bug
- [GitHub #1986](https://github.com/anthropics/claude-code/issues/1986) — Unicode quote handling
- [GitHub #13152](https://github.com/anthropics/claude-code/issues/13152) — Tab handling issues
- [GitHub #1657](https://github.com/anthropics/claude-code/issues/1657) — Partial replace_all bug
- [mariozechner.at cchistory](https://mariozechner.at/posts/2025-08-03-cchistory/) — tracks prompt changes across versions