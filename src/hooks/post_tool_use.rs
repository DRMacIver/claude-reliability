//! `PostToolUse` hook dispatcher.
//!
//! This module handles hooks that run after a tool completes execution.
//! Currently supports:
//! - `ExitPlanMode`: Creates tasks to track plan implementation

use crate::hooks::plan_tasks::{create_plan_tasks, ExitPlanModeToolResponse};
use std::path::Path;

/// Input provided to `PostToolUse` hooks by Claude Code.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostToolUseInput {
    /// The name of the tool that was executed.
    pub tool_name: Option<String>,
    /// The response from the tool.
    pub tool_response: Option<serde_json::Value>,
}

/// Run all applicable `PostToolUse` hooks for the given input.
///
/// This function dispatches to the appropriate handlers based on `tool_name`.
///
/// # Arguments
///
/// * `input` - The parsed hook input containing `tool_name` and `tool_response`
/// * `base_dir` - The base directory for the project
///
/// # Errors
///
/// Returns an error if the tool response cannot be parsed or if task creation fails.
pub fn run_post_tool_use(input: &PostToolUseInput, base_dir: &Path) -> Result<(), String> {
    let tool_name = input.tool_name.as_deref().unwrap_or("");

    if tool_name == "ExitPlanMode" {
        // Parse ExitPlanMode-specific response
        if let Some(response) = &input.tool_response {
            let exit_response: ExitPlanModeToolResponse = serde_json::from_value(response.clone())
                .map_err(|e| format!("Failed to parse ExitPlanMode response: {e}"))?;
            create_plan_tasks(&exit_response, base_dir)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths;
    use crate::tasks::{SqliteTaskStore, TaskFilter, TaskStore};
    use tempfile::TempDir;

    fn setup_db(dir: &Path) {
        let db_path = paths::project_db_path(dir);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn test_run_post_tool_use_exit_plan_mode() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let input = PostToolUseInput {
            tool_name: Some("ExitPlanMode".to_string()),
            tool_response: Some(serde_json::json!({
                "filePath": "~/.claude/plans/test-plan.md"
            })),
        };

        let result = run_post_tool_use(&input, dir.path());
        assert!(result.is_ok());

        // Verify tasks were created
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 2);
    }

    #[test]
    fn test_run_post_tool_use_unknown_tool() {
        let dir = TempDir::new().unwrap();

        let input = PostToolUseInput {
            tool_name: Some("UnknownTool".to_string()),
            tool_response: Some(serde_json::json!({"foo": "bar"})),
        };

        // Should succeed (no hooks for unknown tools)
        let result = run_post_tool_use(&input, dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_post_tool_use_no_tool_name() {
        let dir = TempDir::new().unwrap();

        let input = PostToolUseInput { tool_name: None, tool_response: None };

        // Should succeed (no tool name means nothing to do)
        let result = run_post_tool_use(&input, dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_post_tool_use_exit_plan_mode_no_response() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let input =
            PostToolUseInput { tool_name: Some("ExitPlanMode".to_string()), tool_response: None };

        // Should succeed (no response means nothing to process)
        let result = run_post_tool_use(&input, dir.path());
        assert!(result.is_ok());

        // Verify no tasks were created
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 0);
    }

    #[test]
    fn test_run_post_tool_use_exit_plan_mode_invalid_response() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let input = PostToolUseInput {
            tool_name: Some("ExitPlanMode".to_string()),
            // Missing filePath - this will cause create_plan_tasks to fail
            tool_response: Some(serde_json::json!({"plan": "content only"})),
        };

        let result = run_post_tool_use(&input, dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No plan file path"));
    }
}
