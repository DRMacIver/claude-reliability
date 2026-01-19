//! No-verify hook to prevent bypassing git pre-commit hooks.
//!
//! This hook blocks `git commit --no-verify` and `git commit -n` unless
//! explicitly acknowledged via the `NO_VERIFY_OK` environment variable.

use crate::error::Result;
use crate::hooks::{HookInput, PreToolUseOutput};
use once_cell::sync::Lazy;
use regex::Regex;
use std::env;
use std::io::Write;

/// Acknowledgment phrase required in `NO_VERIFY_OK` env var.
const ACKNOWLEDGMENT: &str = "I promise the user has said I can use --no-verify here";

/// Patterns that match git commit with --no-verify or -n flag.
static NO_VERIFY_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"\bgit\s+commit\b.*--no-verify\b").unwrap(),
        Regex::new(r"\bgit\s+commit\b.*\s-[a-zA-Z]*n").unwrap(), // -n anywhere in flags
    ]
});

/// Result of the no-verify check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoVerifyResult {
    /// No --no-verify flag found, allow.
    NoFlag,
    /// Has --no-verify but acknowledged, allow.
    Acknowledged,
    /// Has --no-verify and NOT acknowledged, block.
    Blocked,
}

/// Check no-verify with an explicit acknowledgment value (for testing).
fn check_no_verify_with_ack(command: &str, ack_value: Option<&str>) -> NoVerifyResult {
    // Check if this is a git commit with --no-verify or -n
    let has_no_verify = NO_VERIFY_PATTERNS.iter().any(|re| re.is_match(command));

    if !has_no_verify {
        return NoVerifyResult::NoFlag;
    }

    // Check for the acknowledgment
    if let Some(no_verify_ok) = ack_value {
        if no_verify_ok.contains(ACKNOWLEDGMENT) {
            return NoVerifyResult::Acknowledged;
        }
    }

    NoVerifyResult::Blocked
}

/// Run the no-verify hook.
///
/// Returns exit code: 0 = allow, 2 = block.
///
/// # Errors
///
/// Returns an error if writing to stderr fails.
pub fn run_no_verify_hook(input: &HookInput) -> Result<i32> {
    let ack_value = env::var("NO_VERIFY_OK").ok();
    run_no_verify_hook_with_ack(input, ack_value.as_deref())
}

/// Run the no-verify hook with an explicit acknowledgment value (for testing).
///
/// # Errors
///
/// Returns an error if writing to stderr fails.
fn run_no_verify_hook_with_ack(input: &HookInput, ack_value: Option<&str>) -> Result<i32> {
    // Only run for Bash tool calls
    if input.tool_name.as_deref() != Some("Bash") {
        return Ok(0);
    }

    // Get the command
    let command = input.tool_input.as_ref().and_then(|t| t.command.as_deref()).unwrap_or("");

    match check_no_verify_with_ack(command, ack_value) {
        NoVerifyResult::NoFlag => Ok(0),
        NoVerifyResult::Acknowledged => {
            eprintln!("--no-verify acknowledged by NO_VERIFY_OK environment variable");
            Ok(0)
        }
        NoVerifyResult::Blocked => {
            let mut stderr = std::io::stderr();
            writeln!(stderr, "ERROR: Attempting to use git commit with --no-verify.")?;
            writeln!(stderr)?;
            writeln!(stderr, "The --no-verify flag skips pre-commit hooks, which are")?;
            writeln!(stderr, "important for:")?;
            writeln!(stderr, "- Running quality checks before commits")?;
            writeln!(stderr, "- Preventing secrets from being committed")?;
            writeln!(stderr, "- Ensuring beads are properly synced")?;
            writeln!(stderr)?;
            writeln!(stderr, "If the user has explicitly said you can skip hooks, set:")?;
            writeln!(stderr)?;
            writeln!(stderr, "  NO_VERIFY_OK=\"{ACKNOWLEDGMENT}\"")?;
            writeln!(stderr)?;
            Ok(2)
        }
    }
}

/// Generate JSON output for `PreToolUse` hook.
#[allow(dead_code)] // Available for when hook is used as PreToolUse
#[allow(clippy::needless_pass_by_value)] // Simple enum, passing by value is cleaner
pub fn generate_output(result: NoVerifyResult) -> Option<PreToolUseOutput> {
    match result {
        NoVerifyResult::Blocked => Some(PreToolUseOutput::block(Some(
            "git commit --no-verify is not allowed without explicit acknowledgment".to_string(),
        ))),
        _ => None, // No output needed for allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_no_verify_not_git() {
        assert_eq!(check_no_verify_with_ack("echo hello", None), NoVerifyResult::NoFlag);
    }

    #[test]
    fn test_check_no_verify_normal_commit() {
        assert_eq!(check_no_verify_with_ack("git commit -m 'test'", None), NoVerifyResult::NoFlag);
    }

    #[test]
    fn test_check_no_verify_with_flag() {
        assert_eq!(
            check_no_verify_with_ack("git commit --no-verify -m 'test'", None),
            NoVerifyResult::Blocked
        );
        assert_eq!(
            check_no_verify_with_ack("git commit -n -m 'test'", None),
            NoVerifyResult::Blocked
        );
        assert_eq!(
            check_no_verify_with_ack("git commit -am 'test' --no-verify", None),
            NoVerifyResult::Blocked
        );
    }

    #[test]
    fn test_check_no_verify_acknowledged() {
        let ack = Some("I promise the user has said I can use --no-verify here");
        assert_eq!(
            check_no_verify_with_ack("git commit --no-verify -m 'test'", ack),
            NoVerifyResult::Acknowledged
        );
    }

    #[test]
    fn test_check_no_verify_wrong_acknowledgment() {
        let ack = Some("wrong phrase");
        assert_eq!(
            check_no_verify_with_ack("git commit --no-verify -m 'test'", ack),
            NoVerifyResult::Blocked
        );
    }

    #[test]
    fn test_generate_output_blocked() {
        let output = generate_output(NoVerifyResult::Blocked);
        assert!(output.is_some());
        let json = serde_json::to_string(&output.unwrap()).unwrap();
        assert!(json.contains("block"));
    }

    #[test]
    fn test_generate_output_allow() {
        assert!(generate_output(NoVerifyResult::NoFlag).is_none());
        assert!(generate_output(NoVerifyResult::Acknowledged).is_none());
    }

    #[test]
    fn test_run_no_verify_hook_not_bash() {
        let input = HookInput { tool_name: Some("Read".to_string()), ..Default::default() };
        assert_eq!(run_no_verify_hook(&input).unwrap(), 0);
    }

    #[test]
    fn test_run_no_verify_hook_no_tool_name() {
        let input = HookInput::default();
        assert_eq!(run_no_verify_hook(&input).unwrap(), 0);
    }

    #[test]
    fn test_run_no_verify_hook_normal_command() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput { command: Some("git status".to_string()) }),
            ..Default::default()
        };
        assert_eq!(run_no_verify_hook(&input).unwrap(), 0);
    }

    #[test]
    fn test_run_no_verify_hook_blocked() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit --no-verify -m 'test'".to_string()),
            }),
            ..Default::default()
        };
        assert_eq!(run_no_verify_hook(&input).unwrap(), 2);
    }

    #[test]
    fn test_run_no_verify_hook_no_command() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput { command: None }),
            ..Default::default()
        };
        assert_eq!(run_no_verify_hook(&input).unwrap(), 0);
    }

    #[test]
    fn test_run_no_verify_hook_no_tool_input() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: None,
            ..Default::default()
        };
        assert_eq!(run_no_verify_hook(&input).unwrap(), 0);
    }

    #[test]
    fn test_run_no_verify_hook_acknowledged() {
        let ack = Some("I promise the user has said I can use --no-verify here");
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(crate::hooks::ToolInput {
                command: Some("git commit --no-verify -m 'test'".to_string()),
            }),
            ..Default::default()
        };

        // Should allow (exit code 0) since acknowledged
        assert_eq!(run_no_verify_hook_with_ack(&input, ack).unwrap(), 0);
    }
}
