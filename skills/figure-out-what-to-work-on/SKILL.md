---
name: figure-out-what-to-work-on
description: This skill should be used when the user asks "what should I work on", "what's next", "pick a task", or when needing to select between multiple available tasks. Provides guidance for prioritizing and selecting tasks.
---

# Figuring Out What to Work On

## Quick Start

If you just need something to work on:
```
what_should_i_work_on
```

This picks a random task from the highest-priority unblocked tasks.

## Decision Framework

### 1. Check for Blockers First

Before picking new work, check if anything is blocked on you:
```
list_tasks(status="open", ready_only=true)
```

Look for tasks you previously started that might be waiting.

### 2. Priority Order

Work in priority order:
- **P0 (Critical)**: Stop everything else, fix this now
- **P1 (High)**: Should be done soon, before starting new features
- **P2 (Medium)**: Normal work, the bulk of tasks
- **P3 (Low)**: Nice to have, do when higher priorities clear
- **P4 (Backlog)**: Future work, not urgent

```
list_tasks(max_priority=1, ready_only=true)  # Show P0 and P1 only
```

### 3. Check for Questions

Tasks might be blocked on user questions:
```
get_question_blocked_tasks
```

If you can answer any of these yourself now (with context you've gained), do so with `answer_question`.

### 4. Consider Dependencies

Some tasks unblock others. Prioritize tasks that are blocking other work:
```
get_task(id="...") # Check what tasks this one blocks
```

## When User is Away

If the user hasn't been active:

1. **Avoid big decisions** - Stick to well-defined tasks
2. **Prefer cleanup** - Tests, docs, small fixes
3. **Don't start new features** - Unless explicitly requested
4. **Check the task list** - User may have left work queued

## When User is Active

If the user is present:

1. **Ask about priorities** - "Should I continue with X or switch to Y?"
2. **Report blockers** - Use `create_question` to document blockers
3. **Confirm big changes** - Before major refactoring, verify direction

## Workflow

1. `list_tasks(status="open", ready_only=true)` - See what's available
2. Pick highest priority unblocked task
3. `work_on(task_id="...")` - Mark as in-progress
4. `get_task(id="...")` - Read full description and notes
5. Do the work
6. `add_note(task_id="...", content="...")` - Record progress/findings
7. `update_task(id="...", status="complete")` - Mark done
8. Repeat

## When Stuck

If you're stuck on a task:

1. **Create a question**: `create_question(text="How should I handle X?")`
2. **Link it**: `link_task_to_question(task_id, question_id)`
3. **Move on**: The task is now blocked, pick another

Don't spin on problems that need user input.

## When There's No Work

If `list_tasks(ready_only=true)` shows nothing:

1. Check `get_question_blocked_tasks` - maybe you can answer some
2. Look at backlog: `list_tasks(priority=4)`
3. Ask the user if there's something they want done
4. Stop if genuinely nothing to do - it's okay to stop when work is done
