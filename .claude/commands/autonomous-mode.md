---
description: Enter autonomous development mode with iterative task completion
---

# Autonomous Development Mode

You are entering **autonomous development mode**. This mode allows you to work
iteratively on tasks with minimal human intervention, using beads for issue
tracking and automatic progress detection.

## Setup Phase

Before starting, gather information from the user:

1. **Understand the Goal**: What should be accomplished?
2. **Define Success Criteria**: How will you know when work is complete?
3. **Identify Constraints**: Areas to avoid? Scope boundaries?
4. **Quality Requirements**: What quality gates must pass?

Write the session configuration to `.claude/autonomous-session.local.md`.

## Work Loop

For each iteration:
1. Check for available issues with `bd ready`
2. Pick the highest priority issue
3. Implement the solution
4. Run quality checks with `/quality-check`
5. If checks pass, close the issue with `bd close <id>`
6. Repeat until no issues remain or staleness detected

## Staleness Detection

Stop if no progress for 5 iterations (same issues, no closes).

## Exit Conditions

- All issues completed
- Staleness detected
- User intervention required
- Quality gates failing repeatedly
