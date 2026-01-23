---
name: get-to-100-coverage
description: This skill should be used when the user asks to "get to 100% coverage", "improve test coverage", "cover uncovered code", "fix coverage gaps", or mentions uncovered lines or coverage percentage. Provides strategies for achieving complete test coverage through better code design.
user-invocable: true
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

#### Create Traits for Dependency Injection

```rust
// Bad: Hardcoded dependency
fn get_current_time() -> DateTime<Utc> {
    Utc::now() // Can't test time-dependent logic
}

// Good: Inject the dependency
trait Clock {
    fn now(&self) -> DateTime<Utc>;
}

struct RealClock;
impl Clock for RealClock {
    fn now(&self) -> DateTime<Utc> { Utc::now() }
}

struct MockClock(DateTime<Utc>);
impl Clock for MockClock {
    fn now(&self) -> DateTime<Utc> { self.0 }
}

fn is_expired(clock: &impl Clock, expiry: DateTime<Utc>) -> bool {
    clock.now() > expiry
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_is_expired() {
        let mock = MockClock(Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap());
        let past = Utc.with_ymd_and_hms(2024, 1, 1, 11, 0, 0).unwrap();
        assert!(is_expired(&mock, past));
    }
}
```

#### Manipulate PATH to Mock Commands

For code that shells out to external commands:

```rust
#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use tempfile::tempdir;

    fn setup_mock_git(dir: &Path, script: &str) {
        let git_path = dir.join("git");
        fs::write(&git_path, format!("#!/bin/sh\n{}", script)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&git_path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn test_deploy_handles_git_failure() {
        let temp = tempdir().unwrap();
        setup_mock_git(temp.path(), "exit 1"); // Mock git to fail

        // Prepend our mock to PATH
        let original_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{}:{}", temp.path().display(), original_path));

        let result = deploy();

        env::set_var("PATH", original_path); // Restore
        assert!(result.is_err());
    }
}
```

### Category D: Error Handling Branches

Error paths that are hard to trigger.

**Fix**: Design for testability.

```rust
// Bad: Can't test the error branch without IO errors
fn read_and_parse(path: &Path) -> Result<Data> {
    let content = std::fs::read_to_string(path)?;
    parse(&content)
}

// Good: Separate IO from parsing
fn parse(content: &str) -> Result<Data> {
    // All parsing logic here - easy to test with bad input
}

fn read_and_parse(path: &Path) -> Result<Data> {
    let content = std::fs::read_to_string(path)?;
    parse(&content)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_invalid_input() {
        assert!(parse("not valid").is_err());
    }
}
```

## Step 4: Refactor Patterns

### The "Extract and Test" Pattern

1. Identify the hard-to-test code
2. Extract the logic into a pure function
3. Test the pure function directly
4. Leave minimal untestable glue code

### The "Trait Object" Pattern

1. Define a trait for the dependency
2. Create a real implementation for production
3. Create a mock implementation for tests
4. Inject via generics or trait objects

### The "Test Helper" Pattern

Create utilities in `#[cfg(test)]` modules:

```rust
#[cfg(test)]
mod test_helpers {
    pub fn create_mock_config() -> Config { ... }
    pub fn setup_test_environment() -> TempDir { ... }
    pub fn assert_output_matches(expected: &str, actual: &str) { ... }
}
```

## Step 5: Create Subtasks

Break down the coverage work:

```
create_task(title="Cover error handling in parse_config")
create_task(title="Add trait for HTTP client")
  → add_dependency(depends_on="error-handling")
create_task(title="Test network failure scenarios")
  → add_dependency(depends_on="http-trait")
```

## Step 6: Verify Coverage

After changes:

```bash
# Run tests with coverage
cargo llvm-cov

# Check specific file
cargo llvm-cov --html
# Look at the specific file in the report

# Verify 100%
cargo llvm-cov --fail-under-lines 100
```

## Common Anti-Patterns to Avoid

### Don't: Suppress with Annotations

```rust
// Bad: Hiding the problem
#[cfg_attr(coverage_nightly, coverage(off))]
fn some_function() { ... }
```

### Don't: Add Unreachable Tests

```rust
// Bad: Testing nothing
#[test]
fn test_unreachable() {
    // This test exists only to hit a line
}
```

### Don't: Mock Everything

```rust
// Bad: Testing mocks, not code
#[test]
fn test_with_all_mocks() {
    let mock_db = MockDb::new();
    let mock_http = MockHttp::new();
    let mock_fs = MockFs::new();
    // At this point, what are you even testing?
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
