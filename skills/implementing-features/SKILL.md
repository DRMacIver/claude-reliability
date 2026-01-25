---
name: Implementing a New Feature
description: "This skill should be used when the user asks to implement a new feature, add new functionality, or build something new. Provides a systematic approach to feature implementation that avoids common pitfalls like making false assumptions about APIs or tool capabilities."
---

# Implementing a New Feature

## Before You Start

### Verify Your Assumptions

**If you are unsure how to do something or believe it is impossible, always check the official documentation for the relevant tools and libraries.** Do not make claims about what an API, library, or tool can or cannot do based on assumptions. Incorrect assumptions waste time and erode trust.

Common mistakes to avoid:
- Claiming an API doesn't support a feature without checking the docs
- Assuming a field or parameter doesn't exist because you haven't seen it
- Declaring something impossible when the documentation says otherwise
- Relying on memory when the official docs are available

**When in doubt, check the docs first. When confident, still consider checking the docs.**

### Understand the Existing Code

Before writing any new code:

1. **Read the code you're modifying.** Never propose changes to code you haven't read.
2. **Find existing patterns.** How does the codebase handle similar features? Follow those patterns.
3. **Identify the integration points.** Where does new code connect to existing code? What interfaces exist?
4. **Check for existing infrastructure.** The feature might already be partially implemented, or there might be helpers you can reuse.

## Implementation Approach

### 1. Research Phase

- Read relevant source files to understand the current architecture
- Check official documentation for APIs, tools, and libraries you'll use
- Identify existing patterns in the codebase that your implementation should follow
- Look at tests to understand expected behavior and testing patterns

### 2. Design Phase

- Plan the minimal changes needed to implement the feature
- Identify which files need modification
- Decide on function signatures, data structures, and interfaces
- Consider error handling: what can go wrong and how should it be handled?

### 3. Test-First Phase (Write Tests Before Implementation)

Write tests **before** implementing the feature. This ensures:
- You understand the expected behavior before writing code
- The interface is well-designed (if it's hard to test, it's hard to use)
- You have a clear definition of "done"

Steps:
1. Write tests that describe the expected behavior of the new feature
2. Include tests for error paths and edge cases
3. Verify the tests **fail** (they should, since the feature doesn't exist yet)
4. Use the failing tests as a guide for implementation

For each piece of functionality:
- Write a test that exercises it
- Implement just enough to make the test pass
- Refactor if needed while keeping tests green

### 4. Implementation Phase

- Make changes incrementally, guided by the failing tests
- Follow existing code style and patterns in the codebase
- Handle errors appropriately but avoid over-engineering
- Keep the implementation minimal - do what's asked, nothing more
- Run tests frequently to confirm progress

### 5. Verification Phase

- Run the full test suite to confirm nothing is broken
- Add any additional tests discovered during implementation (integration tests, additional edge cases)
- Run all quality gates (linters, formatters, tests, coverage)
- Review your changes for security issues (injection, XSS, etc.)
- Verify the implementation matches what was requested

## Common Pitfalls

### Don't Guess at APIs
If you need to use a library or tool and aren't sure of the interface, read the documentation or source code. Wrong guesses lead to wasted effort and broken code.

### Don't Over-Engineer
Implement what's requested. Don't add configuration options, feature flags, or abstractions that weren't asked for. Three lines of straightforward code is better than a premature abstraction.

### Don't Ignore the Test Suite
If the project has test infrastructure, use it. If there's a coverage requirement, meet it. Run the tests before declaring the work done.

### Don't Skip Error Handling Research
When implementing error handling, understand what errors can actually occur. Don't handle hypothetical errors that can't happen, but do handle real failure modes properly.

### Never Silently Swallow Errors
Errors you can't handle **must** be propagated, not hidden. Silent error handling makes debugging nearly impossible and hides real problems:

- **Don't** use empty catch blocks, `unwrap_or_default()` to hide failures, or `let _ = ...` to discard Results
- **Do** propagate errors with `?`, return `Result`, or log the error visibly before continuing
- If an error is truly recoverable, document **why** it's safe to ignore with a comment
- "Log and continue" is only acceptable when the log message is guaranteed to be visible

The only valid reason to swallow an error is when the operation is genuinely optional and failing to perform it has no user-visible consequences. Even then, prefer logging.

### Prefer Writing Tests Before Implementation
Writing tests after implementation often leads to tests that merely confirm the implementation rather than verifying the intended behavior. Writing tests first helps define what "correct" means before you build it.

That said, writing tests after the fact is fine when needed — especially for edge cases discovered during implementation, for coverage gaps, or when the interface wasn't clear enough upfront. The goal is not to *need* to write tests after the fact, not to avoid it entirely. If you find yourself there, write the tests — it's better to have them than not.

## Checklist

Before marking a feature as complete:

- [ ] Official documentation consulted for any unfamiliar APIs or tools
- [ ] Existing codebase patterns followed
- [ ] Tests written (ideally before implementation; after is fine if needed)
- [ ] Implementation is minimal and focused on the request
- [ ] All tests passing (including pre-existing tests)
- [ ] Quality gates pass (lints, formatting, coverage)
- [ ] No security vulnerabilities introduced
