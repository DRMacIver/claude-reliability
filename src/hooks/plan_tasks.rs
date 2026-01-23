//! Create tasks when a plan is approved via `ExitPlanMode`.

use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
use std::path::Path;

/// Tool response from `ExitPlanMode`.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitPlanModeToolResponse {
    /// Path to the plan file (e.g., `~/.claude/plans/enchanted-wondering-valiant.md`).
    pub file_path: Option<String>,
    /// Direct plan content (alternative to `file_path`).
    /// Currently unused but part of the API spec for future use.
    #[allow(dead_code)]
    pub plan: Option<String>,
}

/// Create tasks for an approved plan.
///
/// Creates two tasks:
/// 1. "Break up plan: <plan-name>" - Parse the plan and create individual tasks
/// 2. "Implement plan: <plan-name>" - Complete all work described in the plan
///
/// The implement task depends on the break-up task.
///
/// # Errors
///
/// Returns an error if the task store cannot be opened or tasks cannot be created.
pub fn create_plan_tasks(
    tool_response: &ExitPlanModeToolResponse,
    base_dir: &Path,
) -> Result<(), String> {
    let file_path =
        tool_response.file_path.as_deref().ok_or("No plan file path in ExitPlanMode response")?;

    // Extract plan name from filename
    let plan_name = Path::new(file_path).file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");

    let store = SqliteTaskStore::for_project(base_dir)
        .map_err(|e| format!("Failed to open task store: {e}"))?;

    // Task 1: Break up the plan
    let breakdown_task = store
        .create_task(
            &format!("Break up plan: {plan_name}"),
            &format!("Parse {file_path} and create individual tasks for each implementation step"),
            Priority::High,
        )
        .map_err(|e| format!("Failed to create breakdown task: {e}"))?;

    // Task 2: Implement the plan (depends on breakdown)
    let implement_task = store
        .create_task(
            &format!("Implement plan: {plan_name}"),
            &format!("Complete all work described in plan file: {file_path}"),
            Priority::High,
        )
        .map_err(|e| format!("Failed to create implement task: {e}"))?;

    // Add dependency: implement depends on breakdown
    store
        .add_dependency(&implement_task.id, &breakdown_task.id)
        .map_err(|e| format!("Failed to add dependency: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths;
    use crate::tasks::TaskFilter;
    use tempfile::TempDir;

    fn setup_db(dir: &Path) {
        let db_path = paths::project_db_path(dir);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn test_create_plan_tasks_success() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let tool_response = ExitPlanModeToolResponse {
            file_path: Some("/home/user/.claude/plans/my-cool-plan.md".to_string()),
            plan: None,
        };

        let result = create_plan_tasks(&tool_response, dir.path());
        assert!(result.is_ok());

        // Verify tasks were created
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 2);

        // Find the breakdown and implement tasks
        let breakdown = tasks.iter().find(|t| t.title.contains("Break up")).unwrap();
        let implement = tasks.iter().find(|t| t.title.contains("Implement")).unwrap();

        assert!(breakdown.title.contains("my-cool-plan"));
        assert!(implement.title.contains("my-cool-plan"));
        assert!(breakdown.description.contains("/home/user/.claude/plans/my-cool-plan.md"));
        assert!(implement.description.contains("/home/user/.claude/plans/my-cool-plan.md"));

        // Verify dependency
        let deps = store.get_dependencies(&implement.id).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], breakdown.id);
    }

    #[test]
    fn test_create_plan_tasks_no_file_path() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let tool_response =
            ExitPlanModeToolResponse { file_path: None, plan: Some("plan content".to_string()) };

        let result = create_plan_tasks(&tool_response, dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No plan file path"));
    }

    #[test]
    fn test_create_plan_tasks_extracts_plan_name() {
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        let tool_response = ExitPlanModeToolResponse {
            file_path: Some("~/.claude/plans/enchanted-wondering-valiant.md".to_string()),
            plan: None,
        };

        create_plan_tasks(&tool_response, dir.path()).unwrap();

        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();

        let breakdown = tasks.iter().find(|t| t.title.contains("Break up")).unwrap();
        assert!(breakdown.title.contains("enchanted-wondering-valiant"));
    }
}
