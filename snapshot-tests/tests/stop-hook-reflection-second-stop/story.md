# Stop Hook: Reflection Second Stop Allow

## Scenario

Tests that the stop hook allows exit when:
- The reflection marker already exists (first stop already prompted)
- Agent tries to stop again

This tests the "second stop" path where the agent has already been prompted for reflection and is now allowed to exit.

## Expected Behavior

1. Reflection marker exists from setup (simulating first stop already happened)
2. Agent tries to stop
3. Stop hook clears the marker and allows exit (exit code 0)

## Post-Condition

- Reflection marker is cleared
- Exit was allowed

## Prompt

```
I've reviewed my work and it's complete. Stop now.
```
