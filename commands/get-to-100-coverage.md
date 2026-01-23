---
description: This skill should be used when the user asks to "get to 100% coverage", "improve test coverage", "cover uncovered code", "fix coverage gaps", or mentions uncovered lines or coverage percentage. Provides strategies for achieving complete test coverage through better code design.
---

# Getting to 100% Test Coverage

## Philosophy

**Coverage isn't about the number - it's about code quality.** Code that's hard to test is usually hard to understand, maintain, and reason about. Pursuing 100% coverage forces you to write better code.

### The Galahad Principle

Why 100% specifically? Because 100% provides disproportionate value compared to 95%:

**Simplicity:** A binary state (covered vs. not) provides clear, unambiguous signals. When everything is at 100%, any deviation immediately demands attention. "We have 100% coverage" is a statement you can trust. "We have 95% coverage" always raises the question: which 5% and why?

**Trust:** Complete coverage eliminates ambiguity. If coverage is always 100%, then any uncovered line is a bug - either in your code or your process. Absence of coverage becomes evidence of a problem, not just noise to filter out. You stop needing to ask "is this uncovered code intentional?" - the answer is always no.

Like Sir Galahad's pure heart granting him the strength of ten, 100% coverage gives you something that 95% coverage cannot: certainty.

Don't suppress coverage. Fix the code.

## Step 1: Create a Task

```
create_task(
  title="Achieve 100% coverage",
  description="Get all uncovered code under test",
  priority=1
)
```

## Step 2: Find Uncovered Code

```bash
# Generate coverage report
cargo llvm-cov --html

# Open the report (or view in terminal)
open target/llvm-cov/html/index.html

# Or get a summary
cargo llvm-cov --summary-only
```

Look for:
- Red lines (not covered)
- Yellow lines (partially covered - branches)
- Files with low coverage percentage

## Step 3: Categorize Each Gap

For each uncovered section, ask: **Why isn't this covered?**

### Category A: Genuinely Unreachable Code

Code that should never execute under any circumstances.

**Fix**: Make it an explicit error.

```rust
// Bad: Silent unreachable code
fn process(state: State) -> Result<()> {
    match state {
        State::A => handle_a(),
        State::B => handle_b(),
        State::C => return Ok(()), // "Can't happen" - but coverage sees it
    }
}

// Good: Explicit unreachable
fn process(state: State) -> Result<()> {
    match state {
        State::A => handle_a(),
        State::B => handle_b(),
        State::C => unreachable!("State C is never created"),
    }
}
```

The `unreachable!()` macro documents intent and will panic if your assumption is wrong.

### Category B: Defensive Code That Swallows Errors

Code that catches errors "just in case" but hides problems.

**Fix**: Let errors propagate.

```rust
// Bad: Silently swallow errors
fn load_config() -> Config {
    match std::fs::read_to_string("config.toml") {
        Ok(s) => parse_config(&s),
        Err(_) => Config::default(), // Hides file permission errors, etc.
    }
}

// Good: Propagate errors
fn load_config() -> Result<Config> {
    let s = std::fs::read_to_string("config.toml")?;
    Ok(parse_config(&s))
}

// Or if default is intentional, be explicit about which errors
fn load_config() -> Result<Config> {
    match std::fs::read_to_string("config.toml") {
        Ok(s) => Ok(parse_config(&s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.into()), // Propagate unexpected errors
    }
}
```

### Category C: Hard-to-Test Dependencies

Code that interacts with external systems (filesystem, network, time, environment).

**Fix**: Extract and inject dependencies.

#### Extract Functions

```rust
// Bad: Monolithic function
fn deploy() -> Result<()> {
    let output = Command::new("git").args(["push"]).output()?;
    if !output.status.success() {
        return Err(Error::GitPushFailed);
    }
    // ... more logic
    Ok(())
}

// Good: Extract the testable logic
fn check_command_success(output: &Output) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_check_command_success() {
        let output = Output { status: ExitStatus::from_raw(0), ... };
        assert!(check_command_success(&output).is_ok());
    }
}
```

## Tips

1. **Coverage tools show symptoms** - Low coverage reveals design problems
2. **100% is achievable** - If it's not, the code needs refactoring
3. **Tests are documentation** - Each test explains a use case
4. **Untestable code is a smell** - It's telling you something about the design
5. **Mock at boundaries** - Don't mock internal implementation details

## Quick Reference

```bash
# Generate HTML report
cargo llvm-cov --html

# Fail if under 100%
cargo llvm-cov --fail-under-lines 100

# Show uncovered lines in terminal
cargo llvm-cov --show-missing-lines

# Run specific test with coverage
cargo llvm-cov -- test_name
```
