---
name: Implementing a New Feature
description: "This skill should be used when the user asks to implement a new feature, add new functionality, or build something new. Provides a systematic approach to feature implementation that avoids common pitfalls like making false assumptions about APIs or tool capabilities."
---

# Implementing a New Feature

## Before You Start

### Understand the Feature Intent

Before writing any code, ensure you understand **why** this feature exists and **what success looks like**.

**1. Clarify the intent:**
- What problem does this feature solve?
- Who will use it and in what context?
- What is the user trying to accomplish when they use this feature?

**2. Sketch out usage scenarios:**
- Describe 2-3 concrete scenarios where someone would use this feature
- What does each scenario look like from the user's perspective?
- What inputs would they provide? What outputs would they expect?

**3. Define requirements for usefulness:**
- For each scenario, what must be true for the implementation to actually help?
- What behavior would make the feature useless or frustrating?
- Are there edge cases within these scenarios that need handling?

**4. Validate understanding:**
- If the requirements are ambiguous, ask clarifying questions before proceeding
- If you're uncertain about the intended use, propose your understanding and confirm it
- It's better to spend time clarifying upfront than to build the wrong thing

**Document these findings.** Write down the intent, scenarios, and requirements somewhere (a work item note, a comment, or just in your working memory). You'll need to verify against them after implementation.

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

### 5. Runtime Validation Phase

**Actually run the code.** Tests are necessary but not sufficient. Before declaring a feature complete, execute it in a realistic scenario.

Why this matters:
- Tests often exercise code in isolation; real usage involves integration
- Tests may mock dependencies that behave subtly differently from the real thing
- The gap between "tests pass" and "feature works" is where bugs hide

**How to validate:**

1. **For features with a user interface or CLI:**
   - Actually use it. Run the command. Click the button. Fill out the form.
   - If the feature requires setup, do the setup and verify the whole flow.

2. **For internal features or APIs:**
   - Write a temporary script that exercises the feature end-to-end
   - Put temporary scripts in gitignored directories (e.g., `tmp/`, `.scratch/`)
   - Run the script repeatedly, fixing issues until it works

3. **For bug fixes:**
   - Reproduce the original bug first (before applying the fix)
   - Apply the fix
   - Verify the reproduction now succeeds

**Iterate until it works:**
- Don't just run it once. Run it multiple times with different inputs.
- If it fails, fix it and run again. Don't move on until it actually works.
- Temporary validation scripts can be messy — they're for validation, not production.

**After validation:**
- Refactor any useful temporary code into permanent locations (tests, utilities)
- Delete scratch files that are no longer needed
- If the temporary script revealed test gaps, add proper tests

**Key principle:** Never ask "does this work?" when you can answer it yourself by running the code. Self-validate before asking humans.

### 6. Final Verification Phase

- Run the full test suite to confirm nothing is broken
- Add any additional tests discovered during implementation (integration tests, additional edge cases)
- Run all quality gates (linters, formatters, tests, coverage)
- Review your changes for security issues (injection, XSS, etc.)
- Verify the implementation matches what was requested

**Revisit the feature intent:**
- Return to the intent, scenarios, and requirements you identified before starting
- For each scenario: walk through it with the actual implementation. Does it work as expected?
- For each requirement: confirm it is met. If a requirement isn't met, the feature isn't complete.
- If you discover the implementation doesn't serve the original intent, either fix it or explicitly discuss the gap with the user

## Design Principles

### Parse, Don't Validate
Transform unstructured data into structured types at system boundaries rather than checking validity repeatedly throughout code. Validation discards information (returning a boolean); parsing preserves it by producing a refined type. Concentrate parsing at the system edge so that internal code can trust its inputs structurally.

- Use data structures that make illegal states impossible. Prefer `Map`/`Dict` over a list of pairs when uniqueness is required. Prefer an enum with fixed variants over a string when there are known valid values.
- Avoid "shotgun parsing" — scattering validation checks throughout processing code.
- If a function checks a condition and returns nothing, ask whether it should instead return a refined value that captures what was learned.

### Push Ifs Up and Fors Down
Move conditional logic *up* toward callers so callees have simpler, unconditional implementations. Move loops *down* into lower-level functions so they operate on batches.

- If a function accepts `Optional<X>` and the first thing it does is return early on `None`, change the signature to require `X` and make the caller responsible for the check.
- Instead of calling a function in a loop, pass the entire collection to a function that processes it internally. This enables batch optimizations and amortizes overhead.
- The ideal architecture places decisions at the top level and batch processing at the bottom level.

### Write Code That Is Easy to Delete
Every line of code is a liability. Optimize for disposability over extensibility. Write straightforward, slightly duplicated code that can be easily understood and deleted, rather than building elaborate abstractions. Write modules with clear boundaries so that removing a feature means deleting a file, not performing surgery across twenty files.

### Resist Premature Abstraction
Wrong abstractions are more damaging than duplication. DRY only makes sense when it reduces the cost of change more than it increases the cost of understanding. Three similar lines of code is better than a premature abstraction. Resist creating abstractions until they insist upon being created — if you have seen only one or two instances, it is too early to abstract.

### Choose Boring Technology
Default to established tools and patterns already used in the project. Do not introduce new dependencies, frameworks, or languages without a compelling reason that outweighs the cost of adding something unfamiliar to the stack. Each new dependency is a long-term maintenance commitment.

## Common Pitfalls

### Don't Guess at APIs
If you need to use a library or tool and aren't sure of the interface, read the documentation or source code. Wrong guesses lead to wasted effort and broken code. In the face of ambiguity, refuse the temptation to guess — read the reference, use a debugger, be thorough.

### Don't Over-Engineer
Implement what's requested. Don't add configuration options, feature flags, or abstractions that weren't asked for. Three lines of straightforward code is better than a premature abstraction. Before reaching for complex architectures, verify that a simple single-threaded implementation would not be sufficient.

### Don't Ignore the Test Suite
If the project has test infrastructure, use it. If there's a coverage requirement, meet it. Run the tests before declaring the work done.

### Classify Errors: Bugs vs. Recoverable Conditions
Before writing error handling code, classify the error. **Bugs** are conditions you didn't expect (null dereferences, logic errors, failed assertions) — the correct response is to fail fast. **Recoverable errors** are foreseeable conditions (network failures, parse errors, invalid input) — the correct response is explicit handling: retry, fallback, or propagate.

Do not write elaborate recovery logic for "impossible" conditions. Assert and fail instead. Focus recovery code on genuinely recoverable situations.

### Never Silently Swallow Errors
Errors you can't handle **must** be propagated, not hidden. Silent error handling makes debugging nearly impossible and hides real problems:

- **Don't** use empty catch blocks, `unwrap_or_default()` to hide failures, or `let _ = ...` to discard Results
- **Do** propagate errors with `?`, return `Result`, or log the error visibly before continuing
- If an error is truly recoverable, document **why** it's safe to ignore with a comment
- "Log and continue" is only acceptable when the log message is guaranteed to be visible

The only valid reason to swallow an error is when the operation is genuinely optional and failing to perform it has no user-visible consequences. Even then, prefer logging.

### Use Assertions Liberally
Validate preconditions at the top of functions. Use assertions at the end of functions to verify your work. Treat assertions as documentation of invariants — they communicate intent to future readers.

### Refactor Incrementally, Not in Big Batches
When you need to touch an area of code to deliver a feature, clean up that area as you go. Do not propose sweeping refactoring campaigns across code you do not need to touch for the current task. Small, incremental improvements bundled with feature work are easy to review. If you rename a concept or change an approach, do so consistently throughout the codebase — don't leave a refactoring half-done with two patterns coexisting.

### Prefer Writing Tests Before Implementation
Writing tests after implementation often leads to tests that merely confirm the implementation rather than verifying the intended behavior. Writing tests first helps define what "correct" means before you build it.

That said, writing tests after the fact is fine when needed — especially for edge cases discovered during implementation, for coverage gaps, or when the interface wasn't clear enough upfront. The goal is not to *need* to write tests after the fact, not to avoid it entirely. If you find yourself there, write the tests — it's better to have them than not.

### When Something Inexplicably Fails, Check the Mundane
Before diving into complex debugging, verify: Am I editing the right file? Did the changes actually get saved? Am I running the code I think I am? Is there stale build state? Did I forget a step? These take seconds to check and eliminate the most common causes of "impossible" bugs.

## Checklist

Before marking a feature as complete:

- [ ] Feature intent understood (what problem does it solve, for whom?)
- [ ] Usage scenarios sketched out (2-3 concrete examples of how it will be used)
- [ ] Requirements for usefulness defined (what must be true for each scenario to work?)
- [ ] Official documentation consulted for any unfamiliar APIs or tools
- [ ] Existing codebase patterns followed
- [ ] Tests written (ideally before implementation; after is fine if needed)
- [ ] Implementation is minimal and focused on the request
- [ ] All tests passing (including pre-existing tests)
- [ ] Feature validated by actually running it (not just tests passing)
- [ ] Quality gates pass (lints, formatting, coverage)
- [ ] No security vulnerabilities introduced
- [ ] Error handling classifies bugs vs. recoverable errors appropriately
- [ ] No new dependencies added without justification
- [ ] Original scenarios revisited and confirmed to work with the implementation
- [ ] All requirements for usefulness verified as met
