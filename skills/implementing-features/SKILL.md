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

### 3. Implementation Phase

- Make changes incrementally, one logical step at a time
- Follow existing code style and patterns in the codebase
- Handle errors appropriately but avoid over-engineering
- Keep the implementation minimal - do what's asked, nothing more

### 4. Testing Phase

- Write tests that exercise the new functionality
- Cover error paths and edge cases
- Ensure tests could actually fail if the code is broken (no tautological tests)
- Run the project's test suite to verify nothing is broken

### 5. Verification Phase

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

## Checklist

Before marking a feature as complete:

- [ ] Official documentation consulted for any unfamiliar APIs or tools
- [ ] Existing codebase patterns followed
- [ ] Implementation is minimal and focused on the request
- [ ] Tests written and passing
- [ ] Quality gates pass (lints, formatting, coverage)
- [ ] No security vulnerabilities introduced
