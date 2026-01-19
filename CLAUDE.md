# claude-reliability

## IMPORTANT: Fresh Project Setup

**This is a freshly created project from the drmaciver-project template.**

Your first task is to help set up this project properly:

1. **Understand the project's purpose** - Ask the user what this project is for
2. **Create a development plan** - Use beads (`bd create`) to track planned work
3. **Update this CLAUDE.md** - Replace this section with project-specific instructions

Until you've completed setup, this file contains only generic instructions. Update it with:
- Project overview and architecture
- Important conventions and patterns
- Key files and their purposes
- Any domain-specific knowledge

## Current Capabilities

This project was created with the following capabilities (see `.capabilities.json`):
- **devcontainer**: Development container with all tools pre-installed
- **claude-code**: Claude Code commands, hooks, and scripts

Additional capabilities may have been added. Check `.capabilities.json` for the full list.

## Development Commands

```bash
just install         # Install dependencies
just test            # Run tests
just lint            # Run linters
just format          # Format code
just check           # Run all checks
```

## Quality Standards

- All code must have tests
- **Warnings are errors**: Treat all warnings as serious issues that must be fixed
- No linter suppressions without clear justification
- Fix problems properly rather than suppressing errors
- Type hints on all functions

## Issue Tracking with Beads

This project uses **bd** (beads) for issue tracking. **ALWAYS track your work in beads.**

### CRITICAL: Add Issues Immediately

When the user points out a problem or requests a feature, add it to beads IMMEDIATELY:

```bash
bd create --title="<description>" --type=task|bug|feature --priority=2
```

Do NOT wait until the end of the session or until the current task is complete. Context can be lost through conversation compaction or session ends, and if issues aren't tracked immediately, they may be forgotten.

### Common Commands

```bash
bd ready              # Find available work (no blockers)
bd list --status=open # All open issues
bd show <id>          # View issue details
bd create --title="..." --type=task --priority=2  # Create new issue
bd update <id> --status=in_progress  # Claim work
bd close <id>         # Complete work
bd sync               # Sync with git
```

### Priority Levels
- P0: Critical (blocking)
- P1: High
- P2: Medium (default)
- P3: Low
- P4: Backlog

## Landing Work (Session Completion)

When ending a work session, complete ALL steps below. Work is NOT complete until pushed.

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - `just check`
3. **Update issue status** - Close finished work, update in-progress items
4. **Push to remote**:
   ```bash
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Verify** - All changes committed AND pushed
