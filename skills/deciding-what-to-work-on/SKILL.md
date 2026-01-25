---
name: Deciding What to Work On
description: "This skill should be used when the user asks 'what should I work on', 'what's next', 'pick a task', or when needing to select between multiple available tasks. Provides guidance for prioritizing and selecting tasks."
---

# Deciding What to Work On

## Quick Start

If you just need something to work on:
```
what_should_i_work_on
```

This picks a random work item from the highest-priority unblocked items.

## Decision Framework

### 1. Check for Blockers First

Before picking new work, check if anything is blocked on you:
```
list_work_items(status="open", ready_only=true)
```

Look for items you previously started that might be waiting.

### 2. Priority Order

Work in priority order:
- **P0 (Critical)**: Stop everything else, fix this now
- **P1 (High)**: Should be done soon, before starting new features
- **P2 (Medium)**: Normal work, the bulk of items
- **P3 (Low)**: Nice to have, do when higher priorities clear
- **P4 (Backlog)**: Future work, not urgent

```
list_work_items(max_priority=1, ready_only=true)  # Show P0 and P1 only
```

### 3. Check for Questions

Items might be blocked on user questions:
```
get_question_blocked_work
```

If you can answer any of these yourself now (with context you've gained), do so with `answer_question`.

### 4. Consider Dependencies

Some items unblock others. Prioritize items that are blocking other work:
```
get_work_item(id="...") # Check what items this one blocks
```

## Work Autonomously

**You do not need user guidance on priorities.** The priority system (P0-P4) tells you what to work on. Use `what_should_i_work_on` and follow its suggestion.

**Large backlogs are normal.** Having many open tasks doesn't mean anything is wrong. Just work through them one at a time. Don't ask which subset to focus on - the priority system handles this.

**Report genuine blockers.** Use `create_question` only for things that actually block progress:
- Missing information needed to proceed
- Conflicting requirements that need clarification
- Decisions only the user can make

**Don't use emergency_stop for "too much work".** Emergency stop is for genuine blockers that prevent ANY progress. A large backlog is not a blocker.

## Workflow

1. `list_work_items(status="open", ready_only=true)` - See what's available
2. Pick highest priority unblocked item
3. `work_on(work_item_id="...")` - Mark as in-progress
4. `get_work_item(id="...")` - Read full description and notes
5. Do the work
6. `add_note(work_item_id="...", content="...")` - Record progress/findings
7. `update_work_item(id="...", status="complete")` - Mark done
8. Repeat

## When Stuck

If you're stuck on an item:

1. **Create a question**: `create_question(text="How should I handle X?")`
2. **Link it**: `link_work_to_question(work_item_id, question_id)`
3. **Move on**: The item is now blocked, pick another

Don't spin on problems that need user input.

## When There's No Work

If `list_work_items(ready_only=true)` shows nothing:

1. Check `get_question_blocked_work` - maybe you can answer some
2. Look at backlog: `list_work_items(priority=4)`
3. Ask the user if there's something they want done
4. Stop if genuinely nothing to do - it's okay to stop when work is done
