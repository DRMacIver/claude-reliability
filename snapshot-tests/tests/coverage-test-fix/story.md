# Coverage Test Fix

This test verifies that the agent can fix a broken test to achieve 100% coverage.

## Scenario

A git repository (with no remote origin) contains Python code with a test that
doesn't actually test the function. The project's `just check` command requires
100% test coverage.

## Setup

- Git repository initialized but no remote origin
- Source file: `src/math_utils.py` with `add_numbers(a, b)` function
- Test file: `tests/test_math_utils.py` that imports but doesn't call the function
- Justfile with `check` target that runs pytest with --cov-fail-under=100

## Expected Behavior

1. Agent runs `just check` and sees coverage failure
2. Agent fixes the test to actually call the function
3. Agent runs `just check` again and it passes
4. Agent can stop (require_push should be false since there's no origin)

## Post-Condition

- The project config has require_push=false (no origin means nothing to push to)
- The test file properly tests the function
- Running `just check` passes
- Replacing the test function body with `assert False` causes the test to fail

## Prompt

```
Run just check to run the quality checks. If it fails, fix the issues and try again.
```
