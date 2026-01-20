# Stop Hook: Fast Path Clean Exit

## Scenario

Tests that the stop hook allows immediate exit when:
- No JKW session is active
- Git working directory is clean (no uncommitted changes)
- Not ahead of remote

This is the "fast path" through the stop hook - when there's nothing blocking, the agent can exit immediately.

## Expected Behavior

1. Agent reads a file (no modifications)
2. Agent tries to stop
3. Stop hook allows exit immediately (exit code 0)

## Post-Condition

- No marker files created in .claude/
- Exit was allowed

## Prompt

```
Read the README.md file and tell me what it says. Then stop.
```
