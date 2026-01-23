---
name: Fix Failing GitHub Build
description: This skill should be used when the user asks to "fix the CI", "fix the build", "fix GitHub Actions", "why is the build failing", or mentions a failing CI/CD pipeline. Provides systematic workflow for diagnosing and fixing GitHub Actions failures.
---

# Fixing a Failing GitHub Build

## Overview

This skill guides you through diagnosing and fixing CI failures systematically.

## Step 1: Create a Task

First, capture the work:
```
create_task(
  title="Fix failing CI build",
  description="GitHub Actions build is failing - diagnose and fix",
  priority=1
)
```

## Step 2: Check Current GitHub Actions Status

Use `gh` to see what's failing:

```bash
# List recent workflow runs
gh run list --limit 5

# View details of the failing run
gh run view <run-id>

# View the logs
gh run view <run-id> --log-failed
```

Look for:
- Which job failed?
- Which step failed?
- What's the error message?

## Step 3: Create Subtasks

Break down the fix into steps:

```
create_task(title="Reproduce CI failure locally")
create_task(title="Identify root cause")
  → add_dependency(depends_on="reproduce")
create_task(title="Implement fix")
  → add_dependency(depends_on="identify")
create_task(title="Verify fix passes locally")
  → add_dependency(depends_on="implement")
create_task(title="Push and verify CI passes")
  → add_dependency(depends_on="verify-local")
```

## Step 4: Reproduce Locally

Run the same commands CI runs:

```bash
# Check what CI runs (look at .github/workflows/*.yml)
# Common commands:
cargo test
cargo clippy -- -D warnings
cargo fmt --check
just check  # If using just
```

If it passes locally but fails in CI:
- Check for environment differences (Rust version, OS)
- Look for flaky tests (timing, ordering)
- Check for missing dependencies in CI

## Step 5: Identify Root Cause

Common CI failures:

### Compilation Errors
- Missing dependency in Cargo.toml
- Feature flag not enabled
- Wrong Rust version

### Test Failures
- Flaky tests (timing, file system)
- Missing test fixtures
- Environment-dependent behavior

### Lint Failures
- Clippy warnings (often new lints in newer Rust)
- Formatting issues
- Missing documentation

### Coverage Failures
- Uncovered code paths
- New code without tests

## Step 6: Fix Locally

Make the fix and verify:

```bash
# Run the specific check that failed
cargo test
cargo clippy -- -D warnings

# Run full check suite
just check
```

Add a note to your task:
```
add_note(task_id="...", content="Fixed by adding missing test for edge case X")
```

## Step 7: Push and Monitor

```bash
# Commit your fix
git add -A && git commit -m "Fix CI: description of fix"

# Push
git push
```

Watch the CI run:
```bash
# Watch for the new run
gh run list --limit 1

# Watch it in real-time
gh run watch
```

## Step 8: If It Fails Again

Don't give up - iterate:

1. Check the new failure with `gh run view --log-failed`
2. Was it the same failure or a new one?
3. Update your task notes with findings
4. Fix and push again

```
add_note(task_id="...", content="First fix didn't work - new error: ...")
```

## Step 9: Complete the Task

Once CI passes:
```
update_task(id="fix-failing-ci", status="complete")
```

## Common Fixes

### "cargo fmt" check fails
```bash
cargo fmt
git add -A && git commit -m "Format code"
```

### Clippy warnings
```bash
# See what clippy wants
cargo clippy -- -D warnings

# Fix the warnings (don't just suppress them)
```

### Missing coverage
```bash
# Find uncovered lines
cargo llvm-cov --html
# Open target/llvm-cov/html/index.html

# Add tests for uncovered code
```

### Dependency issues
```bash
# Update Cargo.lock
cargo update

# Or pin a specific version in Cargo.toml
```

## Tips

1. **Don't push blind fixes** - Always reproduce locally first
2. **Read the full error** - Often the fix is obvious from the message
3. **Check recent changes** - What was the last passing commit?
4. **Use `gh run view --log`** - Full logs often reveal context
5. **Cache invalidation** - Sometimes CI caches cause issues; re-run if suspicious
