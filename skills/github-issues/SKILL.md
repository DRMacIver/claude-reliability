---
name: Working on GitHub Issues
description: "This skill should be used when the user asks to work on a GitHub issue, fix a bug from an issue, or implement a feature request from GitHub. Provides guidance on properly understanding issues, defining acceptance criteria, and committing work that references issues correctly."
---

# Working on GitHub Issues

## Before You Start: Read Everything

**The most common mistake when working on issues is not fully understanding the problem.**

Before writing any code:

### 1. Read the Full Issue Description

Don't just skim the title. Read the entire issue body, including:
- The problem description
- Steps to reproduce (for bugs)
- Expected vs actual behavior
- Any code examples or error messages
- Labels and assignees

### 2. Read ALL Comments

**This is critical.** Issue comments often contain:
- Clarifications from the reporter
- Additional context from maintainers
- Discussion of potential solutions
- Reasons why certain approaches won't work
- Agreement on the correct behavior or implementation

The comments might completely change your understanding of what needs to be done. An issue titled "Feature X doesn't work" might have comments explaining that it's actually working as designed, or that the real fix is something different than what the title suggests.

```bash
# View an issue with all comments
gh issue view 123 --comments

# Or for detailed JSON output
gh api repos/{owner}/{repo}/issues/123/comments
```

### 3. Check Related Issues and PRs

Look for:
- Duplicate or related issues that provide more context
- Previous PRs that attempted to fix this (and why they were closed)
- Issues that this one depends on or blocks

```bash
# Search for related issues
gh issue list --search "related keywords"

# Check if there are linked PRs
gh pr list --search "fixes #123"
```

## Define Acceptance Criteria

**Before writing code, write down what "fixed" means.**

An issue is NOT fixed just because you made changes. It's fixed when specific, verifiable conditions are met.

### Create Clear Acceptance Criteria

For each issue, define:

1. **What behavior should change?**
   - Before: [describe current behavior]
   - After: [describe expected behavior]

2. **How will you verify the fix?**
   - What test(s) will pass?
   - What manual verification steps demonstrate the fix?
   - What edge cases need to be considered?

3. **What should NOT change?**
   - Existing functionality that must be preserved
   - Backwards compatibility requirements
   - Performance characteristics

### Example: Good vs Bad Acceptance Criteria

**Bad:** "Fix the login bug"

**Good:**
- When a user enters valid credentials, they are redirected to the dashboard (not shown an error)
- When a user enters invalid credentials, they see the error message "Invalid username or password"
- Login sessions persist across browser refreshes
- Existing users' sessions are not invalidated by this change
- Add a test case for the login flow with valid credentials

### Document Your Understanding

Before implementing, write down your understanding of:
1. What the issue is asking for
2. Why the current behavior is wrong (or what's missing)
3. Your proposed solution
4. Your acceptance criteria

If anything is unclear, ask for clarification in the issue comments or ask the user before proceeding.

## Working with `gh` CLI

### Viewing Issues

```bash
# List open issues
gh issue list

# View a specific issue (includes body and comments)
gh issue view 123 --comments

# View issue in browser
gh issue view 123 --web

# Filter issues by label
gh issue list --label "bug"

# Search issues
gh issue list --search "keyword in:title,body"
```

### Creating Issues

```bash
# Create an issue interactively
gh issue create

# Create with title and body
gh issue create --title "Bug: X doesn't work" --body "Description here"

# Create from a file
gh issue create --title "Title" --body-file issue-body.md
```

### Commenting on Issues

```bash
# Add a comment
gh issue comment 123 --body "Your comment here"

# Add a comment from a file
gh issue comment 123 --body-file comment.md
```

### Closing Issues

```bash
# Close an issue
gh issue close 123

# Close with a comment
gh issue close 123 --comment "Fixed in commit abc123"
```

## Commit Practices for Issues

### Reference Issues in Commits

Every commit related to an issue should reference it:

```bash
git commit -m "Fix login redirect when credentials valid

Resolves #123"
```

### Use the Right Keywords

GitHub recognizes these keywords to automatically close issues when the commit is merged:
- `Closes #123`
- `Fixes #123`
- `Resolves #123`

Use them in your commit message when the commit fully addresses the issue:

```bash
git commit -m "Handle edge case in date parsing

Fixes #456"
```

If your commit is related but doesn't fully fix the issue, just reference it:

```bash
git commit -m "Add validation for empty input

Related to #789 - this addresses part of the issue"
```

### One Issue, Focused Commits

Each issue should have its own commits that:
1. Reference the issue number
2. Are focused on that specific issue
3. Don't mix unrelated changes

If working on multiple issues, make separate commits for each:

```bash
# Good - separate commits for separate issues
git commit -m "Fix null pointer in user lookup

Fixes #101"

git commit -m "Add retry logic for API calls

Fixes #102"
```

```bash
# Bad - mixing multiple issues in one commit
git commit -m "Fix various bugs"  # Which bugs? What issues?
```

## Common Pitfalls

### 1. Fixing the Wrong Thing

**Problem:** You fix what the title says, but the comments clarified that something else was needed.

**Solution:** Read all comments. If there's any ambiguity, clarify before implementing.

### 2. Documenting Instead of Fixing

**Problem:** The issue asks for a behavior change, but you document the existing behavior as "expected."

**Solution:** Your acceptance criteria should describe the NEW behavior. If the current behavior is actually correct, the issue should be closed as "not a bug" or "working as intended" - not fixed by documentation.

### 3. Incomplete Fixes

**Problem:** You fix the main case but miss edge cases mentioned in comments.

**Solution:** Your acceptance criteria should cover all cases mentioned in the issue and comments. Write tests for each.

### 4. Breaking Other Things

**Problem:** Your fix for issue #123 breaks functionality that was working.

**Solution:** Run the full test suite. Your acceptance criteria should include "existing tests still pass."

### 5. Not Verifying the Fix

**Problem:** You made changes and assumed they work.

**Solution:** Actually test your changes against your acceptance criteria. Run the specific scenario from the issue.

**For bug fixes, follow this workflow:**

1. **Reproduce the bug first** — Before making any changes, run the exact reproduction steps from the issue. If you can't reproduce it, you can't verify you've fixed it. If the reproduction requires setup, do the setup. Write a temporary script if needed.

2. **Apply your fix** — Make the code changes.

3. **Verify the reproduction now succeeds** — Run the exact same reproduction steps. The bug should be gone. If it's not, iterate until it is.

4. **Write a test** — If the bug was reproducible, that reproduction should become a test case.

**Never ask "please check if this worked" when you can verify it yourself.** Run the actual command, exercise the actual workflow, reproduce the actual bug. Self-validate before asking humans.

## Checklist

Before marking an issue as fixed:

- [ ] Read the full issue description
- [ ] Read ALL comments on the issue
- [ ] Checked for related issues and PRs
- [ ] Defined clear acceptance criteria
- [ ] For bugs: reproduced the issue before attempting a fix
- [ ] Implemented a fix that addresses the root cause (not just symptoms)
- [ ] Commits reference the issue number
- [ ] Added tests that verify the fix
- [ ] All existing tests still pass
- [ ] Manually verified the fix against acceptance criteria
- [ ] If creating a PR, it includes `Fixes #123` or `Closes #123` in the description

## Quick Reference

```bash
# View issue with comments
gh issue view NUMBER --comments

# List all open issues
gh issue list

# Search issues
gh issue list --search "search terms"

# Comment on an issue
gh issue comment NUMBER --body "comment"

# Close an issue
gh issue close NUMBER --comment "reason"

# Commit referencing an issue
git commit -m "Description

Fixes #NUMBER"
```
