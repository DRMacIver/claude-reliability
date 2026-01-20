# Stop Hook: Reflection First Stop Block

## Scenario

Tests that the stop hook prompts for reflection when:
- Agent has used modifying tools (Write/Edit/Delete)
- Agent tries to stop for the first time
- No reflection marker exists yet

The reflection prompt asks the agent to verify task completion before exiting.

## Expected Behavior

1. Agent reads test.txt
2. Agent modifies the file (Write/Edit)
3. Agent commits the change
4. Agent tries to stop
5. Stop hook blocks with "Task Completion Check" message
6. Reflection marker is set
7. Agent must stop again to actually exit

## Post-Condition

- File was modified
- Reflection marker was set (then cleared on second stop)

## Prompt

```
Read test.txt and change the content to "modified content". Commit the change, then stop.
```
