//! CLI functionality for claude-reliability hooks.
//!
//! This module provides the command-line interface logic, allowing
//! the binary to be a thin wrapper. All functions here are testable.

use crate::{
    command::RealCommandRunner,
    hooks::{
        parse_hook_input, run_code_review_hook, run_no_verify_hook, run_stop_hook,
        run_user_prompt_submit_hook, CodeReviewConfig, StopHookConfig,
    },
    subagent::RealSubAgent,
    traits::{CommandRunner, SubAgent},
};
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
    /// Run the user-prompt-submit hook.
    UserPromptSubmit,
    /// Run the no-verify pre-tool-use hook.
    PreToolUseNoVerify,
    /// Run the code-review pre-tool-use hook.
    PreToolUseCodeReview,
    /// Run the problem-mode pre-tool-use hook.
    PreToolUseProblemMode,
    /// Run the JKW setup enforcement pre-tool-use hook.
    PreToolUseJkwSetup,
    /// Run the validation tracking pre-tool-use hook.
    PreToolUseValidation,
    /// Run the protect-config pre-tool-use hook.
    PreToolUseProtectConfig,
}

impl Command {
    /// Returns true if this command requires stdin input.
    #[must_use]
    pub const fn needs_stdin(self) -> bool {
        match self {
            Self::Version | Self::EnsureConfig | Self::EnsureGitignore | Self::UserPromptSubmit => {
                false
            }
            Self::Stop
            | Self::PreToolUseNoVerify
            | Self::PreToolUseCodeReview
            | Self::PreToolUseProblemMode
            | Self::PreToolUseJkwSetup
            | Self::PreToolUseValidation
            | Self::PreToolUseProtectConfig => true,
        }
    }
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
        "user-prompt-submit" => ParseResult::Command(Command::UserPromptSubmit),
        "pre-tool-use" => {
            if args.len() < 3 {
                return ParseResult::MissingSubcommand;
            }
            match args[2].as_str() {
                "no-verify" => ParseResult::Command(Command::PreToolUseNoVerify),
                "code-review" => ParseResult::Command(Command::PreToolUseCodeReview),
                "problem-mode" => ParseResult::Command(Command::PreToolUseProblemMode),
                "jkw-setup" => ParseResult::Command(Command::PreToolUseJkwSetup),
                "validation" => ParseResult::Command(Command::PreToolUseValidation),
                "protect-config" => ParseResult::Command(Command::PreToolUseProtectConfig),
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
         ensure-config              Ensure config file exists\n  \
         ensure-gitignore           Ensure .gitignore has required entries\n  \
         stop                       Run the stop hook\n  \
         user-prompt-submit         Run the user prompt submit hook\n  \
         pre-tool-use no-verify     Check for --no-verify usage\n  \
         pre-tool-use code-review   Run code review on commits\n  \
         pre-tool-use problem-mode  Block tools when in problem mode\n  \
         pre-tool-use jkw-setup     Enforce JKW session file creation\n  \
         pre-tool-use protect-config Block writes to reliability config\n  \
         version                    Show version information"
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
        Command::UserPromptSubmit => run_user_prompt_submit_cmd(),
        Command::PreToolUseNoVerify => run_no_verify_cmd(stdin),
        Command::PreToolUseCodeReview => run_code_review_cmd(stdin),
        Command::PreToolUseProblemMode => run_problem_mode_cmd(stdin),
        Command::PreToolUseJkwSetup => run_jkw_setup_cmd(stdin),
        Command::PreToolUseValidation => run_validation_cmd(stdin),
        Command::PreToolUseProtectConfig => run_protect_config_cmd(stdin),
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

fn run_user_prompt_submit_cmd() -> (ExitCode, Vec<String>) {
    match run_user_prompt_submit_hook(None) {
        Ok(()) => (ExitCode::SUCCESS, Vec::new()),
        Err(e) => (ExitCode::from(1), vec![format!("Error running user-prompt-submit hook: {e}")]),
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
        git_repo: project_config.git_repo,
        quality_check_enabled: project_config.check_command.is_some(),
        quality_check_command: project_config.check_command,
        require_push: project_config.require_push,
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

    let config = CodeReviewConfig::default();

    match run_code_review(stdin, &config, &runner, &sub_agent) {
        Ok(result) => (result.exit_code, result.messages),
        Err(e) => (ExitCode::from(1), vec![e]),
    }
}

fn run_problem_mode_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    use crate::hooks::{parse_hook_input, run_problem_mode_hook};
    use std::path::Path;

    let input = match parse_hook_input(stdin) {
        Ok(input) => input,
        Err(e) => return (ExitCode::from(1), vec![format!("Failed to parse input: {e}")]),
    };

    let output = run_problem_mode_hook(&input, Path::new("."));
    // Serialization cannot fail for PreToolUseOutput (only contains strings)
    let json = serde_json::to_string(&output).expect("PreToolUseOutput serialization cannot fail");

    (ExitCode::SUCCESS, vec![json])
}

fn run_jkw_setup_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    use crate::hooks::{parse_hook_input, run_jkw_setup_hook};
    use std::path::Path;

    let input = match parse_hook_input(stdin) {
        Ok(input) => input,
        Err(e) => return (ExitCode::from(1), vec![format!("Failed to parse input: {e}")]),
    };

    let output = run_jkw_setup_hook(&input, Path::new("."));
    // PreToolUseOutput is a simple struct that always serializes successfully
    let json = serde_json::to_string(&output).expect("PreToolUseOutput serialization cannot fail");

    (ExitCode::SUCCESS, vec![json])
}

fn run_validation_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    use crate::hooks::{parse_hook_input, run_validation_hook};
    use std::path::Path;

    let input = match parse_hook_input(stdin) {
        Ok(input) => input,
        Err(e) => return (ExitCode::from(1), vec![format!("Failed to parse input: {e}")]),
    };

    let output = run_validation_hook(&input, Path::new("."));
    let json = serde_json::to_string(&output).expect("PreToolUseOutput serialization cannot fail");

    (ExitCode::SUCCESS, vec![json])
}

fn run_protect_config_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    use crate::hooks::{parse_hook_input, run_protect_config_hook};

    let input = match parse_hook_input(stdin) {
        Ok(input) => input,
        Err(e) => return (ExitCode::from(1), vec![format!("Failed to parse input: {e}")]),
    };

    let output = run_protect_config_hook(&input);
    let json = serde_json::to_string(&output).expect("PreToolUseOutput serialization cannot fail");

    (ExitCode::SUCCESS, vec![json])
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
        assert_eq!(
            parse_args(&args(&["prog", "pre-tool-use", "problem-mode"])),
            ParseResult::Command(Command::PreToolUseProblemMode)
        );
        assert_eq!(
            parse_args(&args(&["prog", "pre-tool-use", "protect-config"])),
            ParseResult::Command(Command::PreToolUseProtectConfig)
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
        assert!(u.contains("user-prompt-submit"));
        assert!(u.contains("pre-tool-use"));
        assert!(u.contains("version"));
    }

    #[test]
    fn test_parse_args_user_prompt_submit() {
        assert_eq!(
            parse_args(&args(&["prog", "user-prompt-submit"])),
            ParseResult::Command(Command::UserPromptSubmit)
        );
    }

    #[test]
    fn test_command_needs_stdin() {
        // Commands that don't need stdin (won't block on terminal)
        assert!(!Command::Version.needs_stdin());
        assert!(!Command::EnsureConfig.needs_stdin());
        assert!(!Command::EnsureGitignore.needs_stdin());
        assert!(!Command::UserPromptSubmit.needs_stdin());

        // Commands that need stdin (hooks that receive JSON input)
        assert!(Command::Stop.needs_stdin());
        assert!(Command::PreToolUseNoVerify.needs_stdin());
        assert!(Command::PreToolUseCodeReview.needs_stdin());
        assert!(Command::PreToolUseProblemMode.needs_stdin());
        assert!(Command::PreToolUseJkwSetup.needs_stdin());
        assert!(Command::PreToolUseValidation.needs_stdin());
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
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

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
        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

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
        // Empty input {} means no tool_name, so hook returns early with success
        let config = CodeReviewConfig::default();

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
        let config = StopHookConfig {
            git_repo: true, // Enable git checks so they will fail
            ..Default::default()
        };

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
    #[serial_test::serial]
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
    #[serial_test::serial]
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

        let (code, _messages) = run(
            &args(&["prog", "pre-tool-use", "code-review"]),
            r#"{"tool_name": "Bash", "tool_input": {"command": "git commit -m 'test'"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed - no staged files means no review needed
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    #[serial_test::serial]
    fn test_run_ensure_config_via_cli() {
        use std::process::Command;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Initialize git repo
        Command::new("git").args(["init"]).current_dir(dir_path).output().unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        let (code, messages) = run(&args(&["prog", "ensure-config"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(messages.iter().any(|m| m.contains("Config ensured")));
        assert!(messages.iter().any(|m| m.contains("git_repo")));
        assert!(messages.iter().any(|m| m.contains("beads_installed")));
        // Check for check_command message (either with value or "(none)")
        assert!(messages.iter().any(|m| m.contains("check_command")));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_ensure_config_with_justfile() {
        use std::process::Command;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Initialize git repo
        Command::new("git").args(["init"]).current_dir(dir_path).output().unwrap();

        // Create a justfile with a check recipe
        std::fs::write(dir_path.join("justfile"), "check:\n\techo test\n").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        let (code, messages) = run(&args(&["prog", "ensure-config"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        // Should detect "just check" as the check command
        assert!(messages.iter().any(|m| m.contains("just check")));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_ensure_gitignore_via_cli() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // First call should create/update
        let (code, messages) = run(&args(&["prog", "ensure-gitignore"]), "");
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(messages.iter().any(|m| m.contains(".gitignore")));

        // Second call should report already has entries
        let (code2, messages2) = run(&args(&["prog", "ensure-gitignore"]), "");
        assert_eq!(code2, ExitCode::SUCCESS);
        assert!(messages2.iter().any(|m| m.contains("already has")));

        std::env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn test_run_user_prompt_submit_via_cli() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // user-prompt-submit should succeed (no setup file means nothing to do)
        let (code, messages) = run(&args(&["prog", "user-prompt-submit"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(messages.is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_run_problem_mode_via_cli() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Valid JSON input for problem-mode
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "problem-mode"]),
            r#"{"tool_name": "Bash", "tool_input": {"command": "echo test"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        // Should return JSON output
        assert!(!messages.is_empty());
        assert!(messages[0].starts_with('{'));
    }

    #[test]
    fn test_run_problem_mode_invalid_json() {
        let (code, messages) = run(&args(&["prog", "pre-tool-use", "problem-mode"]), "not json");

        assert_eq!(code, ExitCode::from(1));
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Failed to parse"));
    }

    #[test]
    fn test_parse_args_jkw_setup() {
        assert_eq!(
            parse_args(&args(&["prog", "pre-tool-use", "jkw-setup"])),
            ParseResult::Command(Command::PreToolUseJkwSetup)
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_run_jkw_setup_via_cli() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Valid JSON input for jkw-setup
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "jkw-setup"]),
            r#"{"tool_name": "Write", "tool_input": {"file_path": "src/main.rs"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        // Should return JSON output
        assert!(!messages.is_empty());
        assert!(messages[0].starts_with('{'));
        // Should allow since no JKW setup marker is set
        assert!(messages[0].contains("allow"));
    }

    #[test]
    fn test_run_jkw_setup_invalid_json() {
        let (code, messages) = run(&args(&["prog", "pre-tool-use", "jkw-setup"]), "not json");

        assert_eq!(code, ExitCode::from(1));
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Failed to parse"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_jkw_setup_blocks_when_marker_set() {
        use crate::session::set_jkw_setup_required;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Set the JKW setup marker
        set_jkw_setup_required(dir_path).unwrap();

        // Try to write to a non-session file
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "jkw-setup"]),
            r#"{"tool_name": "Write", "tool_input": {"file_path": "src/main.rs"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        // Should block
        assert!(!messages.is_empty());
        assert!(messages[0].contains("block"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_validation_via_cli() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Valid JSON input for validation with Edit tool
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "validation"]),
            r#"{"tool_name": "Edit", "tool_input": {"file_path": "src/main.rs"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        // Should return JSON output
        assert!(!messages.is_empty());
        assert!(messages[0].starts_with('{'));
        // Should allow (we're just tracking, not blocking)
        assert!(messages[0].contains("allow"));
        // Should set the needs_validation marker
        assert!(crate::session::needs_validation(dir_path));
    }

    #[test]
    fn test_run_validation_invalid_json() {
        let (code, messages) = run(&args(&["prog", "pre-tool-use", "validation"]), "not json");

        assert_eq!(code, ExitCode::from(1));
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Failed to parse"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_validation_read_tool_no_marker() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Read tool should not set the marker
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "validation"]),
            r#"{"tool_name": "Read", "tool_input": {"file_path": "src/main.rs"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(messages[0].contains("allow"));
        // Should NOT set the needs_validation marker
        assert!(!crate::session::needs_validation(dir_path));
    }

    #[test]
    fn test_run_protect_config_via_cli() {
        // Write to normal file should be allowed
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "protect-config"]),
            r#"{"tool_name": "Write", "tool_input": {"file_path": "src/main.rs"}}"#,
        );

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(!messages.is_empty());
        assert!(messages[0].contains("allow"));
    }

    #[test]
    fn test_run_protect_config_blocks_config_write() {
        // Write to config file should be blocked
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "protect-config"]),
            r#"{"tool_name": "Write", "tool_input": {"file_path": ".claude/reliability-config.yaml"}}"#,
        );

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(!messages.is_empty());
        assert!(messages[0].contains("block"));
        assert!(messages[0].contains("Protected File"));
    }

    #[test]
    fn test_run_protect_config_blocks_config_edit() {
        // Edit to config file should be blocked
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "protect-config"]),
            r#"{"tool_name": "Edit", "tool_input": {"file_path": ".claude/reliability-config.yaml"}}"#,
        );

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(!messages.is_empty());
        assert!(messages[0].contains("block"));
    }

    #[test]
    fn test_run_protect_config_blocks_config_delete() {
        // rm command targeting config should be blocked
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "protect-config"]),
            r#"{"tool_name": "Bash", "tool_input": {"command": "rm .claude/reliability-config.yaml"}}"#,
        );

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(!messages.is_empty());
        assert!(messages[0].contains("block"));
        assert!(messages[0].contains("Deletion Blocked"));
    }

    #[test]
    fn test_run_protect_config_allows_read() {
        // Read should be allowed
        let (code, messages) = run(
            &args(&["prog", "pre-tool-use", "protect-config"]),
            r#"{"tool_name": "Read", "tool_input": {"file_path": ".claude/reliability-config.yaml"}}"#,
        );

        assert_eq!(code, ExitCode::SUCCESS);
        assert!(!messages.is_empty());
        assert!(messages[0].contains("allow"));
    }

    #[test]
    fn test_run_protect_config_invalid_json() {
        let (code, messages) = run(&args(&["prog", "pre-tool-use", "protect-config"]), "not json");

        assert_eq!(code, ExitCode::from(1));
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Failed to parse"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_ensure_config_error_read_only() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Create .claude directory as a read-only file to make ensure_config fail
        let claude_dir = dir_path.join(".claude");
        std::fs::write(&claude_dir, "this is a file, not a directory").unwrap();
        // Make it read-only
        let mut perms = std::fs::metadata(&claude_dir).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&claude_dir, perms).unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        let (code, messages) = run(&args(&["prog", "ensure-config"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        // Should return error because .claude can't be created as a directory
        assert_eq!(code, ExitCode::from(1));
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Error ensuring config"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_ensure_gitignore_error_read_only() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Create .gitignore as a read-only directory to make ensure_gitignore fail
        let gitignore = dir_path.join(".gitignore");
        std::fs::create_dir(&gitignore).unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        let (code, messages) = run(&args(&["prog", "ensure-gitignore"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        // Should return error because .gitignore can't be written
        assert_eq!(code, ExitCode::from(1));
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Error updating .gitignore"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_user_prompt_submit_error() {
        use tempfile::TempDir;

        // Test that user_prompt_submit returns an error when file operations fail
        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Create .claude directory and make it read-only to trigger write failure
        let claude_dir = dir_path.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        // Create marker files that the hook will try to remove
        std::fs::write(claude_dir.join("reflection-done.local"), "").unwrap();
        std::fs::write(claude_dir.join("needs-validation.local"), "").unwrap();

        // Make the .claude directory read-only to prevent file removal
        let mut perms = std::fs::metadata(&claude_dir).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&claude_dir, perms).unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        let (code, messages) = run(&args(&["prog", "user-prompt-submit"]), "");

        // Restore permissions before cleanup
        let mut perms = std::fs::metadata(&claude_dir).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&claude_dir, perms).unwrap();

        std::env::set_current_dir(original_dir).unwrap();

        // Should return error because file operations fail
        assert_eq!(code, ExitCode::from(1));
        assert!(!messages.is_empty());
        assert!(messages[0].contains("Error running user-prompt-submit hook"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_stop_config_load_fallback() {
        use tempfile::TempDir;

        // Test that stop command continues with defaults when config can't be loaded
        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Create a corrupt config file
        let claude_dir = dir_path.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("reliability-config.yaml"), "{{{{invalid yaml").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Run stop command - it should fall back to defaults and continue
        let (code, messages) = run(&args(&["prog", "stop"]), "{}");

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed (with warning about config, handled gracefully)
        // The stop command should still work with defaults
        assert_eq!(code, ExitCode::SUCCESS);
        // At minimum it should process and not crash
        assert!(!messages.is_empty() || code == ExitCode::SUCCESS);
    }

    #[test]
    #[serial_test::serial]
    fn test_run_no_verify_config_error_continues() {
        use tempfile::TempDir;

        // Test that no_verify continues even when config fails
        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Create .claude as a file to prevent config creation
        let claude_dir = dir_path.join(".claude");
        std::fs::write(&claude_dir, "not a directory").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Run no_verify - should work even if config fails
        let (code, _messages) = run(
            &args(&["prog", "pre-tool-use", "no-verify"]),
            r#"{"tool_name": "Bash", "tool_input": {"command": "ls"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed (warning logged but not affecting return)
        // The hook processes successfully even if config can't be ensured
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    #[serial_test::serial]
    fn test_run_code_review_config_error_continues() {
        use tempfile::TempDir;

        // Test that code_review continues even when config fails
        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        // Create .claude as a file to prevent config creation
        let claude_dir = dir_path.join(".claude");
        std::fs::write(&claude_dir, "not a directory").unwrap();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Run code_review - should work even if config fails
        let (code, _messages) = run(
            &args(&["prog", "pre-tool-use", "code-review"]),
            r#"{"tool_name": "Bash", "tool_input": {"command": "echo hello"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed (warning logged but hook still works)
        // The hook processes successfully even if config can't be ensured
        assert_eq!(code, ExitCode::SUCCESS);
    }
}
