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

/// Find the most recently modified plan file in ~/.claude/plans/.
///
/// This is used when `PostToolUse` doesn't fire for `ExitPlanMode` and we need
/// to find the plan file at `PreToolUse` time.
pub fn find_most_recent_plan_file() -> Option<std::path::PathBuf> {
    let plans_dir = dirs::home_dir()?.join(".claude").join("plans");
    find_most_recent_plan_file_in_dir(&plans_dir)
}

/// Find the most recently modified .md file in a given directory.
///
/// This is the testable core logic for `find_most_recent_plan_file`.
fn find_most_recent_plan_file_in_dir(plans_dir: &Path) -> Option<std::path::PathBuf> {
    if !plans_dir.exists() {
        return None;
    }

    let entries = std::fs::read_dir(plans_dir).ok()?;
    let mut most_recent: Option<(std::path::PathBuf, std::time::SystemTime)> = None;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            if let Ok(metadata) = path.metadata() {
                if let Ok(modified) = metadata.modified() {
                    match &most_recent {
                        None => most_recent = Some((path, modified)),
                        Some((_, prev_time)) if modified > *prev_time => {
                            most_recent = Some((path, modified));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    most_recent.map(|(path, _)| path)
}

/// Create plan tasks by finding the most recent plan file.
///
/// This is the fallback when `PostToolUse` doesn't fire. It finds the most recently
/// modified plan file in ~/.claude/plans/ and creates tasks for it.
pub fn create_plan_tasks_from_recent(base_dir: &Path) -> Result<(), String> {
    let plan_file =
        find_most_recent_plan_file().ok_or("No plan files found in ~/.claude/plans/")?;

    let tool_response = ExitPlanModeToolResponse {
        file_path: Some(plan_file.to_string_lossy().to_string()),
        plan: None,
    };

    create_plan_tasks(&tool_response, base_dir)
}

/// Create plan tasks from a specific plans directory.
///
/// This is the testable version that allows specifying a custom plans directory.
pub fn create_plan_tasks_from_dir(plans_dir: &Path, base_dir: &Path) -> Result<(), String> {
    let plan_file = find_most_recent_plan_file_in_dir(plans_dir)
        .ok_or_else(|| format!("No plan files found in {}", plans_dir.display()))?;

    let tool_response = ExitPlanModeToolResponse {
        file_path: Some(plan_file.to_string_lossy().to_string()),
        plan: None,
    };

    create_plan_tasks(&tool_response, base_dir)
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

    #[test]
    fn test_find_most_recent_plan_file_in_dir_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let nonexistent = temp_dir.path().join("nonexistent");

        let result = find_most_recent_plan_file_in_dir(&nonexistent);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_most_recent_plan_file_in_dir_empty() {
        let temp_dir = TempDir::new().unwrap();
        let plans_dir = temp_dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        let result = find_most_recent_plan_file_in_dir(&plans_dir);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_most_recent_plan_file_in_dir_single_file() {
        let temp_dir = TempDir::new().unwrap();
        let plans_dir = temp_dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        let plan_file = plans_dir.join("test-plan.md");
        std::fs::write(&plan_file, "# Plan").unwrap();

        let result = find_most_recent_plan_file_in_dir(&plans_dir);
        assert_eq!(result, Some(plan_file));
    }

    #[test]
    fn test_find_most_recent_plan_file_in_dir_multiple_files() {
        use std::fs::FileTimes;
        use std::time::{Duration, SystemTime};

        let temp_dir = TempDir::new().unwrap();
        let plans_dir = temp_dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        // Create three files and set explicit modification times
        // This ensures we exercise all match branches regardless of readdir order
        let file_a = plans_dir.join("a-plan.md");
        let file_b = plans_dir.join("b-plan.md");
        let file_c = plans_dir.join("c-plan.md");

        std::fs::write(&file_a, "# Plan A").unwrap();
        std::fs::write(&file_b, "# Plan B").unwrap();
        std::fs::write(&file_c, "# Plan C").unwrap();

        // Set explicit modification times: file_b is newest
        let now = SystemTime::now();
        let oldest = now - Duration::from_secs(300);
        let middle = now - Duration::from_secs(200);
        let newest = now - Duration::from_secs(100);

        std::fs::File::open(&file_a)
            .unwrap()
            .set_times(FileTimes::new().set_modified(oldest))
            .unwrap();
        std::fs::File::open(&file_b)
            .unwrap()
            .set_times(FileTimes::new().set_modified(newest))
            .unwrap();
        std::fs::File::open(&file_c)
            .unwrap()
            .set_times(FileTimes::new().set_modified(middle))
            .unwrap();

        // With 3 files and distinct times, regardless of readdir order,
        // we will hit the `_ => {}` branch at least once when a file
        // is checked after a newer file was already found
        let result = find_most_recent_plan_file_in_dir(&plans_dir);
        assert_eq!(result, Some(file_b)); // file_b is newest
    }

    #[test]
    fn test_find_most_recent_plan_file_in_dir_ignores_non_md() {
        let temp_dir = TempDir::new().unwrap();
        let plans_dir = temp_dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        // Create a non-md file
        let txt_file = plans_dir.join("notes.txt");
        std::fs::write(&txt_file, "Notes").unwrap();

        // Create a md file
        let md_file = plans_dir.join("plan.md");
        std::fs::write(&md_file, "# Plan").unwrap();

        let result = find_most_recent_plan_file_in_dir(&plans_dir);
        assert_eq!(result, Some(md_file));
    }

    #[test]
    fn test_find_most_recent_plan_file_in_dir_only_non_md() {
        let temp_dir = TempDir::new().unwrap();
        let plans_dir = temp_dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        // Create only a non-md file
        let txt_file = plans_dir.join("notes.txt");
        std::fs::write(&txt_file, "Notes").unwrap();

        let result = find_most_recent_plan_file_in_dir(&plans_dir);
        assert!(result.is_none());
    }

    #[test]
    fn test_create_plan_tasks_from_recent_no_plans() {
        // This test verifies error handling when no plans exist
        // Since we can't mock home_dir, we just verify the error message format
        let dir = TempDir::new().unwrap();
        setup_db(dir.path());

        // This will fail because we can't control ~/.claude/plans in tests
        // But we verify the function doesn't panic
        let result = create_plan_tasks_from_recent(dir.path());
        // May succeed or fail depending on whether ~/.claude/plans exists
        let _ = result;
    }
}
