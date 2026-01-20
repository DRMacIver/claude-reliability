# Add Multiply Function

This test verifies that the agent can add a new function to a module with proper
test coverage, and that the stop hook correctly determines require_push is false
when there's no remote origin.

## Scenario

A git repository (with no remote origin) contains a Python math utilities module
with an `add_numbers` function and a working test. The project's `just check`
command requires 100% test coverage.

## Setup

- Git repository initialized but no remote origin
- Source file: `src/math_utils.py` with `add_numbers(a, b)` function
- Test file: `tests/test_math_utils.py` with a working test for add_numbers
- Justfile with `check` target that runs pytest with --cov-fail-under=100

## Expected Behavior

1. Agent adds a `multiply_numbers` function to the module
2. Agent adds a test for the new function
3. Agent runs `just check` to verify everything passes
4. Agent can stop (require_push should be false since there's no origin)

## Post-Condition

- The new `multiply_numbers` function exists and is correct
- The original `add_numbers` function is unchanged
- The justfile is unchanged
- Running `just check` passes

## Prompt

```
Add a function called multiply_numbers(a: int, b: int) -> int that multiplies two numbers together. Add a test for it.
```
