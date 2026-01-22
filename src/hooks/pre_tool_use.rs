//! Unified `PreToolUse` hook dispatcher.
//!
//! This module consolidates all `PreToolUse` hook logic into a single entry point,
//! dispatching to appropriate handlers based on tool name.

use crate::hooks::{
    run_code_review_hook, run_jkw_setup_hook, run_problem_mode_hook, run_protect_config_hook,
    run_require_task_hook, run_validation_hook, CodeReviewConfig, HookInput, PreToolUseOutput,
};
use crate::subagent::RealSubAgent;
use crate::templates;
use crate::traits::CommandRunner;
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
            // The sub-agent calls out to claude CLI, tested via code_review module
            let sub_agent = RealSubAgent::new(runner);
            let config = CodeReviewConfig::default();
            if let Ok(exit_code) = run_code_review_hook(input, &config, runner, &sub_agent) {
                if exit_code != 0 {
                    // coverage:ignore - requires real claude CLI; code_review.rs tests rejection path
                    #[rustfmt::skip]
                    return PreToolUseOutput::block(Some( // coverage:ignore
                        "Code review required before commit".to_string())); // coverage:ignore
                }
            }
        }

        "Write" | "Edit" => {
            // Validation tracking (doesn't block, just tracks)
            check_hook!(run_validation_hook(input, base_dir));

            // JKW setup enforcement
            check_hook!(run_jkw_setup_hook(input, base_dir));

            // Require task in progress
            check_hook!(run_require_task_hook(input, base_dir));

            // Protect config files
            check_hook!(run_protect_config_hook(input));
        }

        "NotebookEdit" => {
            // Validation tracking
            check_hook!(run_validation_hook(input, base_dir));
        }

        "Skill" => {
            // JKW setup - sets marker if JKW invoked, always allows Skill itself
            let _ = run_jkw_setup_hook(input, base_dir);
        }

        _ => {
            // No additional hooks for other tools
        }
    }

    // All hooks passed
    PreToolUseOutput::allow(None)
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
        let db_path = paths::project_db_path(dir.path()).expect("should have home dir");
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
    fn test_edit_blocked_by_jkw_setup() {
        use crate::session::set_jkw_setup_required;

        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        // Set JKW setup required marker
        set_jkw_setup_required(dir.path()).unwrap();

        // Edit to non-session file should be blocked
        let input = HookInput {
            tool_name: Some("Edit".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        // Should be blocked by jkw_setup hook (only Write/Edit blocked, not Skill)
        assert!(output.is_block());
    }

    #[test]
    fn test_skill_jkw_always_allowed() {
        use crate::session::set_jkw_setup_required;

        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();

        // Set JKW setup required marker
        set_jkw_setup_required(dir.path()).unwrap();

        let input = HookInput {
            tool_name: Some("Skill".to_string()),
            tool_input: Some(ToolInput {
                skill: Some("just-keep-working".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_pre_tool_use(&input, dir.path(), &runner);
        // Skill tools are always allowed (they set the marker, don't get blocked by it)
        assert!(!output.is_block());
    }
}
