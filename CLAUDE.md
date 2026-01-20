# claude-reliability

A plugin for Claude to improve its reliability through comprehensive testing infrastructure and behavioral validation.

## Overview

This project provides tools for testing and validating Claude Code behavior by recording and replaying tool call transcripts. The infrastructure supports reliability improvements through snapshot-based testing that captures expected behavior and validates consistency across changes.

## Development Commands

```bash
just install         # Install dependencies
just test            # Run tests
just lint            # Run linters
just format          # Format code
just check           # Run all checks
```

## Snapshot Testing

The `snapshot-tests/` directory contains infrastructure for testing Claude Code behavior by recording and replaying tool call transcripts. See [snapshot-tests/SNAPSHOT_TESTING.md](snapshot-tests/SNAPSHOT_TESTING.md) for full documentation.

### Quick Reference

```bash
cd snapshot-tests

# Run all tests in replay mode (default)
python -m snapshot_tests.run_snapshots

# Run specific test with verbose output
python -m snapshot_tests.run_snapshots --verbose my-test

# Record a new transcript (runs Claude Code)
python -m snapshot_tests.run_snapshots --mode=record my-test

# Save directory snapshot from successful replay
python -m snapshot_tests.run_snapshots --save-snapshot my-test
```

### Key Concepts

- **Replay mode**: Executes recorded tool calls without running Claude Code
- **Directory snapshot**: Verifies Write/Edit produce byte-wise identical files
- **Post-conditions**: Custom verification scripts for test success criteria
- **Output normalization**: Handles timestamps, hashes, and other variable content

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

## Code Review

This section provides guidance to the automated code reviewer.

### What to Check

**Security:**
- No hardcoded secrets, API keys, or credentials
- No SQL injection vulnerabilities
- No command injection vulnerabilities
- Proper input validation and sanitization
- Secure handling of user data

**Correctness:**
- Does the code do what's intended?
- Are there obvious logic errors or bugs?
- Are edge cases handled appropriately?
- What could go wrong? (boundary conditions, null/empty inputs, error states)

**Code Quality:**
- Clear, readable code with appropriate naming
- Proper error handling for critical paths
- Appropriate use of types and type hints
- Consistent style with the rest of the codebase
- No unnecessary complexity or over-engineering
- No redundant if/else branches or conditions that can never trigger
- No commented-out code or stale TODO comments

**Architecture:**
- Changes follow existing patterns in the codebase
- Appropriate separation of concerns
- No backwards-compatibility hacks for unused code

**Test Quality:**

*Comprehensibility:*
- Test names clearly describe what's being tested
- Tests are self-explanatory or have docstrings
- Assertions are clear about what's expected

*Realistic Failure Modes:*
- Tests could actually fail if the code is broken
- Tests aren't just checking implementation details
- Tests exercise meaningful behavior, not just code paths

*Robustness:*
- No unnecessary sleeps or timing dependencies
- No flaky assertions (ordering, floating point equality without tolerance)
- Mocks are minimal and focused
- No unstable references (hardcoded line numbers, timestamps, random values without seeds)

*Performance:*
- Tests don't do unnecessary work
- No excessive iteration counts where fewer would suffice
- Slow operations are mocked where appropriate

*Coverage:*
- New functionality has corresponding tests
- Tests cover edge cases and error paths
- No test code in production files
- Tests should be runnable in parallel wherever possible, and explicitly marked as serial where not
- Coverage is kept at 100% - no attempts to change this are permitted
- Where coverage is hard to achieve, suggest refactoring to improve testability

### When to Reject

- Security vulnerabilities (hardcoded secrets, injection risks)
- Clear bugs or logic errors
- Missing error handling for critical paths
- Breaking changes without proper handling
- Tests that can never fail or don't test meaningful behavior

### When to Approve with Feedback

- Minor style issues
- Suggestions for improvement
- Questions for clarification
- Opportunities to simplify code
- Missing test coverage for non-critical paths

## Documentation

### Stop Hook State Diagram

The file `docs/stop-hook-states.dot` contains a Graphviz state diagram showing all possible states and transitions in the stop hook, from user message to agent stop.

**To regenerate this diagram after modifying `src/hooks/stop.rs`:**

Read through `src/hooks/stop.rs` and update `docs/stop-hook-states.dot` to reflect any changes to:
- Early exit conditions (problem mode, API errors, fast path, bypass phrases)
- Block conditions (validation, uncommitted changes, unpushed commits, questions, JKW mode, quality checks, self-reflection)
- Tool use patterns (git commands, beads commands, quality check commands, sub-agent calls)

**To render the diagram as PNG:**
```bash
dot -Tpng docs/stop-hook-states.dot -o docs/stop-hook-states.png
```
