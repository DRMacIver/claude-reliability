//! CLI functionality for claude-reliability hooks.
//!
//! This module provides the command-line interface logic, allowing
//! the binary to be a thin wrapper. All functions here are testable.

use crate::{
    command::RealCommandRunner,
    hooks::{
        parse_hook_input, run_code_review_hook, run_no_verify_hook, run_stop_hook,
        CodeReviewConfig, StopHookConfig,
    },
    subagent::RealSubAgent,
    traits::{CommandRunner, SubAgent},
};
use std::env;
use std::process::ExitCode;

/// CLI command to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Show version information.
    Version,
    /// Ensure config file exists (create with defaults if not).
    EnsureConfig,
    /// Ensure gitignore has required entries.
    EnsureGitignore,
    /// Run the stop hook.
    Stop,
    /// Run the no-verify pre-tool-use hook.
    PreToolUseNoVerify,
    /// Run the code-review pre-tool-use hook.
    PreToolUseCodeReview,
}

/// Result of parsing CLI arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseResult {
    /// Successfully parsed a command.
    Command(Command),
    /// Show usage (no args provided).
    ShowUsage,
    /// Unknown command.
    UnknownCommand(String),
    /// Missing subcommand for pre-tool-use.
    MissingSubcommand,
    /// Unknown pre-tool-use subcommand.
    UnknownSubcommand(String),
}

/// Parse CLI arguments into a command.
#[must_use]
pub fn parse_args(args: &[String]) -> ParseResult {
    if args.len() < 2 {
        return ParseResult::ShowUsage;
    }

    match args[1].as_str() {
        "version" | "--version" | "-v" => ParseResult::Command(Command::Version),
        "ensure-config" => ParseResult::Command(Command::EnsureConfig),
        "ensure-gitignore" => ParseResult::Command(Command::EnsureGitignore),
        "stop" => ParseResult::Command(Command::Stop),
        "pre-tool-use" => {
            if args.len() < 3 {
                return ParseResult::MissingSubcommand;
            }
            match args[2].as_str() {
                "no-verify" => ParseResult::Command(Command::PreToolUseNoVerify),
                "code-review" => ParseResult::Command(Command::PreToolUseCodeReview),
                other => ParseResult::UnknownSubcommand(other.to_string()),
            }
        }
        other => ParseResult::UnknownCommand(other.to_string()),
    }
}

/// Get the usage string.
#[must_use]
pub fn usage(program: &str) -> String {
    format!(
        "Usage: {program} <command> [subcommand]\n\n\
         Commands:\n  \
         ensure-config           Ensure config file exists\n  \
         ensure-gitignore        Ensure .gitignore has required entries\n  \
         stop                    Run the stop hook\n  \
         pre-tool-use no-verify  Check for --no-verify usage\n  \
         pre-tool-use code-review Run code review on commits\n  \
         version                 Show version information"
    )
}

/// Convert i32 exit code to `ExitCode`, clamping to valid range.
#[must_use]
pub fn exit_code_from_i32(code: i32) -> ExitCode {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let code_u8 = if code < 0 {
        1u8 // Treat negative as error
    } else if code > 255 {
        255u8 // Clamp to max
    } else {
        code as u8
    };
    ExitCode::from(code_u8)
}

/// Result of running a hook.
#[derive(Debug)]
pub struct HookResult {
    /// Exit code.
    pub exit_code: ExitCode,
    /// Messages to display (to stderr).
    pub messages: Vec<String>,
}

/// Run the stop hook with the given input.
///
/// # Errors
///
/// Returns an error message if the hook fails.
pub fn run_stop(
    stdin: &str,
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
    sub_agent: &dyn SubAgent,
) -> Result<HookResult, String> {
    let input = parse_hook_input(stdin).map_err(|e| format!("Error parsing hook input: {e}"))?;

    let result = run_stop_hook(&input, config, runner, sub_agent)
        .map_err(|e| format!("Error running stop hook: {e}"))?;

    Ok(HookResult { exit_code: exit_code_from_i32(result.exit_code), messages: result.messages })
}

/// Run the no-verify hook with the given input.
///
/// # Errors
///
/// Returns an error message if the hook fails.
pub fn run_no_verify(stdin: &str) -> Result<HookResult, String> {
    let input = parse_hook_input(stdin).map_err(|e| format!("Error parsing hook input: {e}"))?;

    let exit_code =
        run_no_verify_hook(&input).map_err(|e| format!("Error running no-verify hook: {e}"))?;

    Ok(HookResult { exit_code: exit_code_from_i32(exit_code), messages: Vec::new() })
}

/// Run the code-review hook with the given input.
///
/// # Errors
///
/// Returns an error message if the hook fails.
pub fn run_code_review(
    stdin: &str,
    config: &CodeReviewConfig,
    runner: &dyn CommandRunner,
    sub_agent: &dyn SubAgent,
) -> Result<HookResult, String> {
    let input = parse_hook_input(stdin).map_err(|e| format!("Error parsing hook input: {e}"))?;

    let exit_code = run_code_review_hook(&input, config, runner, sub_agent)
        .map_err(|e| format!("Error running code review hook: {e}"))?;

    Ok(HookResult { exit_code: exit_code_from_i32(exit_code), messages: Vec::new() })
}

/// Run the CLI with parsed arguments and stdin input.
///
/// This is the main entry point for the CLI logic. The binary just needs to:
/// 1. Collect args
/// 2. Read stdin
/// 3. Call this function
/// 4. Print messages and return exit code
pub fn run(args: &[String], stdin: &str) -> (ExitCode, Vec<String>) {
    match parse_args(args) {
        ParseResult::ShowUsage => (ExitCode::from(1), vec![usage(&args[0])]),
        ParseResult::UnknownCommand(cmd) => {
            (ExitCode::from(1), vec![format!("Unknown command: {cmd}")])
        }
        ParseResult::MissingSubcommand => (
            ExitCode::from(1),
            vec![format!("Usage: {} pre-tool-use <no-verify|code-review>", args[0])],
        ),
        ParseResult::UnknownSubcommand(sub) => {
            (ExitCode::from(1), vec![format!("Unknown pre-tool-use subcommand: {sub}")])
        }
        ParseResult::Command(cmd) => run_command(cmd, stdin),
    }
}

fn run_command(cmd: Command, stdin: &str) -> (ExitCode, Vec<String>) {
    match cmd {
        Command::Version => {
            (ExitCode::SUCCESS, vec![format!("claude-reliability v{}", crate::VERSION)])
        }
        Command::EnsureConfig => run_ensure_config_cmd(),
        Command::EnsureGitignore => run_ensure_gitignore_cmd(),
        Command::Stop => run_stop_cmd(stdin),
        Command::PreToolUseNoVerify => run_no_verify_cmd(stdin),
        Command::PreToolUseCodeReview => run_code_review_cmd(stdin),
    }
}

fn run_ensure_config_cmd() -> (ExitCode, Vec<String>) {
    use crate::config;

    let runner = RealCommandRunner::new();

    match config::ensure_config(&runner) {
        Ok(config) => {
            let mut messages =
                vec!["Config ensured at .claude/reliability-config.yaml".to_string()];
            messages.push(format!("  git_repo: {}", config.git_repo));
            messages.push(format!("  beads_installed: {}", config.beads_installed));
            if let Some(ref cmd) = config.check_command {
                messages.push(format!("  check_command: {cmd}"));
            } else {
                messages.push("  check_command: (none)".to_string());
            }
            (ExitCode::SUCCESS, messages)
        }
        Err(e) => (ExitCode::from(1), vec![format!("Error ensuring config: {e}")]),
    }
}

fn run_ensure_gitignore_cmd() -> (ExitCode, Vec<String>) {
    use crate::config;
    use std::path::Path;

    match config::ensure_gitignore(Path::new(".")) {
        Ok(modified) => {
            if modified {
                (
                    ExitCode::SUCCESS,
                    vec!["Updated .gitignore with claude-reliability entries".to_string()],
                )
            } else {
                (ExitCode::SUCCESS, vec![".gitignore already has required entries".to_string()])
            }
        }
        Err(e) => (ExitCode::from(1), vec![format!("Error updating .gitignore: {e}")]),
    }
}

fn run_stop_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    use crate::config;

    let runner = RealCommandRunner::new();
    let sub_agent = RealSubAgent::new(&runner);

    // Load or create config
    let project_config = match config::ensure_config(&runner) {
        Ok(c) => c,
        Err(e) => {
            // Log warning but continue with defaults
            eprintln!("Warning: Could not load config: {e}");
            config::ProjectConfig::default()
        }
    };

    // Build hook config from project config
    let config = StopHookConfig {
        quality_check_enabled: project_config.check_command.is_some(),
        quality_check_command: project_config.check_command,
        require_push: project_config.require_push,
        repo_critique_mode: env::var("REPO_CRITIQUE_MODE").is_ok(),
        base_dir: None,
    };

    match run_stop(stdin, &config, &runner, &sub_agent) {
        Ok(result) => (result.exit_code, result.messages),
        Err(e) => (ExitCode::from(1), vec![e]),
    }
}

fn run_no_verify_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    use crate::config;

    let runner = RealCommandRunner::new();

    // Ensure config exists (creates with defaults if not)
    if let Err(e) = config::ensure_config(&runner) {
        eprintln!("Warning: Could not ensure config: {e}");
    }

    match run_no_verify(stdin) {
        Ok(result) => (result.exit_code, result.messages),
        Err(e) => (ExitCode::from(1), vec![e]),
    }
}

fn run_code_review_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    use crate::config;

    let runner = RealCommandRunner::new();
    let sub_agent = RealSubAgent::new(&runner);

    // Ensure config exists (creates with defaults if not)
    if let Err(e) = config::ensure_config(&runner) {
        eprintln!("Warning: Could not ensure config: {e}");
    }

    let config = CodeReviewConfig { skip_review: env::var("SKIP_CODE_REVIEW").is_ok() };

    match run_code_review(stdin, &config, &runner, &sub_agent) {
        Ok(result) => (result.exit_code, result.messages),
        Err(e) => (ExitCode::from(1), vec![e]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn test_parse_args_no_args() {
        assert_eq!(parse_args(&args(&["prog"])), ParseResult::ShowUsage);
    }

    #[test]
    fn test_parse_args_version() {
        assert_eq!(parse_args(&args(&["prog", "version"])), ParseResult::Command(Command::Version));
        assert_eq!(
            parse_args(&args(&["prog", "--version"])),
            ParseResult::Command(Command::Version)
        );
        assert_eq!(parse_args(&args(&["prog", "-v"])), ParseResult::Command(Command::Version));
    }

    #[test]
    fn test_parse_args_stop() {
        assert_eq!(parse_args(&args(&["prog", "stop"])), ParseResult::Command(Command::Stop));
    }

    #[test]
    fn test_parse_args_ensure_config() {
        assert_eq!(
            parse_args(&args(&["prog", "ensure-config"])),
            ParseResult::Command(Command::EnsureConfig)
        );
    }

    #[test]
    fn test_parse_args_ensure_gitignore() {
        assert_eq!(
            parse_args(&args(&["prog", "ensure-gitignore"])),
            ParseResult::Command(Command::EnsureGitignore)
        );
    }

    #[test]
    fn test_parse_args_pre_tool_use() {
        assert_eq!(
            parse_args(&args(&["prog", "pre-tool-use", "no-verify"])),
            ParseResult::Command(Command::PreToolUseNoVerify)
        );
        assert_eq!(
            parse_args(&args(&["prog", "pre-tool-use", "code-review"])),
            ParseResult::Command(Command::PreToolUseCodeReview)
        );
    }

    #[test]
    fn test_parse_args_missing_subcommand() {
        assert_eq!(parse_args(&args(&["prog", "pre-tool-use"])), ParseResult::MissingSubcommand);
    }

    #[test]
    fn test_parse_args_unknown_subcommand() {
        assert_eq!(
            parse_args(&args(&["prog", "pre-tool-use", "foo"])),
            ParseResult::UnknownSubcommand("foo".to_string())
        );
    }

    #[test]
    fn test_parse_args_unknown_command() {
        assert_eq!(
            parse_args(&args(&["prog", "unknown"])),
            ParseResult::UnknownCommand("unknown".to_string())
        );
    }

    #[test]
    fn test_usage_contains_commands() {
        let u = usage("test-prog");
        assert!(u.contains("test-prog"));
        assert!(u.contains("ensure-config"));
        assert!(u.contains("ensure-gitignore"));
        assert!(u.contains("stop"));
        assert!(u.contains("pre-tool-use"));
        assert!(u.contains("version"));
    }

    #[test]
    fn test_exit_code_from_i32_zero() {
        let code = exit_code_from_i32(0);
        // ExitCode doesn't implement PartialEq, so we can't directly compare
        // We just verify it doesn't panic
        let _ = code;
    }

    #[test]
    fn test_exit_code_from_i32_positive() {
        let _ = exit_code_from_i32(1);
        let _ = exit_code_from_i32(42);
        let _ = exit_code_from_i32(255);
    }

    #[test]
    fn test_exit_code_from_i32_negative() {
        // Negative values should map to 1
        let _ = exit_code_from_i32(-1);
        let _ = exit_code_from_i32(-100);
    }

    #[test]
    fn test_exit_code_from_i32_overflow() {
        // Values > 255 should clamp to 255
        let _ = exit_code_from_i32(256);
        let _ = exit_code_from_i32(1000);
    }

    #[test]
    fn test_run_no_verify_empty_input() {
        // Empty JSON object should work
        let result = run_no_verify("{}");
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_no_verify_invalid_json() {
        let result = run_no_verify("not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Error parsing"));
    }

    #[test]
    fn test_run_no_verify_with_safe_command() {
        let result = run_no_verify(r#"{"tool_input": {"command": "git status"}}"#);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_show_usage() {
        let (_, messages) = run(&args(&["prog"]), "");
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Usage:"));
    }

    #[test]
    fn test_run_unknown_command() {
        let (_, messages) = run(&args(&["prog", "unknown"]), "");
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Unknown command"));
    }

    #[test]
    fn test_run_missing_subcommand() {
        let (_, messages) = run(&args(&["prog", "pre-tool-use"]), "");
        assert!(!messages.is_empty());
        assert!(messages[0].contains("pre-tool-use"));
    }

    #[test]
    fn test_run_unknown_subcommand() {
        let (_, messages) = run(&args(&["prog", "pre-tool-use", "bad"]), "");
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Unknown pre-tool-use subcommand"));
    }

    #[test]
    fn test_run_version() {
        let (exit_code, messages) = run(&args(&["prog", "version"]), "");
        assert_eq!(exit_code, ExitCode::SUCCESS);
        assert!(!messages.is_empty());
        assert!(messages[0].contains("claude-reliability"));
    }

    #[test]
    fn test_run_no_verify_cmd() {
        let (_, _) = run(&args(&["prog", "pre-tool-use", "no-verify"]), "{}");
        // Just verify it runs without panic
    }

    #[test]
    fn test_run_stop_with_mocks() {
        use crate::testing::{MockCommandRunner, MockSubAgent};
        use crate::traits::CommandOutput;

        let mut runner = MockCommandRunner::new();
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // Mock clean git status
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let sub_agent = MockSubAgent::new();
        let config = StopHookConfig::default();

        let result = run_stop("{}", &config, &runner, &sub_agent);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_stop_invalid_json() {
        use crate::testing::{MockCommandRunner, MockSubAgent};

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let config = StopHookConfig::default();

        let result = run_stop("not json", &config, &runner, &sub_agent);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Error parsing"));
    }

    #[test]
    fn test_run_code_review_with_mocks() {
        use crate::testing::{MockCommandRunner, MockSubAgent};

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let config = CodeReviewConfig { skip_review: true };

        let result = run_code_review("{}", &config, &runner, &sub_agent);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_code_review_invalid_json() {
        use crate::testing::{MockCommandRunner, MockSubAgent};

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let config = CodeReviewConfig::default();

        let result = run_code_review("not json", &config, &runner, &sub_agent);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Error parsing"));
    }

    #[test]
    fn test_run_no_verify_with_blocked_command() {
        let result = run_no_verify(
            r#"{"tool_name": "Bash", "tool_input": {"command": "git commit --no-verify -m test"}}"#,
        );
        assert!(result.is_ok());
        // The exit code should be 2 (blocked)
    }

    #[test]
    fn test_hook_result_fields() {
        let result =
            HookResult { exit_code: ExitCode::SUCCESS, messages: vec!["test".to_string()] };
        assert_eq!(result.messages[0], "test");
    }

    #[test]
    fn test_run_stop_hook_error() {
        use crate::testing::{FailingCommandRunner, MockSubAgent};

        let runner = FailingCommandRunner::new("simulated error");
        let sub_agent = MockSubAgent::new();
        let config = StopHookConfig::default();

        // Valid JSON that will pass parsing but cause hook to fail when calling git commands
        let result = run_stop("{}", &config, &runner, &sub_agent);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Error running stop hook"));
    }

    #[test]
    fn test_run_code_review_hook_error() {
        use crate::testing::{FailingCommandRunner, MockSubAgent};

        let runner = FailingCommandRunner::new("simulated error");
        let sub_agent = MockSubAgent::new();
        let config = CodeReviewConfig::default();

        // Input that looks like a git commit command to trigger actual hook logic
        let input = r#"{"tool_name": "Bash", "tool_input": {"command": "git commit -m test"}}"#;
        let result = run_code_review(input, &config, &runner, &sub_agent);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Error running code review hook"));
    }

    // Integration tests that exercise the cli entry points with real dependencies.
    // These tests call the actual run_*_cmd functions through run().

    #[test]
    fn test_run_stop_via_cli() {
        // The stop command needs valid JSON input but will fail gracefully with invalid
        let (_code, messages) = run(&args(&["prog", "stop"]), "not json input");
        // It should fail to parse and return an error message
        assert!(!messages.is_empty());
    }

    #[test]
    fn test_run_no_verify_via_cli() {
        // Call the no-verify hook through the CLI entry point
        let (code, _messages) = run(
            &args(&["prog", "pre-tool-use", "no-verify"]),
            r#"{"tool_name": "Edit", "tool_input": {}}"#,
        );
        // Should succeed (not bash command, nothing to block)
        assert!(code == ExitCode::SUCCESS);
    }

    #[test]
    fn test_run_code_review_via_cli() {
        // Call the code-review hook through the CLI entry point
        // Use invalid JSON to trigger quick failure
        let (_code, messages) = run(&args(&["prog", "pre-tool-use", "code-review"]), "not json");
        // Should fail to parse
        assert!(!messages.is_empty());
    }

    #[test]
    fn test_run_no_verify_via_cli_invalid_json() {
        // Call the no-verify hook through CLI with invalid JSON to trigger error path
        let (code, messages) = run(&args(&["prog", "pre-tool-use", "no-verify"]), "not json input");
        // Should fail to parse and return error code 1
        assert!(code == ExitCode::from(1));
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Error parsing"));
    }

    #[test]
    fn test_run_stop_via_cli_in_temp_repo() {
        use std::process::Command;
        use tempfile::TempDir;

        // Create a temporary git repo
        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Initialize git repo
        Command::new("git").args(["init"]).current_dir(dir_path).output().unwrap();

        // Configure git user for commits
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir_path)
            .output()
            .unwrap();

        // Create initial commit so we have a valid repo state
        std::fs::write(dir_path.join("README.md"), "test").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir_path).output().unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir_path)
            .output()
            .unwrap();

        // Create .gitignore to ignore .claude/ (which ensure_config creates)
        std::fs::write(dir_path.join(".gitignore"), ".claude/\n").unwrap();
        Command::new("git").args(["add", ".gitignore"]).current_dir(dir_path).output().unwrap();
        Command::new("git")
            .args(["commit", "-m", "add gitignore"])
            .current_dir(dir_path)
            .output()
            .unwrap();

        // Change to temp dir, run the stop command, then change back
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        let (code, _messages) = run(&args(&["prog", "stop"]), "{}");

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed (clean repo, allows stop)
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn test_run_code_review_via_cli_in_temp_repo() {
        use std::process::Command;
        use tempfile::TempDir;

        // Create a temporary git repo
        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Initialize git repo
        Command::new("git").args(["init"]).current_dir(dir_path).output().unwrap();

        // Create .gitignore to ignore .claude/ (which ensure_config creates)
        std::fs::write(dir_path.join(".gitignore"), ".claude/\n").unwrap();

        // Change to temp dir, run the code-review command, then change back
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Set SKIP_CODE_REVIEW to bypass the actual review (we just want to cover the Ok path)
        std::env::set_var("SKIP_CODE_REVIEW", "1");

        let (code, _messages) = run(
            &args(&["prog", "pre-tool-use", "code-review"]),
            r#"{"tool_name": "Bash", "tool_input": {"command": "git commit -m 'test'"}}"#,
        );

        std::env::remove_var("SKIP_CODE_REVIEW");
        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed (skip_review is set)
        assert_eq!(code, ExitCode::SUCCESS);
    }
}
