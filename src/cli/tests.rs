//! Tests for the CLI module.

use super::*;
use crate::paths;
use std::process::ExitCode;
use tempfile::TempDir;

/// Create a fake command script in a temp directory and return a guard that
/// prepends it to PATH. When the guard is dropped, PATH is restored.
struct FakeCommandGuard {
    original_path: String,
}

impl FakeCommandGuard {
    /// Create a fake command that exits with code 0.
    fn new(bin_dir: &std::path::Path, command_name: &str) -> Self {
        let script_path = bin_dir.join(command_name);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&script_path, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(not(unix))]
        {
            // On non-Unix, create a .bat file
            std::fs::write(bin_dir.join(format!("{command_name}.bat")), "@echo off\nexit /b 0\n")
                .unwrap();
        }

        let original_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", bin_dir.display(), original_path);
        std::env::set_var("PATH", &new_path);

        Self { original_path }
    }
}

impl Drop for FakeCommandGuard {
    fn drop(&mut self) {
        std::env::set_var("PATH", &self.original_path);
    }
}

#[test]
fn test_command_needs_stdin() {
    // Commands that don't need stdin (won't block on terminal)
    assert!(!Command::Version.needs_stdin());
    assert!(!Command::EnsureConfig.needs_stdin());
    assert!(!Command::EnsureGitignore.needs_stdin());
    assert!(!Command::Intro.needs_stdin());

    // Commands that need stdin (hooks that receive JSON input)
    assert!(Command::Stop.needs_stdin());
    assert!(Command::PreToolUse.needs_stdin());
    assert!(Command::PostToolUse.needs_stdin());
    assert!(Command::UserPromptSubmit.needs_stdin());

    // Work commands don't need stdin
    assert!(!Command::Work(WorkCommand::Next).needs_stdin());
}

#[test]
fn test_command_hook_type() {
    // Hook commands return their type name
    assert_eq!(Command::Stop.hook_type(), Some("stop"));
    assert_eq!(Command::UserPromptSubmit.hook_type(), Some("user-prompt-submit"));
    assert_eq!(Command::PreToolUse.hook_type(), Some("pre-tool-use"));
    assert_eq!(Command::PostToolUse.hook_type(), Some("post-tool-use"));

    // Non-hook commands return None
    assert_eq!(Command::Version.hook_type(), None);
    assert_eq!(Command::EnsureConfig.hook_type(), None);
    assert_eq!(Command::EnsureGitignore.hook_type(), None);
    assert_eq!(Command::Intro.hook_type(), None);
    assert_eq!(Command::Work(WorkCommand::Next).hook_type(), None);
}

#[test]
fn test_run_version() {
    let output = run(Command::Version, "");
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stderr.is_empty());
    assert!(output.stderr[0].contains("claude-reliability"));
}

#[test]
fn test_run_intro() {
    let output = run(Command::Intro, "");
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stderr.is_empty());
    assert!(output.stderr[0].contains("Reliability Mode"));
}

#[test]
#[serial_test::serial]
fn test_run_pre_tool_use_via_cli() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let output = run(
        Command::PreToolUse,
        r#"{"tool_name": "Read", "tool_input": {"file_path": "src/main.rs"}}"#,
    );

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stderr.is_empty());
    assert!(output.stderr[0].contains("allow"));
}

#[test]
fn test_run_pre_tool_use_invalid_json() {
    let output = run(Command::PreToolUse, "not json");
    assert_eq!(output.exit_code, ExitCode::from(1));
    assert!(!output.stderr.is_empty());
    assert!(output.stderr[0].contains("Failed to parse"));
}

#[test]
#[serial_test::serial]
fn test_run_post_tool_use_via_cli() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    // Set up the database directory (required for task creation)
    let db_path = paths::project_db_path(dir.path());
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    std::env::set_current_dir(dir.path()).unwrap();

    let output = run(
        Command::PostToolUse,
        r#"{"toolName": "ExitPlanMode", "toolResponse": {"filePath": "~/.claude/plans/test-plan.md"}}"#,
    );

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
}

#[test]
fn test_run_post_tool_use_invalid_json() {
    let output = run(Command::PostToolUse, "not json");
    assert_eq!(output.exit_code, ExitCode::from(1));
    assert!(!output.stderr.is_empty());
    assert!(output.stderr[0].contains("Failed to parse"));
}

#[test]
#[serial_test::serial]
fn test_run_post_tool_use_unknown_tool() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let output = run(Command::PostToolUse, r#"{"toolName": "UnknownTool", "toolResponse": {}}"#);

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
}

#[test]
#[serial_test::serial]
fn test_run_post_tool_use_exit_plan_mode_no_file_path() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let output = run(
        Command::PostToolUse,
        r#"{"toolName": "ExitPlanMode", "toolResponse": {"plan": "content only"}}"#,
    );

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::from(1));
    assert!(!output.stderr.is_empty());
    assert!(output.stderr[0].contains("No plan file path"));
}

#[test]
#[serial_test::serial]
fn test_run_stop_via_cli() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let output = run(Command::Stop, "not json input");

    std::env::set_current_dir(original_dir).unwrap();

    assert!(!output.stderr.is_empty());
}

#[test]
#[serial_test::serial]
fn test_run_stop_via_cli_in_temp_repo() {
    use std::process::Command as StdCommand;

    let dir = TempDir::new().unwrap();
    let dir_path = dir.path();

    // Initialize git repo
    StdCommand::new("git").args(["init"]).current_dir(dir_path).output().unwrap();
    StdCommand::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir_path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir_path)
        .output()
        .unwrap();

    // Create initial commit
    std::fs::write(dir_path.join("README.md"), "test").unwrap();
    StdCommand::new("git").args(["add", "."]).current_dir(dir_path).output().unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(dir_path)
        .output()
        .unwrap();

    // Create .gitignore
    std::fs::write(dir_path.join(".gitignore"), ".claude/\n.claude-reliability/\n").unwrap();
    StdCommand::new("git").args(["add", ".gitignore"]).current_dir(dir_path).output().unwrap();
    StdCommand::new("git")
        .args(["commit", "-m", "add gitignore"])
        .current_dir(dir_path)
        .output()
        .unwrap();

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir_path).unwrap();

    let output = run(Command::Stop, "{}");

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
}

#[test]
#[serial_test::serial]
fn test_run_ensure_config_via_cli() {
    use std::process::Command as StdCommand;

    let dir = TempDir::new().unwrap();
    let dir_path = dir.path();

    StdCommand::new("git").args(["init"]).current_dir(dir_path).output().unwrap();

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir_path).unwrap();

    let output = run(Command::EnsureConfig, "");

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.iter().any(|m| m.contains("Config ensured")));
    assert!(output.stderr.iter().any(|m| m.contains("git_repo")));
    assert!(output.stderr.iter().any(|m| m.contains("check_command")));
}

#[test]
#[serial_test::serial]
fn test_run_ensure_config_with_justfile() {
    use std::process::Command as StdCommand;

    let dir = TempDir::new().unwrap();
    let dir_path = dir.path();
    let bin_dir = TempDir::new().unwrap();
    let _fake_just = FakeCommandGuard::new(bin_dir.path(), "just");

    StdCommand::new("git").args(["init"]).current_dir(dir_path).output().unwrap();
    std::fs::write(dir_path.join("justfile"), "check:\n\techo test\n").unwrap();

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir_path).unwrap();

    let output = run(Command::EnsureConfig, "");

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.iter().any(|m| m.contains("just check")));
}

#[test]
#[serial_test::serial]
fn test_run_ensure_gitignore_via_cli() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path();

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir_path).unwrap();

    let output = run(Command::EnsureGitignore, "");
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.iter().any(|m| m.contains(".gitignore")));

    let output2 = run(Command::EnsureGitignore, "");
    assert_eq!(output2.exit_code, ExitCode::SUCCESS);
    assert!(output2.stderr.iter().any(|m| m.contains("already has")));

    std::env::set_current_dir(original_dir).unwrap();
}

#[test]
#[serial_test::serial]
fn test_run_user_prompt_submit_via_cli() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path();

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir_path).unwrap();

    let output = run(Command::UserPromptSubmit, "");

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
}

#[test]
#[serial_test::serial]
fn test_run_user_prompt_submit_post_compaction() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path();

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir_path).unwrap();

    let input = r#"{"isCompactSummary": true}"#;
    let output = run(Command::UserPromptSubmit, input);

    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(output.stdout.len(), 1);
    assert!(output.stdout[0].contains("systemMessage"));
    assert!(output.stdout[0].contains("Post-Compaction"));
}

// === Work command tests ===

#[test]
#[serial_test::serial]
fn test_work_create_and_get() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    // Create a work item
    let output = run(
        Command::Work(WorkCommand::Create {
            title: "Test task".to_string(),
            description: "Test description".to_string(),
            priority: 1,
        }),
        "",
    );

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stdout.is_empty());

    // Parse the created item's ID
    let created: serde_json::Value = serde_json::from_str(&output.stdout[0]).unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    // Get the work item
    let output = run(Command::Work(WorkCommand::Get { id }), "");

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    let fetched: serde_json::Value = serde_json::from_str(&output.stdout[0]).unwrap();
    assert_eq!(fetched["title"], "Test task");
    assert_eq!(fetched["description"], "Test description");
    assert_eq!(fetched["priority"], 1);

    std::env::set_current_dir(original_dir).unwrap();
}

#[test]
#[serial_test::serial]
fn test_work_list_and_next() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    // Create work items
    run(
        Command::Work(WorkCommand::Create {
            title: "High priority".to_string(),
            description: String::new(),
            priority: 1,
        }),
        "",
    );
    run(
        Command::Work(WorkCommand::Create {
            title: "Low priority".to_string(),
            description: String::new(),
            priority: 3,
        }),
        "",
    );

    // List all
    let output = run(
        Command::Work(WorkCommand::List {
            status: None,
            priority: None,
            max_priority: None,
            ready_only: false,
            limit: None,
            offset: None,
        }),
        "",
    );

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    let items: Vec<serde_json::Value> = serde_json::from_str(&output.stdout[0]).unwrap();
    assert_eq!(items.len(), 2);

    // Get next should suggest high priority item
    let output = run(Command::Work(WorkCommand::Next), "");

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    let suggestion: serde_json::Value = serde_json::from_str(&output.stdout[0]).unwrap();
    assert_eq!(suggestion["priority"], 1);

    std::env::set_current_dir(original_dir).unwrap();
}

#[test]
#[serial_test::serial]
fn test_work_update_and_delete() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    // Create
    let output = run(
        Command::Work(WorkCommand::Create {
            title: "Original".to_string(),
            description: String::new(),
            priority: 2,
        }),
        "",
    );

    let created: serde_json::Value = serde_json::from_str(&output.stdout[0]).unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    // Update
    let output = run(
        Command::Work(WorkCommand::Update {
            id: id.clone(),
            title: Some("Updated".to_string()),
            description: None,
            priority: Some(0),
            status: None,
        }),
        "",
    );

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    let updated: serde_json::Value = serde_json::from_str(&output.stdout[0]).unwrap();
    assert_eq!(updated["title"], "Updated");
    assert_eq!(updated["priority"], 0);

    // Delete
    let output = run(Command::Work(WorkCommand::Delete { id }), "");

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout[0].contains("deleted"));

    std::env::set_current_dir(original_dir).unwrap();
}

// === HowTo command tests ===

#[test]
#[serial_test::serial]
fn test_howto_crud() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    // Create
    let output = run(
        Command::Howto(HowToCommand::Create {
            title: "Test Guide".to_string(),
            instructions: "Step 1: Do the thing".to_string(),
        }),
        "",
    );

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    let created: serde_json::Value = serde_json::from_str(&output.stdout[0]).unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    // List
    let output = run(Command::Howto(HowToCommand::List), "");
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    let items: Vec<serde_json::Value> = serde_json::from_str(&output.stdout[0]).unwrap();
    assert_eq!(items.len(), 1);

    // Search
    let output =
        run(Command::Howto(HowToCommand::Search { query: "Guide".to_string(), limit: None }), "");
    assert_eq!(output.exit_code, ExitCode::SUCCESS);

    // Delete
    let output = run(Command::Howto(HowToCommand::Delete { id }), "");
    assert_eq!(output.exit_code, ExitCode::SUCCESS);

    std::env::set_current_dir(original_dir).unwrap();
}

// === Question command tests ===

#[test]
#[serial_test::serial]
fn test_question_crud() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    // Create (note: may be auto-answered by sub-agent in real use)
    let output = run(
        Command::Question(QuestionCommand::Create {
            text: "What color should the button be?".to_string(),
        }),
        "",
    );

    // Either creates question or auto-answers
    assert_eq!(output.exit_code, ExitCode::SUCCESS);

    // List questions
    let output =
        run(Command::Question(QuestionCommand::List { unanswered_only: false, limit: None }), "");
    assert_eq!(output.exit_code, ExitCode::SUCCESS);

    std::env::set_current_dir(original_dir).unwrap();
}

// === Audit log test ===

#[test]
#[serial_test::serial]
fn test_audit_log() {
    let dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    // Create a work item to generate audit log entries
    run(
        Command::Work(WorkCommand::Create {
            title: "Audit test".to_string(),
            description: String::new(),
            priority: 2,
        }),
        "",
    );

    // Get audit log
    let output = run(Command::AuditLog { work_id: None, limit: None }, "");

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    let entries: Vec<serde_json::Value> = serde_json::from_str(&output.stdout[0]).unwrap();
    assert!(!entries.is_empty());

    std::env::set_current_dir(original_dir).unwrap();
}
