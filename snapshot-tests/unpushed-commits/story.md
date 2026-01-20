# Unpushed Commits Test

This test verifies that the stop hook blocks when there are unpushed commits.

## Scenario

A commit is made locally but not pushed to the remote. Claude should be
blocked from stopping until the commit is pushed.

## Setup

The setup script creates a local commit without pushing.

## Expected Behavior

When Claude tries to stop, the stop hook should:
1. Detect unpushed commits
2. Block the stop
3. Display a message about unpushed commits

## Prompt

```
Check if there are any uncommitted changes
```
