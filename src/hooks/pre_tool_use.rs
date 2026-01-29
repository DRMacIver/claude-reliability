//! Unified `PreToolUse` hook dispatcher.
//!
//! This module consolidates all `PreToolUse` hook logic into a single entry point,
//! dispatching to appropriate handlers based on tool name.

use crate::hooks::{
    plan_tasks, run_code_review_hook, run_problem_mode_hook, run_protect_config_hook,
    run_require_task_hook, run_validation_hook, CodeReviewConfig, HookInput, PreToolUseOutput,
};
use crate::reminders;
use crate::subagent::RealSubAgent;
use crate::templates;
use crate::traits::{CommandRunner, SubAgent};
use crate::transcript;
use std::path::Path;
use tera::Context;

/// Run all applicable `PreToolUse` hooks for the given input.
///
/// This function dispatches to the appropriate handlers based on `tool_name`.
/// Hooks are evaluated in order and the function returns early on the first block.
///
/// # Arguments
///
/// * `input` - The parsed hook input containing `tool_name` and `tool_input`
/// * `base_dir` - The base directory for the project
/// * `runner` - Command runner for executing external commands
///
/// # Returns
///
/// A `PreToolUseOutput` that either allows or blocks the operation.
pub fn run_pre_tool_use(
    input: &HookInput,
    base_dir: &Path,
    runner: &dyn CommandRunner,
) -> PreToolUseOutput {
    let sub_agent = RealSubAgent::new(runner);
    run_pre_tool_use_with_sub_agent(input, base_dir, runner, &sub_agent)
}

/// Run all applicable `PreToolUse` hooks with a provided sub-agent.
///
/// This is the testable version that accepts a `SubAgent` trait.
pub fn run_pre_tool_use_with_sub_agent(
    input: &HookInput,
    base_dir: &Path,
    runner: &dyn CommandRunner,
    sub_agent: &dyn SubAgent,
) -> PreToolUseOutput {
    let tool_name = input.tool_name.as_deref().unwrap_or("");

    // Helper macro to return early if result is a block
    macro_rules! check_hook {
        ($result:expr) => {
            let result = $result;
            if result.is_block() {
                return result;
            }
        };
    }

    // Problem mode check - applies to all tools
    check_hook!(run_problem_mode_hook(input, base_dir));

    // Tool-specific hooks
    match tool_name {
        "Bash" => {
            // Check for --no-verify
            check_hook!(run_no_verify_check(input));

            // Code review for git commits
            let config = CodeReviewConfig::default();
            if let Ok(exit_code) = run_code_review_hook(input, &config, runner, sub_agent) {
                if exit_code != 0 {
                    return PreToolUseOutput::block(Some(
                        "Code review required before commit".to_string(),
                    ));
                }
            }
        }

        "Write" | "Edit" => {
            // Validation tracking (doesn't block, just tracks)
            check_hook!(run_validation_hook(input, base_dir));

            // Require task in progress
            check_hook!(run_require_task_hook(input, base_dir));

            // Protect config files
            check_hook!(run_protect_config_hook(input));
        }

        "NotebookEdit" => {
            // Validation tracking
            check_hook!(run_validation_hook(input, base_dir));
        }

        "EnterPlanMode" => {
            // Inject user intent guidance before planning
            return handle_enter_plan_mode();
        }

        "ExitPlanMode" => {
            // Create tasks for the plan being approved
            // This runs at PreToolUse because PostToolUse doesn't fire for ExitPlanMode
            handle_exit_plan_mode(base_dir, None);
        }

        _ => {
            // No additional hooks for other tools
        }
    }

    // All hooks passed - check for reminders
    let context = get_reminder_context(input, base_dir);
    PreToolUseOutput::allow(context)
}

/// Get reminder context from the last assistant output in the transcript.
///
/// Returns `None` if:
/// - No transcript path is provided
/// - The transcript cannot be parsed
/// - No assistant output exists
/// - No reminders match
fn get_reminder_context(input: &HookInput, base_dir: &Path) -> Option<String> {
    let transcript_path = input.transcript_path.as_ref()?;
    let info = transcript::parse_transcript(Path::new(transcript_path)).ok()?;
    let assistant_output = info.last_assistant_output.as_ref()?;

    let reminder_messages = reminders::check_reminders(assistant_output, base_dir);
    if reminder_messages.is_empty() {
        return None;
    }

    Some(reminder_messages.join("\n\n"))
}

/// Handle `EnterPlanMode` by injecting user intent guidance.
///
/// This injects guidance about understanding user intent as additional context,
/// ensuring the agent considers the user's actual intent before planning.
///
/// # Panics
///
/// Panics if the embedded template fails to render. Templates are verified by
/// `test_all_embedded_templates_render`, so this should only occur if a template
/// has a bug that escaped tests.
fn handle_enter_plan_mode() -> PreToolUseOutput {
    let ctx = Context::new();
    let context = templates::render("messages/enter_plan_mode_intent.tera", &ctx)
        .expect("enter_plan_mode_intent.tera template should always render");
    PreToolUseOutput::allow(Some(context))
}

/// Handle `ExitPlanMode` by creating tasks from the plan file.
///
/// # Arguments
/// * `base_dir` - The project base directory for the task store
/// * `plans_dir_override` - Optional override for the plans directory (for testing)
fn handle_exit_plan_mode(base_dir: &Path, plans_dir_override: Option<&Path>) {
    let result = plans_dir_override.map_or_else(
        || plan_tasks::create_plan_tasks_from_recent(base_dir),
        |dir| plan_tasks::create_plan_tasks_from_dir(dir, base_dir),
    );

    if let Err(e) = result {
        // Log but don't block - plan approval should continue
        eprintln!("Warning: Failed to create plan tasks: {e}");
    }
}

/// Check for --no-verify flag in git commands.
fn run_no_verify_check(input: &HookInput) -> PreToolUseOutput {
    let command = input.tool_input.as_ref().and_then(|ti| ti.command.as_deref()).unwrap_or("");

    // Check for --no-verify or -n flag (only for git commit)
    let is_git_commit = command.contains("git commit") || command.contains("git push");
    let has_no_verify = command.contains("--no-verify");

    if is_git_commit && has_no_verify {
        let mut ctx = Context::new();
        ctx.insert("acknowledgment", "I promise the user has said I can use --no-verify here");
        let message = templates::render("messages/no_verify_block.tera", &ctx)
            .expect("no_verify_block.tera template should always render");
        PreToolUseOutput::block(Some(message))
    } else {
        PreToolUseOutput::allow(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ToolInput;
    use crate::testing::MockCommandRunner;
    use tempfile::TempDir;

    #[test]
    fn test_bash_allowed_without_no_verify() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("git status".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(!output.is_block());
    }

    #[test]
    fn test_bash_blocked_with_no_verify() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("git commit --no-verify -m test".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(output.is_block());
    }

    #[test]
    fn test_read_always_allowed() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        let input = HookInput {
            tool_name: Some("Read".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(!output.is_block());
    }

    #[test]
    fn test_write_blocked_without_in_progress_task() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        let input = HookInput {
            tool_name: Some("Write".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        // Should be blocked by require_task hook
        assert!(output.is_block());
    }

    #[test]
    fn test_unknown_tool_allowed() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        let input = HookInput {
            tool_name: Some("UnknownTool".to_string()),
            tool_input: None,
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(!output.is_block());
    }

    #[test]
    fn test_no_verify_check_allows_regular_commits() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("git commit -m 'test'".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_no_verify_check(&input);
        assert!(!output.is_block());
    }

    #[test]
    fn test_no_verify_check_blocks_no_verify() {
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("git commit --no-verify -m 'test'".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_no_verify_check(&input);
        assert!(output.is_block());
    }

    #[test]
    fn test_write_blocks_config_file_with_in_progress_task() {
        use crate::paths;
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore, TaskUpdate};

        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        // Set up task store with in-progress task
        let db_path = paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::new(&db_path).unwrap();
        let task = store.create_task("Test task", "Description", Priority::Medium).unwrap();
        store
            .update_task(&task.id, TaskUpdate { in_progress: Some(true), ..Default::default() })
            .unwrap();

        // Try to write to config file - should be blocked by protect_config
        let input = HookInput {
            tool_name: Some("Write".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some(".claude/reliability-config.yaml".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(output.is_block());
    }

    #[test]
    fn test_notebook_edit_tracks_validation() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        let input = HookInput {
            tool_name: Some("NotebookEdit".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("notebook.ipynb".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        // Should be allowed (validation just tracks, doesn't block)
        assert!(!output.is_block());
        // Check that validation marker was set
        assert!(crate::session::needs_validation(dir.path()));
    }

    #[test]
    fn test_bash_blocked_by_code_review_rejection() {
        use crate::testing::MockSubAgent;
        use crate::traits::CommandOutput;

        let dir = TempDir::new().unwrap();
        let mut runner = MockCommandRunner::new();

        // Set up expectations for git diff commands used by code review
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only"],
            CommandOutput {
                exit_code: 0,
                stdout: "src/main.rs\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "-U0"],
            CommandOutput {
                exit_code: 0,
                stdout: "+fn main() {}\n".to_string(),
                stderr: String::new(),
            },
        );

        // Sub-agent rejects the code review
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_review(false, "Security issue found");

        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput {
                command: Some("git commit -m 'test'".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use_with_sub_agent(&input, dir.path(), &runner, &sub_agent);
        assert!(output.is_block());
        assert!(output
            .hook_specific_output
            .additional_context
            .as_ref()
            .unwrap()
            .contains("Code review required"));
    }

    #[test]
    fn test_enter_plan_mode_includes_intent_context() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        let input = HookInput {
            tool_name: Some("EnterPlanMode".to_string()),
            tool_input: None,
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(!output.is_block());
        let context = output
            .hook_specific_output
            .additional_context
            .expect("EnterPlanMode should include additional context");
        assert!(
            context.contains("Load-Bearing Feature"),
            "Intent context should mention load-bearing features"
        );
        assert!(
            context.contains("Sanity Check"),
            "Intent context should include sanity check guidance"
        );
        assert!(
            context.contains("understanding-user-intent"),
            "Intent context should reference the full skill"
        );
    }

    #[test]
    fn test_exit_plan_mode_allowed() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        // Set up the task store (needed for potential task creation)
        let db_path = crate::paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        let input = HookInput {
            tool_name: Some("ExitPlanMode".to_string()),
            tool_input: None,
            ..Default::default()
        };

        // ExitPlanMode should always be allowed (task creation failures are logged, not blocking)
        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(!output.is_block());
    }

    #[test]
    fn test_handle_exit_plan_mode_with_plan_file() {
        use crate::tasks::{SqliteTaskStore, TaskFilter, TaskStore};
        use std::fs::FileTimes;
        use std::time::{Duration, SystemTime};

        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        // Create a plan file
        let plan_file = plans_dir.join("test-plan.md");
        std::fs::write(&plan_file, "# Test Plan").unwrap();

        // Set explicit mtime
        let now = SystemTime::now();
        std::fs::File::open(&plan_file)
            .unwrap()
            .set_times(FileTimes::new().set_modified(now - Duration::from_secs(10)))
            .unwrap();

        // Set up task store
        let db_path = crate::paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // This should create tasks (success path)
        handle_exit_plan_mode(dir.path(), Some(&plans_dir));

        // Verify tasks were created
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_handle_exit_plan_mode_empty_plans_dir() {
        use crate::tasks::{SqliteTaskStore, TaskFilter, TaskStore};

        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join("empty_plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        // Set up task store
        let db_path = crate::paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // This should log a warning (error path) but not panic
        handle_exit_plan_mode(dir.path(), Some(&plans_dir));

        // Verify no tasks were created
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 0);
    }

    #[test]
    fn test_pre_tool_use_includes_reminder_context() {
        use crate::paths::project_data_dir;
        use std::io::Write;

        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        // Create reminders.yaml
        let data_dir = project_data_dir(dir.path());
        std::fs::create_dir_all(&data_dir).unwrap();
        let reminders_path = data_dir.join("reminders.yaml");
        let mut file = std::fs::File::create(&reminders_path).unwrap();
        file.write_all(
            br#"
reminders:
  - message: "Reminder: Handle edge cases carefully"
    patterns:
      - "edge case"
"#,
        )
        .unwrap();

        // Create a transcript file with assistant output that matches
        let transcript_path = dir.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "We should consider this edge case"}]}}
"#,
        )
        .unwrap();

        // Clear reminder cache to ensure fresh load
        crate::reminders::clear_cache();

        let input = HookInput {
            tool_name: Some("Read".to_string()),
            tool_input: None,
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(!output.is_block());
        let context = output.hook_specific_output.additional_context.unwrap();
        assert!(context.contains("Handle edge cases carefully"));
    }

    #[test]
    fn test_pre_tool_use_no_reminders_when_no_match() {
        use crate::paths::project_data_dir;
        use std::io::Write;

        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        // Create reminders.yaml
        let data_dir = project_data_dir(dir.path());
        std::fs::create_dir_all(&data_dir).unwrap();
        let reminders_path = data_dir.join("reminders.yaml");
        let mut file = std::fs::File::create(&reminders_path).unwrap();
        file.write_all(
            br#"
reminders:
  - message: "Handle edge cases"
    patterns:
      - "edge case"
"#,
        )
        .unwrap();

        // Create a transcript file with assistant output that doesn't match
        let transcript_path = dir.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "This is a normal response"}]}}
"#,
        )
        .unwrap();

        // Clear reminder cache
        crate::reminders::clear_cache();

        let input = HookInput {
            tool_name: Some("Read".to_string()),
            tool_input: None,
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        assert!(!output.is_block());
        assert!(output.hook_specific_output.additional_context.is_none());
    }

    #[test]
    fn test_get_reminder_context_no_transcript() {
        let dir = TempDir::new().unwrap();

        let input = HookInput {
            tool_name: Some("Read".to_string()),
            tool_input: None,
            transcript_path: None,
        };

        let context = get_reminder_context(&input, dir.path());
        assert!(context.is_none());
    }

    #[test]
    fn test_get_reminder_context_invalid_transcript() {
        let dir = TempDir::new().unwrap();

        let input = HookInput {
            tool_name: Some("Read".to_string()),
            tool_input: None,
            transcript_path: Some("/nonexistent/transcript.jsonl".to_string()),
        };

        let context = get_reminder_context(&input, dir.path());
        assert!(context.is_none());
    }

    #[test]
    fn test_get_reminder_context_no_assistant_output() {
        let dir = TempDir::new().unwrap();

        // Create a transcript file with only user message
        let transcript_path = dir.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z"}
"#,
        )
        .unwrap();

        let input = HookInput {
            tool_name: Some("Read".to_string()),
            tool_input: None,
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
        };

        let context = get_reminder_context(&input, dir.path());
        assert!(context.is_none());
    }
}
