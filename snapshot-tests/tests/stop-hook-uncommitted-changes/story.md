# Stop Hook: Uncommitted Changes Block

## Scenario

Tests that the stop hook blocks exit when there are uncommitted changes in the git working directory.

The agent should be prompted to commit or discard changes before exiting.

## Expected Behavior

1. Agent reads src/main.py
2. Agent modifies the file
3. Agent tries to stop
4. Stop hook blocks with "Uncommitted Changes Detected" message
5. Agent commits the changes
6. Agent tries to stop again
7. Stop hook allows exit

## Post-Condition

- Changes are committed
- Git working directory is clean

## Prompt

```
Read src/main.py and add a docstring to the top. Then stop.
```
