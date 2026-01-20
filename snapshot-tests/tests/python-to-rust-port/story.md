# Python to Rust Port

This test verifies that the agent can successfully port a Python project to Rust,
delete the Python code, and ensure 100% test coverage in the Rust implementation.

## Scenario

A git repository contains a Python project with 5 modules:
- `string_utils.py` - string manipulation functions
- `math_utils.py` - mathematical operations
- `list_utils.py` - list processing functions
- `dict_utils.py` - dictionary operations
- `validation.py` - input validation functions

Each module has a simple function with corresponding tests achieving 100% coverage.

## Setup

- Git repository initialized (no remote)
- Python package structure with `src/` and `tests/`
- Justfile with `check` target for Python tests
- pytest with coverage configured for 100% enforcement

## Expected Behavior

1. Agent may ask clarifying questions about the Rust module structure
2. User responds to questions and puts agent in just-keep-working mode
3. Agent creates Rust project structure (Cargo.toml, src/lib.rs, etc.)
4. Agent ports each Python function to idiomatic Rust
5. Agent writes Rust tests for each function
6. Agent configures coverage (tarpaulin or llvm-cov)
7. Agent updates Justfile with Rust test command
8. Agent deletes all Python code
9. Agent verifies `just check` passes with 100% coverage

## Post-Condition

- No Python files remain (no .py files except in .claude/)
- Rust module exists with all 5 functions ported
- `just check` runs Rust tests successfully
- Coverage is at 100%

## Prompt

```
Port this Python project to Rust. Create a standalone Rust library (no Python bindings needed). Delete all the Python code when you're done. Don't stop until all functions are ported, tests pass, and you have 100% coverage. You can ask me questions if you need clarification.
```
