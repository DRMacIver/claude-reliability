//! Hook to require a task be in progress before making code changes.
//!
//! This hook blocks Write and Edit operations when no task is marked as in-progress,
//! encouraging the use of task tracking for all code modifications.

use crate::hooks::{HookInput, PreToolUseOutput};
use crate::tasks::{SqliteTaskStore, TaskStore};
use crate::templates;
use std::path::Path;
use tera::Context;

/// Run the require task `PreToolUse` hook.
///
/// Blocks Write and Edit operations when no task is in progress.
///
/// # Panics
///
/// Panics if embedded templates fail to render.
pub fn run_require_task_hook(input: &HookInput, base_dir: &Path) -> PreToolUseOutput {
    let tool_name = input.tool_name.as_deref().unwrap_or("");

    // Only check Write and Edit tools
    if tool_name != "Write" && tool_name != "Edit" {
        return PreToolUseOutput::allow(None);
    }

    // Open the task store - panic on database errors since something is seriously wrong
    let store = SqliteTaskStore::for_project(base_dir).expect("task store should be accessible");

    if store.has_in_progress_task().expect("task store query should succeed") {
        PreToolUseOutput::allow(None)
    } else {
        let message = templates::render("messages/require_task.tera", &Context::new())
            .expect("require_task.tera template should always render");
        PreToolUseOutput::block(Some(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ToolInput;
    use crate::paths;
    use crate::tasks::{Priority, TaskStore, TaskUpdate};
    use tempfile::TempDir;

    fn setup_test_store(base: &Path) -> SqliteTaskStore {
        let db_path = paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        SqliteTaskStore::new(&db_path).unwrap()
    }

    #[test]
    fn test_write_blocked_without_in_progress_task() {
        let dir = TempDir::new().unwrap();
        let _store = setup_test_store(dir.path());

        let input = HookInput {
            tool_name: Some("Write".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_require_task_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
    }

    #[test]
    fn test_edit_blocked_without_in_progress_task() {
        let dir = TempDir::new().unwrap();
        let _store = setup_test_store(dir.path());

        let input = HookInput {
            tool_name: Some("Edit".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_require_task_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("block"));
    }

    #[test]
    fn test_write_allowed_with_in_progress_task() {
        let dir = TempDir::new().unwrap();
        let store = setup_test_store(dir.path());

        // Create a task and mark it in progress
        let task = store.create_task("Test task", "Description", Priority::Medium).unwrap();
        store
            .update_task(&task.id, TaskUpdate { in_progress: Some(true), ..Default::default() })
            .unwrap();

        let input = HookInput {
            tool_name: Some("Write".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_require_task_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
    }

    #[test]
    fn test_other_tools_always_allowed() {
        let dir = TempDir::new().unwrap();
        let _store = setup_test_store(dir.path());

        // Bash should be allowed even without in-progress task
        let input = HookInput {
            tool_name: Some("Bash".to_string()),
            tool_input: Some(ToolInput { command: Some("ls".to_string()), ..Default::default() }),
            ..Default::default()
        };

        let output = run_require_task_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
    }

    #[test]
    fn test_read_always_allowed() {
        let dir = TempDir::new().unwrap();
        let _store = setup_test_store(dir.path());

        let input = HookInput {
            tool_name: Some("Read".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_require_task_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("allow"));
    }

    #[test]
    fn test_blocked_when_no_tasks_exist() {
        let dir = TempDir::new().unwrap();
        // Store is auto-created, but no tasks exist and none are in progress

        let input = HookInput {
            tool_name: Some("Write".to_string()),
            tool_input: Some(ToolInput {
                file_path: Some("src/main.rs".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let output = run_require_task_hook(&input, dir.path());
        let json = serde_json::to_string(&output).unwrap();
        // Should block - agent must create and work_on a task first
        assert!(json.contains("block"));
    }
}
