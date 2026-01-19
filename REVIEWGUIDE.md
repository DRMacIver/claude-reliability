# Code Review Guidelines

This file provides guidance to the automated code reviewer. Customize it for your project.

## What to Check

### Security
- No hardcoded secrets, API keys, or credentials
- No SQL injection vulnerabilities
- No command injection vulnerabilities
- Proper input validation and sanitization
- Secure handling of user data

### Correctness
- Does the code do what's intended?
- Are there obvious logic errors or bugs?
- Are edge cases handled appropriately?
- What could go wrong? (boundary conditions, null/empty inputs, error states)

### Code Quality
- Clear, readable code with appropriate naming
- Proper error handling for critical paths
- Appropriate use of types and type hints
- Consistent style with the rest of the codebase
- No unnecessary complexity or over-engineering
- No redundant if/else branches or conditions that can never trigger
- No commented-out code or stale TODO comments

### Architecture
- Changes follow existing patterns in the codebase
- Appropriate separation of concerns
- No backwards-compatibility hacks for unused code

### Test Quality

**Comprehensibility:**
- Test names clearly describe what's being tested
- Tests are self-explanatory or have docstrings
- Assertions are clear about what's expected

**Realistic Failure Modes:**
- Tests could actually fail if the code is broken
- Tests aren't just checking implementation details
- Tests exercise meaningful behavior, not just code paths

**Robustness:**
- No unnecessary sleeps or timing dependencies
- No flaky assertions (ordering, floating point equality without tolerance)
- Mocks are minimal and focused
- No unstable references (hardcoded line numbers, timestamps, random values without seeds)

**Performance:**
- Tests don't do unnecessary work
- No excessive iteration counts where fewer would suffice
- Slow operations are mocked where appropriate

**Coverage:**
- New functionality has corresponding tests
- Tests cover edge cases and error paths
- No test code in production files

## When to Reject

- Security vulnerabilities (hardcoded secrets, injection risks)
- Clear bugs or logic errors
- Missing error handling for critical paths
- Breaking changes without proper handling
- Tests that can never fail or don't test meaningful behavior

## When to Approve with Feedback

- Minor style issues
- Suggestions for improvement
- Questions for clarification
- Opportunities to simplify code
- Missing test coverage for non-critical paths

## Project-Specific Guidelines

Add your project-specific review criteria here:

-
-
-
