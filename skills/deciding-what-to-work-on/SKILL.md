---
name: Deciding What to Work On
description: "This skill should be used when the user asks 'what should I work on', 'what's next', 'pick a task', or when needing to select between multiple available tasks. Provides guidance for prioritizing and selecting tasks."
---

# Deciding What to Work On

## Quick Start

If you just need something to work on:
```bash
.claude-reliability/bin/claude-reliability work next
```

This picks a random work item from the highest-priority unblocked items.

## Decision Framework

### 1. Check for Blockers First

Before picking new work, check if anything is blocked on you:
```bash
.claude-reliability/bin/claude-reliability work list --status open --ready-only
```

Look for items you previously started that might be waiting.

### 2. Priority Order

Work in priority order:
- **P0 (Critical)**: Stop everything else, fix this now
- **P1 (High)**: Should be done soon, before starting new features
- **P2 (Medium)**: Normal work, the bulk of items
- **P3 (Low)**: Nice to have, do when higher priorities clear
- **P4 (Backlog)**: Future work, not urgent

```bash
.claude-reliability/bin/claude-reliability work list --max-priority 1 --ready-only  # Show P0 and P1 only
```

### 3. Check for Questions

Items might be blocked on user questions:
```bash
.claude-reliability/bin/claude-reliability work blocked
```

If you can answer any of these yourself now (with context you've gained), do so with `question answer`.

### 4. Consider Dependencies

Some items unblock others. Prioritize items that are blocking other work:
```bash
.claude-reliability/bin/claude-reliability work get <id>  # Check what items this one blocks
```

## Work Autonomously

**You do not need user guidance on priorities.** The priority system (P0-P4) tells you what to work on. Use `work next` and follow its suggestion.

**Large backlogs are normal.** Having many open tasks doesn't mean anything is wrong. Just work through them one at a time. Don't ask which subset to focus on - the priority system handles this.

**Report genuine blockers.** Use `question create` only for things that actually block progress:
- Missing information needed to proceed
- Conflicting requirements that need clarification
- Decisions only the user can make

**Don't use emergency-stop for "too much work".** Emergency stop is for genuine blockers that prevent ANY progress. A large backlog is not a blocker.

## Workflow

1. `.claude-reliability/bin/claude-reliability work list --status open --ready-only` - See what's available
2. Pick highest priority unblocked item
3. `.claude-reliability/bin/claude-reliability work on <id>` - Mark as in-progress
4. `.claude-reliability/bin/claude-reliability work get <id>` - Read full description and notes
5. Do the work
6. `.claude-reliability/bin/claude-reliability work add-note <id> -c "..."` - Record progress/findings
7. `.claude-reliability/bin/claude-reliability work update <id> --status complete` - Mark done
8. Repeat

## When Stuck

If you're stuck on an item:

1. **Create a question**: `.claude-reliability/bin/claude-reliability question create -t "How should I handle X?"`
2. **Link it**: `.claude-reliability/bin/claude-reliability question link <work-id> --question-id <q-id>`
3. **Move on**: The item is now blocked, pick another

Don't spin on problems that need user input.

## When There's No Work

If `work list --ready-only` shows nothing:

1. Check `work blocked` - maybe you can answer some questions
2. Look at backlog: `.claude-reliability/bin/claude-reliability work list --priority 4`
3. Ask the user if there's something they want done
4. Stop if genuinely nothing to do - it's okay to stop when work is done
