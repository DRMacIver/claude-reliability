# Agent Instructions

This project uses the claude-reliability plugin for work tracking.

## Quick Reference

Work management is available through the CLI:
- `claude-reliability work create` - Create a new work item
- `claude-reliability work list` - List work items by status
- `claude-reliability work on` - Mark a work item as in-progress
- `claude-reliability work update` - Update work item status

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **Create work items for remaining work** - Create items for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update work item status** - Complete finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
