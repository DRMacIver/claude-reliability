---
description: Enter autonomous development mode with iterative task completion
---

# Just Keep Working Mode

You are entering **just-keep-working mode**. In this mode, you work through tasks autonomously, checking in with the user only when genuinely needed.

## Getting Started

Before diving in, understand what you're working toward:

1. **Goal**: What should be accomplished in this session?
2. **Success Criteria**: How will you know when the work is done?
3. **Scope**: Any areas to avoid or boundaries to respect?

Write the session configuration to `.claude/jkw-session.local.md`.

## Work Loop

1. Check for tasks: `list_tasks(ready_only=true)`
2. Pick a task and start it: `work_on(task_id="...")`
3. Do the work - write code, run tests, fix issues
4. When the task is done: `update_task(id="...", status="complete")`
5. Repeat until no tasks remain

## The Right Mindset

This mode is about **getting things done thoroughly**. When you encounter something difficult:

- Work through it rather than stopping
- If you genuinely need user input, ask clearly
- Don't declare partial work as "complete"
- Don't add error suppression to avoid fixing issues

## When to Stop

- All tasks are complete (success!)
- You genuinely need user input on a decision
- You've hit a blocker you can't resolve alone

Use the appropriate exit phrase:
- Work complete: "I'm ready for human input"
- Need help: "I've hit a problem I need help with"

## Session Cleanup

Don't manually delete session files. The stop hook handles cleanup when the session ends normally. If you need to cancel, use `/cancel-just-keep-working`.
