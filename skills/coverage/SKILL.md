---
name: Test Coverage
description: "This skill should be used when the user asks to 'get to 100% coverage', 'improve test coverage', 'cover uncovered code', 'fix coverage gaps', or mentions uncovered lines or coverage percentage. Provides strategies for achieving complete test coverage through better code design."
---

# Getting to 100% Test Coverage

## Philosophy

**Coverage isn't about the number - it's about code quality.** Code that's hard to test is usually hard to understand, maintain, and reason about. Pursuing 100% coverage forces you to write better code.

### The Galahad Principle

Why 100% specifically? Because 100% provides disproportionate value compared to 95%:

**Simplicity:** A binary state (covered vs. not) provides clear, unambiguous signals. When everything is at 100%, any deviation immediately demands attention. "We have 100% coverage" is a statement you can trust. "We have 95% coverage" always raises the question: which 5% and why?

**Trust:** Complete coverage eliminates ambiguity. If coverage is always 100%, then any uncovered line is a bug - either in your code or your process. Absence of coverage becomes evidence of a problem, not just noise to filter out. You stop needing to ask "is this uncovered code intentional?" - the answer is always no.

Like Sir Galahad's pure heart granting him the strength of ten, 100% coverage gives you something that 95% coverage cannot: certainty.

**Don't suppress coverage. Fix the code.**

## Workflow

1. Find uncovered code with coverage reports
2. For each gap, categorize why it's uncovered (see references/patterns.md)
3. Apply the appropriate fix pattern
4. Verify 100% coverage

## Categorizing Coverage Gaps

For each uncovered section, ask: **Why isn't this covered?**

- **Category A: Genuinely Unreachable Code** - Make it an explicit error with `unreachable!()`
- **Category B: Defensive Error Swallowing** - Let errors propagate, or be explicit about which errors to catch
- **Category C: Hard-to-Test Dependencies** - Extract and inject dependencies
- **Category D: Error Handling Branches** - Separate IO from logic, test the logic

See `references/patterns.md` for detailed code examples of each category.

## Quick Reference

```bash
# Generate HTML report
cargo llvm-cov --html

# Fail if under 100%
cargo llvm-cov --fail-under-lines 100

# Show uncovered lines in terminal
cargo llvm-cov --show-missing-lines
```

## Key Principles

1. **Coverage tools show symptoms** - Low coverage reveals design problems
2. **100% is achievable** - If it's not, the code needs refactoring
3. **Tests are documentation** - Each test explains a use case
4. **Untestable code is a smell** - It's telling you something about the design
5. **Mock at boundaries** - Don't mock internal implementation details

## Writing Effective Tests

### Use Data That Looks Like Data
Test data should be visually distinctive and obviously different from code, variable names, and system defaults. This makes debugging vastly easier because when a value appears in a log or error message, you can immediately tell where it came from.

- **Don't:** `name = "user"`, `path = "file"`, `id = 1`, `count = 0`
- **Do:** `name = "alice-test-user"`, `path = "test-data/waveforms/meow.wav"`, `id = 42`, `count = 7`

Generic values like `1` and `0` could be defaults, sentinel values, or array indices. Distinctive values are immediately recognizable as test data.

### Write Test Names That Explain Intent
Test names and docstrings should explain WHY a behavior matters, not just WHAT is being tested. Strip boilerplate phrases like "Test that...", "should...", "correctly", and "properly" — these add no information.

- **Don't:** `test_parse_input_correctly` — "Test that input is parsed correctly"
- **Do:** `test_parse_input_extracts_config_without_validation` — explains the purpose and what would go wrong if it failed

### Be Explicit About What Tests Cover
When reporting on tests, be clear about what they verify. "All tests pass" is less useful than "tests verify X and Y, but do not exercise the path where Z happens." This gives reviewers the information they need to assess whether additional testing is warranted.

### Tests Are Checks, Not Proof
A green test suite does not mean nothing broke. Tests only verify what they were written to verify. Remain skeptical even when all tests pass — think about whether a change could affect behavior not covered by any test. Supplement automated checks with active thinking about edge cases, surprising inputs, and interactions.

## Anti-Patterns to Avoid

- Don't suppress with `#[coverage(off)]` annotations
- Don't add meaningless tests just to hit lines
- Don't mock everything - at some point you're testing mocks, not code

For more detail, see `references/patterns.md`.
