# Basic Stop Hook Test

This test verifies that the stop hook blocks when there are uncommitted changes.

## Scenario

A file is modified but not committed. Claude should be blocked from stopping
until the changes are committed.

## Setup

The setup script creates a file, commits it, then modifies it.

## Expected Behavior

When Claude tries to stop, the stop hook should:
1. Detect uncommitted changes
2. Block the stop
3. Display a message about uncommitted changes

## Prompt

```
Write "hello" to test.txt
```
