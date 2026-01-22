# How to Use Claude Reliability Tools

This guide explains how to use the task management MCP server provided by Claude Reliability.

## Overview

The Claude Reliability plugin provides a task management system through MCP (Model Context Protocol) tools. These tools help you organize work into tasks, track dependencies, store guidance as how-to guides, and manage blocking questions.

## Core Concepts

### Tasks

Tasks are the primary unit of work. Each task has:
- **ID**: Auto-generated from title (e.g., `fix-login-bug-a3f2`)
- **Title**: Short description of what needs to be done
- **Description**: Detailed explanation (optional)
- **Priority**: 0=critical, 1=high, 2=medium (default), 3=low, 4=backlog
- **Status**: open, complete, abandoned, stuck, or blocked

### Dependencies

Tasks can depend on other tasks. A task with unfinished dependencies is automatically marked as "blocked" and won't appear in `what_should_i_work_on` results.

### How-To Guides

Reusable instructions that can be linked to multiple tasks. Useful for documenting recurring procedures.

### Questions

Questions that require user input. Tasks can be linked to questions and will be blocked until the question is answered.

## Common Workflows

### Starting Work

1. **Find available work**: Use `what_should_i_work_on` to pick a random task from the highest priority unblocked tasks.

2. **List all tasks**: Use `list_tasks` with optional filters:
   - `status`: Filter by status (e.g., "open")
   - `priority`: Filter by exact priority
   - `max_priority`: Filter by maximum priority (inclusive)
   - `ready_only`: Only show tasks not blocked by dependencies

3. **Search tasks**: Use `search_tasks` with a query to find tasks by title, description, or notes.

### Creating Tasks

Use `create_task` with:
- `title` (required): Short description
- `description` (optional): Detailed explanation
- `priority` (optional): 0-4, defaults to 2 (medium)

Example:
```
create_task(title="Fix login timeout bug", description="Users report being logged out after 5 minutes", priority=1)
```

### Managing Task Progress

1. **Update status**: Use `update_task` to change status:
   - `open` → `complete` when done
   - `open` → `abandoned` if no longer needed
   - `open` → `stuck` if blocked by external factors

2. **Add notes**: Use `add_note` to record progress, findings, or context.

3. **View task details**: Use `get_task` to see full details including dependencies, notes, and linked guidance.

### Working with Dependencies

1. **Add dependency**: `add_dependency(task_id="implement-feature-xyz", depends_on="design-api-abc")`
   - The first task will be blocked until the second is complete

2. **Remove dependency**: `remove_dependency(task_id, depends_on)` when a dependency is no longer needed

### Using How-To Guides

1. **Create guide**: `create_howto(title="How to deploy to production", instructions="Step 1: ...")`

2. **Link to task**: `link_task_to_howto(task_id, howto_id)` to associate guidance with a task

3. **Search guides**: `search_howtos(query)` to find relevant documentation

### Handling Questions

When you need user input to proceed:

1. **Create question**: `create_question(text="Should we use PostgreSQL or MySQL?")`

2. **Link to task**: `link_task_to_question(task_id, question_id)` - the task will be blocked

3. **Answer question**: `answer_question(id, answer)` - linked tasks will be unblocked

4. **Find blocked tasks**: `get_question_blocked_tasks` shows all tasks waiting for answers

## Tool Reference

### Task Tools
- `create_task` - Create a new task
- `get_task` - Get task details by ID
- `update_task` - Update title, description, priority, or status
- `delete_task` - Delete a task
- `list_tasks` - List tasks with filters
- `search_tasks` - Full-text search

### Dependency Tools
- `add_dependency` - Add a dependency between tasks
- `remove_dependency` - Remove a dependency

### Note Tools
- `add_note` - Add a note to a task
- `get_notes` - Get all notes for a task

### How-To Tools
- `create_howto` - Create a how-to guide
- `get_howto` - Get guide by ID
- `update_howto` - Update title or instructions
- `delete_howto` - Delete a guide
- `list_howtos` - List all guides
- `search_howtos` - Search guides
- `link_task_to_howto` - Link task to guide
- `unlink_task_from_howto` - Remove link

### Question Tools
- `create_question` - Create a question
- `get_question` - Get question by ID
- `answer_question` - Provide an answer
- `delete_question` - Delete a question
- `list_questions` - List questions (optionally unanswered only)
- `search_questions` - Search questions
- `link_task_to_question` - Link task to question (blocks task)
- `unlink_task_from_question` - Remove link
- `get_blocking_questions` - Get unanswered questions for a task
- `get_question_blocked_tasks` - Get all tasks blocked by questions

### Utility Tools
- `what_should_i_work_on` - Pick a random high-priority unblocked task
- `get_audit_log` - View change history

## Best Practices

1. **Keep titles concise**: Titles become IDs, so keep them short but descriptive.

2. **Use priorities consistently**: Reserve P0 for truly critical issues.

3. **Add notes as you work**: Notes create a useful history of decisions and progress.

4. **Create how-to guides for repeated tasks**: If you do something more than once, document it.

5. **Use dependencies to model workflows**: Break large tasks into smaller dependent tasks.

6. **Ask questions early**: If you're blocked on a decision, create a question rather than guessing.
