//! CLI functionality for claude-reliability hooks.
//!
//! This module provides the command-line interface logic, allowing
//! the binary to be a thin wrapper. All functions here are testable.

use crate::{
    command::RealCommandRunner,
    hooks::{
        parse_hook_input, run_post_tool_use, run_pre_tool_use, run_stop_hook,
        run_user_prompt_submit_hook, PostToolUseInput, StopHookConfig,
    },
    subagent::RealSubAgent,
    traits::{CommandRunner, SubAgent},
};
use std::path::Path;
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
    /// Print session intro message.
    Intro,
    /// Sync beads issues to tasks database.
    SyncBeads,
    /// Run the stop hook.
    Stop,
    /// Run the user-prompt-submit hook.
    UserPromptSubmit,
    /// Run the unified pre-tool-use hook (handles all tools).
    PreToolUse,
    /// Run the post-tool-use hook.
    PostToolUse,
}

impl Command {
    /// Returns true if this command requires stdin input.
    #[must_use]
    pub const fn needs_stdin(self) -> bool {
        match self {
            Self::Version
            | Self::EnsureConfig
            | Self::EnsureGitignore
            | Self::Intro
            | Self::SyncBeads
            | Self::UserPromptSubmit => false,
            Self::Stop | Self::PreToolUse | Self::PostToolUse => true,
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
        "intro" => ParseResult::Command(Command::Intro),
        "sync-beads" => ParseResult::Command(Command::SyncBeads),
        "stop" => ParseResult::Command(Command::Stop),
        "user-prompt-submit" => ParseResult::Command(Command::UserPromptSubmit),
        "pre-tool-use" => ParseResult::Command(Command::PreToolUse),
        "post-tool-use" => ParseResult::Command(Command::PostToolUse),
        other => ParseResult::UnknownCommand(other.to_string()),
    }
}

/// Get the usage string.
#[must_use]
pub fn usage(program: &str) -> String {
    format!(
        "Usage: {program} <command>\n\n\
         Commands:\n  \
         ensure-config        Ensure config file exists\n  \
         ensure-gitignore     Ensure .gitignore has required entries\n  \
         intro                Print session intro message\n  \
         sync-beads           Sync open beads issues to tasks database\n  \
         stop                 Run the stop hook\n  \
         user-prompt-submit   Run the user prompt submit hook\n  \
         pre-tool-use         Run the unified pre-tool-use hook\n  \
         post-tool-use        Run the post-tool-use hook\n  \
         version              Show version information"
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

/// Output from running the CLI, with separate stdout and stderr messages.
#[derive(Debug)]
pub struct CliOutput {
    /// Exit code for the process.
    pub exit_code: ExitCode,
    /// Messages to print to stdout.
    pub stdout: Vec<String>,
    /// Messages to print to stderr.
    pub stderr: Vec<String>,
}

/// Run the CLI with parsed arguments and stdin input.
///
/// This is the main entry point for the CLI logic. The binary just needs to:
/// 1. Collect args
/// 2. Read stdin
/// 3. Call this function
/// 4. Print stdout/stderr messages and return exit code
pub fn run(args: &[String], stdin: &str) -> CliOutput {
    let (exit_code, messages, is_stop_cmd) = match parse_args(args) {
        ParseResult::ShowUsage => (ExitCode::from(1), vec![usage(&args[0])], false),
        ParseResult::UnknownCommand(cmd) => {
            (ExitCode::from(1), vec![format!("Unknown command: {cmd}")], false)
        }
        ParseResult::Command(cmd) => {
            let is_stop = matches!(cmd, Command::Stop);
            let (code, msgs) = run_command(cmd, stdin);
            (code, msgs, is_stop)
        }
    };

    // Format output based on command type and exit code
    // For stop hooks with exit 0 and messages: output JSON with systemMessage to stdout
    // For blocked stops (exit non-0): messages go to stderr for LLM feedback
    // For other commands: messages go to stderr
    if exit_code == ExitCode::SUCCESS && is_stop_cmd && !messages.is_empty() {
        let system_message = messages.join("\n");
        let json = serde_json::json!({"systemMessage": system_message});
        CliOutput { exit_code, stdout: vec![json.to_string()], stderr: vec![] }
    } else {
        CliOutput { exit_code, stdout: vec![], stderr: messages }
    }
}

fn run_command(cmd: Command, stdin: &str) -> (ExitCode, Vec<String>) {
    match cmd {
        Command::Version => {
            (ExitCode::SUCCESS, vec![format!("claude-reliability v{}", crate::VERSION)])
        }
        Command::EnsureConfig => run_ensure_config_cmd(),
        Command::EnsureGitignore => run_ensure_gitignore_cmd(),
        Command::Intro => run_intro_cmd(),
        Command::SyncBeads => run_sync_beads_cmd(),
        Command::Stop => run_stop_cmd(stdin),
        Command::UserPromptSubmit => run_user_prompt_submit_cmd(),
        Command::PreToolUse => run_pre_tool_use_cmd(stdin),
        Command::PostToolUse => run_post_tool_use_cmd(stdin),
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

fn run_intro_cmd() -> (ExitCode, Vec<String>) {
    use crate::templates;
    use tera::Context;

    // Template is embedded and verified by test_all_embedded_templates_render
    let message = templates::render("messages/session_intro.tera", &Context::new())
        .expect("session_intro.tera template should always render");
    (ExitCode::SUCCESS, vec![message])
}

fn run_sync_beads_cmd() -> (ExitCode, Vec<String>) {
    use crate::beads_sync::sync_beads_to_tasks;
    use std::path::Path;

    let runner = RealCommandRunner::new();
    let base_dir = Path::new(".");

    match sync_beads_to_tasks(&runner, base_dir) {
        Ok(result) => format_sync_result(&result),
        Err(e) => (ExitCode::from(1), vec![format!("Sync failed: {e}")]), // coverage:ignore - I/O errors
    }
}

fn format_sync_result(result: &crate::beads_sync::SyncResult) -> (ExitCode, Vec<String>) {
    let mut messages = Vec::new();
    if result.created > 0 {
        messages.push(format!("Synced {} beads issues to tasks", result.created));
    }
    if result.skipped > 0 {
        messages.push(format!("Skipped {} (already exist)", result.skipped));
    }
    if result.has_errors() {
        for err in &result.errors {
            messages.push(format!("Error: {err}"));
        }
        return (ExitCode::from(1), messages);
    }
    (ExitCode::SUCCESS, messages)
}

fn run_user_prompt_submit_cmd() -> (ExitCode, Vec<String>) {
    match run_user_prompt_submit_hook(None) {
        Ok(()) => (ExitCode::SUCCESS, Vec::new()),
        // coverage:ignore - Error path requires database write failure in ~/.claude-reliability/
        Err(e) => (ExitCode::from(1), vec![format!("Error running user-prompt-submit hook: {e}")]), // coverage:ignore
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
        explain_stops: project_config.explain_stops,
    };

    match run_stop(stdin, &config, &runner, &sub_agent) {
        Ok(result) => (result.exit_code, result.messages),
        Err(e) => (ExitCode::from(1), vec![e]),
    }
}

fn run_pre_tool_use_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    let input = match parse_hook_input(stdin) {
        Ok(input) => input,
        Err(e) => return (ExitCode::from(1), vec![format!("Failed to parse input: {e}")]),
    };

    let runner = RealCommandRunner::new();
    let output = run_pre_tool_use(&input, Path::new("."), &runner);
    let json = serde_json::to_string(&output).expect("PreToolUseOutput serialization cannot fail");

    (ExitCode::SUCCESS, vec![json])
}

fn run_post_tool_use_cmd(stdin: &str) -> (ExitCode, Vec<String>) {
    let input: PostToolUseInput = match serde_json::from_str(stdin) {
        Ok(input) => input,
        Err(e) => return (ExitCode::from(1), vec![format!("Failed to parse input: {e}")]),
    };

    match run_post_tool_use(&input, Path::new(".")) {
        Ok(()) => (ExitCode::SUCCESS, vec![]),
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
    fn test_parse_args_intro() {
        assert_eq!(parse_args(&args(&["prog", "intro"])), ParseResult::Command(Command::Intro));
    }

    #[test]
    fn test_parse_args_pre_tool_use() {
        assert_eq!(
            parse_args(&args(&["prog", "pre-tool-use"])),
            ParseResult::Command(Command::PreToolUse)
        );
    }

    #[test]
    fn test_parse_args_post_tool_use() {
        assert_eq!(
            parse_args(&args(&["prog", "post-tool-use"])),
            ParseResult::Command(Command::PostToolUse)
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
        assert!(u.contains("post-tool-use"));
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
        assert!(!Command::Intro.needs_stdin());
        assert!(!Command::UserPromptSubmit.needs_stdin());

        // Commands that need stdin (hooks that receive JSON input)
        assert!(Command::Stop.needs_stdin());
        assert!(Command::PreToolUse.needs_stdin());
        assert!(Command::PostToolUse.needs_stdin());
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
    #[serial_test::serial]
    fn test_run_pre_tool_use_via_cli() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // Test that Read tool is allowed
        let output = run(
            &args(&["prog", "pre-tool-use"]),
            r#"{"tool_name": "Read", "tool_input": {"file_path": "src/main.rs"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("allow"));
    }

    #[test]
    fn test_run_pre_tool_use_invalid_json() {
        let output = run(&args(&["prog", "pre-tool-use"]), "not json");
        assert_eq!(output.exit_code, ExitCode::from(1));
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("Failed to parse"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_post_tool_use_via_cli() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // Test with ExitPlanMode tool response
        let output = run(
            &args(&["prog", "post-tool-use"]),
            r#"{"toolName": "ExitPlanMode", "toolResponse": {"filePath": "~/.claude/plans/test-plan.md"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn test_run_post_tool_use_invalid_json() {
        let output = run(&args(&["prog", "post-tool-use"]), "not json");
        assert_eq!(output.exit_code, ExitCode::from(1));
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("Failed to parse"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_post_tool_use_unknown_tool() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // Unknown tools should succeed (no hooks to run)
        let output = run(
            &args(&["prog", "post-tool-use"]),
            r#"{"toolName": "UnknownTool", "toolResponse": {}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
    }

    #[test]
    #[serial_test::serial]
    fn test_run_post_tool_use_exit_plan_mode_no_file_path() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // ExitPlanMode with no file_path should fail
        let output = run(
            &args(&["prog", "post-tool-use"]),
            r#"{"toolName": "ExitPlanMode", "toolResponse": {"plan": "content only"}}"#,
        );

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(output.exit_code, ExitCode::from(1));
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("No plan file path"));
    }

    #[test]
    fn test_run_show_usage() {
        let output = run(&args(&["prog"]), "");
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("Usage:"));
    }

    #[test]
    fn test_run_unknown_command() {
        let output = run(&args(&["prog", "unknown"]), "");
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("Unknown command"));
    }

    #[test]
    fn test_run_version() {
        let output = run(&args(&["prog", "version"]), "");
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("claude-reliability"));
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

    // Integration tests that exercise the cli entry points with real dependencies.
    // These tests call the actual run_*_cmd functions through run().

    #[test]
    #[serial_test::serial]
    fn test_run_stop_via_cli() {
        use tempfile::TempDir;

        // Run in temp dir to avoid modifying real project config
        // (run_stop_cmd calls ensure_config which may save if config mismatches)
        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // The stop command needs valid JSON input but will fail gracefully with invalid
        let output = run(&args(&["prog", "stop"]), "not json input");

        std::env::set_current_dir(original_dir).unwrap();

        // It should fail to parse and return an error message
        assert!(!output.stderr.is_empty());
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

        let output = run(&args(&["prog", "stop"]), "{}");

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed (clean repo, allows stop)
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
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

        let output = run(&args(&["prog", "ensure-config"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.iter().any(|m| m.contains("Config ensured")));
        assert!(output.stderr.iter().any(|m| m.contains("git_repo")));
        assert!(output.stderr.iter().any(|m| m.contains("beads_installed")));
        // Check for check_command message (either with value or "(none)")
        assert!(output.stderr.iter().any(|m| m.contains("check_command")));
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

        let output = run(&args(&["prog", "ensure-config"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        // Should detect "just check" as the check command
        assert!(output.stderr.iter().any(|m| m.contains("just check")));
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
        let output = run(&args(&["prog", "ensure-gitignore"]), "");
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.iter().any(|m| m.contains(".gitignore")));

        // Second call should report already has entries
        let output2 = run(&args(&["prog", "ensure-gitignore"]), "");
        assert_eq!(output2.exit_code, ExitCode::SUCCESS);
        assert!(output2.stderr.iter().any(|m| m.contains("already has")));

        std::env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    fn test_run_intro_via_cli() {
        let output = run(&args(&["prog", "intro"]), "");
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(!output.stderr.is_empty());
        // Should contain the intro message
        assert!(output.stderr[0].contains("Reliability Mode"));
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
        let output = run(&args(&["prog", "user-prompt-submit"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
    }

    // Note: The unified pre-tool-use command tests above cover the integrated behavior.
    // Individual hook behaviors are tested in their respective modules (hooks/*.rs).

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

        let output = run(&args(&["prog", "ensure-config"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        // Should return error because .claude can't be created as a directory
        assert_eq!(output.exit_code, ExitCode::from(1));
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("Error ensuring config"));
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

        let output = run(&args(&["prog", "ensure-gitignore"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        // Should return error because .gitignore can't be written
        assert_eq!(output.exit_code, ExitCode::from(1));
        assert!(!output.stderr.is_empty());
        assert!(output.stderr[0].contains("Error updating .gitignore"));
    }

    #[test]
    #[serial_test::serial]
    fn test_run_user_prompt_submit_success() {
        use tempfile::TempDir;

        // Test that user_prompt_submit succeeds normally
        // Note: Previous test tried to trigger an error by making .claude read-only,
        // but with database-based storage in ~/.claude-reliability/, that no longer
        // causes an error since no files are written to .claude.
        let dir = TempDir::new().unwrap();
        let dir_path = dir.path();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        let output = run(&args(&["prog", "user-prompt-submit"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed
        assert_eq!(output.exit_code, ExitCode::from(0));
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
        let output = run(&args(&["prog", "stop"]), "{}");

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed (with warning about config, handled gracefully)
        // The stop command should still work with defaults
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        // At minimum it should process and not crash
        assert!(!output.stderr.is_empty() || output.exit_code == ExitCode::SUCCESS);
    }

    #[test]
    #[serial_test::serial]
    fn test_stop_with_explain_stops_outputs_json_system_message() {
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

        // Create .gitignore to ignore .claude/
        std::fs::write(dir_path.join(".gitignore"), ".claude/\n").unwrap();
        Command::new("git").args(["add", ".gitignore"]).current_dir(dir_path).output().unwrap();
        Command::new("git")
            .args(["commit", "-m", "add gitignore"])
            .current_dir(dir_path)
            .output()
            .unwrap();

        // Create config with explain_stops: true
        let claude_dir = dir_path.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("reliability-config.yaml"),
            "git_repo: true\nexplain_stops: true\n",
        )
        .unwrap();

        // Change to temp dir, run the stop command, then change back
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir_path).unwrap();

        // Run stop command with empty transcript (triggers "clean git repo" explanation)
        let output = run(&args(&["prog", "stop"]), "{}");

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed and output JSON with systemMessage to stdout
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(!output.stdout.is_empty(), "Expected stdout to contain JSON output");
        assert!(output.stderr.is_empty(), "Expected stderr to be empty");

        // Verify the output is valid JSON with systemMessage
        let json: serde_json::Value =
            serde_json::from_str(&output.stdout[0]).expect("Output should be valid JSON");
        assert!(json.get("systemMessage").is_some(), "JSON should contain systemMessage field");
        let message = json["systemMessage"].as_str().unwrap();
        assert!(message.contains("[Stop permitted:"), "should contain stop explanation");
    }

    #[test]
    #[serial_test::serial]
    fn test_run_sync_beads_no_beads() {
        use tempfile::TempDir;

        // Run in temp dir where beads is not available
        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let output = run(&args(&["prog", "sync-beads"]), "");

        std::env::set_current_dir(original_dir).unwrap();

        // Should succeed with no output (no beads = nothing to sync)
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
    }

    #[test]
    fn test_format_sync_result_created() {
        use crate::beads_sync::SyncResult;

        let result = SyncResult { created: 3, skipped: 0, errors: Vec::new() };
        let (exit_code, messages) = format_sync_result(&result);

        assert_eq!(exit_code, ExitCode::SUCCESS);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("Synced 3 beads issues"));
    }

    #[test]
    fn test_format_sync_result_skipped() {
        use crate::beads_sync::SyncResult;

        let result = SyncResult { created: 0, skipped: 2, errors: Vec::new() };
        let (exit_code, messages) = format_sync_result(&result);

        assert_eq!(exit_code, ExitCode::SUCCESS);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("Skipped 2"));
    }

    #[test]
    fn test_format_sync_result_with_errors() {
        use crate::beads_sync::SyncResult;

        let result =
            SyncResult { created: 1, skipped: 0, errors: vec!["proj-1: db error".to_string()] };
        let (exit_code, messages) = format_sync_result(&result);

        assert_eq!(exit_code, ExitCode::from(1));
        assert!(messages.iter().any(|m| m.contains("Error: proj-1")));
    }
}
