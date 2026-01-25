---
name: Task Management
description: "This skill should be used when starting complex multi-step work, when breaking down work into smaller pieces, or when needing to track progress on tasks. Provides guidance on creating fine-grained tasks with proper dependencies."
---

# Task Management Guide

## Finding Work: what_should_i_work_on

When you need to pick a work item to work on, use the `what_should_i_work_on` tool. It automatically:
- Finds the highest priority unblocked items
- Randomly picks one to avoid always doing the same type of work
- Shows the item details so you can start immediately

**Use this instead of manually scanning the work list.**

```
# Start a work session
what_should_i_work_on()
# -> Returns a suggested work item with full details

# Then mark it in progress
work_on(work_item_id="...")
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
```
1. create_work_item("Design API for feature X")
2. create_work_item("Write tests for feature X")
   -> add_dependency(task_id="write-tests", depends_on="design-api")
3. create_work_item("Implement feature X")
   -> add_dependency(task_id="implement", depends_on="write-tests")
```

This ensures: Design -> Tests -> Implementation

### Multi-Phase Work
```
1. create_work_item("Research options for auth system")
2. create_work_item("Prototype chosen approach")
   -> add_dependency(depends_on="research")
3. create_work_item("Full implementation")
   -> add_dependency(depends_on="prototype")
4. create_work_item("Write documentation")
   -> add_dependency(depends_on="implementation")
```

### Parallel with Sync Point
```
1. create_work_item("Implement frontend component")
2. create_work_item("Implement backend API")
3. create_work_item("Integration testing")
   -> add_dependency(depends_on="frontend")
   -> add_dependency(depends_on="backend")
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

### Dependencies Model the Flow
```
create_work_item("Add reset_token field to User model")
create_work_item("Create POST /auth/reset-password endpoint")
  -> depends on model change
create_work_item("Add email sending for reset links")
  -> depends on endpoint
create_work_item("Write integration tests")
  -> depends on all above
```

## Tips

1. **When in doubt, create a work item** - You can always delete it later
2. **Add notes as you work** - Future context is valuable
3. **Use priorities realistically** - P0 is truly critical, most work is P2
4. **Mark blockers with questions** - `create_question` and `link_work_to_question`
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

```
# Find work to do
what_should_i_work_on()

# Create work item
create_work_item(title="...", description="...", priority=2)

# Add dependency (work_item_id depends on depends_on)
add_dependency(task_id="...", depends_on="...")

# Start working
work_on(work_item_id="...")

# Add context
add_note(work_item_id="...", content="...")

# Mark complete
update_work_item(id="...", status="complete")

# List open items
list_work_items(status="open", ready_only=true)
```
