---
name: create-tasks
description: This skill should be used when the user asks to "create tasks", "break down work", "plan the implementation", or when starting complex multi-step work that needs tracking. Provides guidance on creating fine-grained tasks with proper dependencies.
---

# Creating Tasks Effectively

## Philosophy

**Tasks are cheap and exist to help you.** Don't be afraid to create many small, fine-grained tasks. Breaking work into small pieces helps you:
- Track progress clearly
- Identify blockers early
- Maintain context across interruptions
- Enable parallel work paths

## When to Create Tasks

Create a task whenever you:
- Receive a user request (capture it immediately!)
- Identify a problem while working on something else
- Plan to do something "later"
- Notice something that needs fixing

Never assume you'll remember it. Context may be compacted. **Create the task first, then decide whether to work on it now.**

## Task Structure

Good tasks are:
- **Small**: Completable in a focused session
- **Specific**: Clear definition of done
- **Independent**: Minimal dependencies where possible

## Using Dependencies for Ordering

Dependencies ensure tasks happen in the right order. Use them to:

### Test-Driven Development (TDD)
```
1. create_task("Design API for feature X")
2. create_task("Write tests for feature X")
   → add_dependency(task_id="write-tests", depends_on="design-api")
3. create_task("Implement feature X")
   → add_dependency(task_id="implement", depends_on="write-tests")
```

This ensures: Design → Tests → Implementation

### Multi-Phase Work
```
1. create_task("Research options for auth system")
2. create_task("Prototype chosen approach")
   → add_dependency(depends_on="research")
3. create_task("Full implementation")
   → add_dependency(depends_on="prototype")
4. create_task("Write documentation")
   → add_dependency(depends_on="implementation")
```

### Parallel with Sync Point
```
1. create_task("Implement frontend component")
2. create_task("Implement backend API")
3. create_task("Integration testing")
   → add_dependency(depends_on="frontend")
   → add_dependency(depends_on="backend")
```

Both frontend and backend can proceed in parallel, but testing waits for both.

## Task Granularity Examples

### Too Coarse
- "Fix the authentication system" (too vague, too large)

### Just Right
- "Add password reset endpoint to auth API"
- "Write tests for password reset flow"
- "Update user model to track reset tokens"
- "Add rate limiting to reset endpoint"

### Dependencies Model the Flow
```
create_task("Add reset_token field to User model")
create_task("Create POST /auth/reset-password endpoint")
  → depends on model change
create_task("Add email sending for reset links")
  → depends on endpoint
create_task("Write integration tests")
  → depends on all above
```

## Tips

1. **When in doubt, create a task** - You can always delete it later
2. **Add notes as you work** - Future context is valuable
3. **Use priorities realistically** - P0 is truly critical, most work is P2
4. **Mark blockers with questions** - `create_question` and `link_task_to_question`
5. **Close tasks promptly** - Don't leave completed work as open

## Quick Reference

```
# Create task
create_task(title="...", description="...", priority=2)

# Add dependency (task_id depends on depends_on)
add_dependency(task_id="...", depends_on="...")

# Start working
work_on(task_id="...")

# Add context
add_note(task_id="...", content="...")

# Mark complete
update_task(id="...", status="complete")
```
