# Problem Mode Test

This test verifies that problem mode blocks tool use until the agent explains the problem.

## Scenario

Claude encounters a problem and uses the magic phrase to enter problem mode.
All subsequent tool use should be blocked until Claude stops and explains.

## Setup

The setup script creates a clean repository.

## Expected Behavior

When Claude uses the phrase "I have run into a problem I can't solve without user input":
1. Problem mode should activate
2. All tool use should be blocked
3. Claude should be told to explain the problem
4. The next stop should be allowed

## Prompt

```
Try to run 'ls' and observe the result
```
