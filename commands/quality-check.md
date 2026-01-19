---
description: Run all quality gates for the project
---

# Quality Check

Run all quality gates for this project. Check the project's reliability config
at `.claude/reliability-config.yaml` for the check command.

If `check_command` is set (e.g., `just check`), run that command.

Otherwise, look for common quality check patterns:
- `just check` (if justfile exists with check target)
- `make check` (if Makefile exists with check target)
- `npm test` (if package.json exists)
- `cargo test` (if Cargo.toml exists)

Report results clearly:
- If all checks pass: "All quality gates passed"
- If any fail: List specific failures and suggest fixes
