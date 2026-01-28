---
name: Task Management
description: "This skill should be used when starting complex multi-step work, when breaking down work into smaller pieces, or when needing to track progress on tasks. Provides guidance on creating fine-grained tasks with proper dependencies."
---

# Task Management Guide

## Finding Work: work next

When you need to pick a work item to work on, use the `work next` command. It automatically:
- Finds the highest priority unblocked items
- Randomly picks one to avoid always doing the same type of work
- Shows the item details so you can start immediately

**Use this instead of manually scanning the work list.**

```bash
# Start a work session
claude-reliability work next
# -> Returns a suggested work item with full details

# Then mark it in progress
claude-reliability work on <work-item-id>
```

## Creating Work Items Effectively

### Philosophy

**Work items are cheap and exist to help you.** Don't be afraid to create many small, fine-grained items. Breaking work into small pieces helps you:
- Track progress clearly
- Identify blockers early
- Maintain context across interruptions
- Enable parallel work paths

## When to Create Work Items

Create a work item whenever you:
- Receive a user request (capture it immediately!)
- Identify a problem while working on something else
- Plan to do something "later"
- Notice something that needs fixing

Never assume you'll remember it. Context may be compacted. **Create the work item first, then decide whether to work on it now.**

## Work Item Structure

Good work items are:
- **Small**: Completable in a focused session
- **Specific**: Clear definition of done
- **Independent**: Minimal dependencies where possible

## Using Dependencies for Ordering

Dependencies ensure items happen in the right order. Use them to:

### Test-Driven Development (TDD)
```bash
# 1. Create work items
claude-reliability work create -t "Design API for feature X"
claude-reliability work create -t "Write tests for feature X"
claude-reliability work create -t "Implement feature X"

# 2. Add dependencies (use IDs from creation output)
claude-reliability work add-dep <write-tests-id> --depends-on <design-api-id>
claude-reliability work add-dep <implement-id> --depends-on <write-tests-id>
```

This ensures: Design -> Tests -> Implementation

### Multi-Phase Work
```bash
claude-reliability work create -t "Research options for auth system"
claude-reliability work create -t "Prototype chosen approach"
claude-reliability work create -t "Full implementation"
claude-reliability work create -t "Write documentation"

# Chain dependencies
claude-reliability work add-dep <prototype-id> --depends-on <research-id>
claude-reliability work add-dep <implement-id> --depends-on <prototype-id>
claude-reliability work add-dep <docs-id> --depends-on <implement-id>
```

### Parallel with Sync Point
```bash
claude-reliability work create -t "Implement frontend component"
claude-reliability work create -t "Implement backend API"
claude-reliability work create -t "Integration testing"

# Testing depends on both
claude-reliability work add-dep <testing-id> --depends-on <frontend-id>
claude-reliability work add-dep <testing-id> --depends-on <backend-id>
```

Both frontend and backend can proceed in parallel, but testing waits for both.

## Work Item Granularity Examples

### Too Coarse
- "Fix the authentication system" (too vague, too large)

### Just Right
- "Add password reset endpoint to auth API"
- "Write tests for password reset flow"
- "Update user model to track reset tokens"
- "Add rate limiting to reset endpoint"

## Tips

1. **When in doubt, create a work item** - You can always delete it later
2. **Add notes as you work** - Future context is valuable
3. **Use priorities realistically** - P0 is truly critical, most work is P2
4. **Mark blockers with questions** - `question create` and `question link`
5. **Close items promptly** - Don't leave completed work as open

## Bulk Work Item Creation

When creating 3+ items at once, use the bulk-tasks binary for better performance:

```bash
${CLAUDE_PLUGIN_ROOT}/bin/bulk-tasks create <<'EOF'
{
  "tasks": [
    {"id": "t1", "title": "First item", "description": "...", "priority": 1},
    {"id": "t2", "title": "Second item", "priority": 2, "depends_on": ["t1"]},
    {"id": "t3", "title": "Third item", "priority": 2, "depends_on": ["t1", "t2"]}
  ]
}
EOF
```

The `id` fields are temporary for setting up dependencies. Real IDs are returned.

## Quick Reference

```bash
# Find work to do
claude-reliability work next

# Create work item
claude-reliability work create -t "Task title" -d "Description" -p 2

# Get work item details
claude-reliability work get <id>

# Add dependency (work_item_id depends on depends_on)
claude-reliability work add-dep <id> --depends-on <other-id>

# Start working
claude-reliability work on <id>

# Add context
claude-reliability work add-note <id> -c "Progress note here"

# Mark complete
claude-reliability work update <id> --status complete

# List open items
claude-reliability work list --status open --ready-only
```
