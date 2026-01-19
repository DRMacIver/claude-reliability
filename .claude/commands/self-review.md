---
description: Perform a thorough self-review of recent changes
---

# Self Review

Review recent changes systematically:

1. Run `git diff` to see all uncommitted changes
2. Check each change for:
   - Correctness (does it do what's intended?)
   - Edge cases (what could go wrong?)
   - Style consistency (matches project conventions?)
   - Test coverage (are new paths tested?)
3. Run quality checks with `/quality-check`
4. Report any issues found
