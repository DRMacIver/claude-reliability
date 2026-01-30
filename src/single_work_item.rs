//! Single work item mode.
//!
//! When the `CLAUDE_RELIABILITY_SINGLE_WORK_ITEM` environment variable is set,
//! the session is constrained to a single assigned work item. This module
//! provides helpers for reading, validating, and checking the status of that item.

use crate::paths;
use crate::tasks::{SqliteTaskStore, Status, TaskStore};
use std::path::Path;

/// Environment variable name for single work item mode.
const ENV_VAR: &str = "CLAUDE_RELIABILITY_SINGLE_WORK_ITEM";

/// Read the `CLAUDE_RELIABILITY_SINGLE_WORK_ITEM` environment variable.
///
/// Returns `Some(id)` if the variable is set and non-empty, `None` otherwise.
#[must_use]
pub fn get_single_work_item_id() -> Option<String> {
    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
}

/// Validate that the single work item exists and is open.
///
/// Reads the work item ID from the environment variable. To pass the ID
/// explicitly (e.g. in tests), use [`validate_work_item`].
///
/// # Errors
///
/// Returns an error string describing the validation failure.
pub fn validate_single_work_item(base_dir: &Path) -> Result<(String, String), String> {
    let id = get_single_work_item_id()
        .ok_or_else(|| "CLAUDE_RELIABILITY_SINGLE_WORK_ITEM is not set".to_string())?;
    validate_work_item(base_dir, &id)
}

/// Validate that a specific work item exists and is open.
///
/// Returns `Ok((id, title))` if the item exists and is in an open (non-terminal) state.
/// Returns `Err(message)` if the item doesn't exist, is already closed, or the
/// database cannot be opened.
///
/// # Errors
///
/// Returns an error string describing the validation failure.
pub fn validate_work_item(base_dir: &Path, id: &str) -> Result<(String, String), String> {
    let db_path = paths::project_db_path(base_dir);
    let store =
        SqliteTaskStore::new(&db_path).map_err(|e| format!("Failed to open task store: {e}"))?;

    let task = store
        .get_task(id)
        .map_err(|e| format!("Failed to look up work item {id}: {e}"))?
        .ok_or_else(|| format!("Single work item not found: {id}"))?;

    if matches!(task.status, Status::Complete | Status::Abandoned) {
        return Err(format!(
            "Single work item {id} is already closed (status: {})",
            task.status.as_str()
        ));
    }

    Ok((task.id, task.title))
}

/// Check whether the single work item (from env var) is complete (or abandoned).
///
/// Returns `true` if the item has a terminal status (complete/abandoned),
/// `false` if it is still open, the env var is not set, or the database cannot be read.
#[must_use]
pub fn is_single_work_item_complete(base_dir: &Path) -> bool {
    let Some(id) = get_single_work_item_id() else {
        return false;
    };
    is_work_item_complete(base_dir, &id)
}

/// Check whether a specific work item is complete (or abandoned).
///
/// Returns `true` if the item has a terminal status (complete/abandoned),
/// `false` if it is still open or if the database cannot be read.
#[must_use]
pub fn is_work_item_complete(base_dir: &Path, id: &str) -> bool {
    let db_path = paths::project_db_path(base_dir);
    let Ok(store) = SqliteTaskStore::new(&db_path) else {
        return false;
    };

    match store.get_task(id) {
        Ok(Some(task)) => matches!(task.status, Status::Complete | Status::Abandoned),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{Priority, TaskStore, TaskUpdate};
    use serial_test::serial;
    use tempfile::TempDir;

    /// Helper: set the env var for the duration of a test.
    fn set_env(val: &str) {
        std::env::set_var(ENV_VAR, val);
    }

    /// Helper: remove the env var.
    fn unset_env() {
        std::env::remove_var(ENV_VAR);
    }

    /// Helper: create a store in a temp dir and return `(dir, store)`.
    fn make_store() -> (TempDir, SqliteTaskStore) {
        let dir = TempDir::new().unwrap();
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        (dir, store)
    }

    // -- get_single_work_item_id --

    #[test]
    #[serial]
    fn test_get_returns_none_when_unset() {
        unset_env();
        assert!(get_single_work_item_id().is_none());
    }

    #[test]
    #[serial]
    fn test_get_returns_none_when_empty() {
        set_env("");
        assert!(get_single_work_item_id().is_none());
        unset_env();
    }

    #[test]
    #[serial]
    fn test_get_returns_some_when_set() {
        set_env("my-task-1234");
        assert_eq!(get_single_work_item_id().as_deref(), Some("my-task-1234"));
        unset_env();
    }

    // -- validate_single_work_item --

    #[test]
    #[serial]
    fn test_validate_errors_when_env_unset() {
        unset_env();
        let dir = TempDir::new().unwrap();
        let result = validate_single_work_item(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not set"));
    }

    #[test]
    #[serial]
    fn test_validate_errors_for_nonexistent_id() {
        let (dir, _store) = make_store();
        set_env("nonexistent-id");
        let result = validate_single_work_item(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
        unset_env();
    }

    #[test]
    #[serial]
    fn test_validate_errors_for_complete_task() {
        let (dir, store) = make_store();
        let task = store.create_task("Done task", "Already done", Priority::Medium).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        set_env(&task.id);
        let result = validate_single_work_item(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already closed"));
        unset_env();
    }

    #[test]
    #[serial]
    fn test_validate_errors_for_abandoned_task() {
        let (dir, store) = make_store();
        let task = store.create_task("Abandoned task", "Was abandoned", Priority::Medium).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Abandoned), ..Default::default() },
            )
            .unwrap();

        set_env(&task.id);
        let result = validate_single_work_item(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already closed"));
        unset_env();
    }

    #[test]
    #[serial]
    fn test_validate_succeeds_for_open_task() {
        let (dir, store) = make_store();
        let task = store.create_task("Open task", "Needs work", Priority::High).unwrap();

        set_env(&task.id);
        let result = validate_single_work_item(dir.path());
        assert!(result.is_ok());
        let (id, title) = result.unwrap();
        assert_eq!(id, task.id);
        assert_eq!(title, "Open task");
        unset_env();
    }

    #[test]
    #[serial]
    fn test_validate_succeeds_for_stuck_task() {
        let (dir, store) = make_store();
        let task = store.create_task("Stuck task", "Stuck on something", Priority::Medium).unwrap();
        store
            .update_task(&task.id, TaskUpdate { status: Some(Status::Stuck), ..Default::default() })
            .unwrap();

        set_env(&task.id);
        let result = validate_single_work_item(dir.path());
        assert!(result.is_ok());
        unset_env();
    }

    // -- is_single_work_item_complete --

    #[test]
    #[serial]
    fn test_complete_returns_false_when_env_unset() {
        unset_env();
        let dir = TempDir::new().unwrap();
        assert!(!is_single_work_item_complete(dir.path()));
    }

    #[test]
    #[serial]
    fn test_complete_returns_false_for_open_task() {
        let (dir, store) = make_store();
        let task = store.create_task("Open task", "Still open", Priority::Medium).unwrap();

        set_env(&task.id);
        assert!(!is_single_work_item_complete(dir.path()));
        unset_env();
    }

    #[test]
    #[serial]
    fn test_complete_returns_true_for_complete_task() {
        let (dir, store) = make_store();
        let task = store.create_task("Done task", "All done", Priority::Medium).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        set_env(&task.id);
        assert!(is_single_work_item_complete(dir.path()));
        unset_env();
    }

    #[test]
    #[serial]
    fn test_complete_returns_true_for_abandoned_task() {
        let (dir, store) = make_store();
        let task = store.create_task("Abandoned", "Gave up", Priority::Medium).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Abandoned), ..Default::default() },
            )
            .unwrap();

        set_env(&task.id);
        assert!(is_single_work_item_complete(dir.path()));
        unset_env();
    }

    #[test]
    #[serial]
    fn test_complete_returns_false_for_nonexistent_id() {
        let (dir, _store) = make_store();
        set_env("nonexistent-id");
        assert!(!is_single_work_item_complete(dir.path()));
        unset_env();
    }

    #[test]
    #[serial]
    fn test_complete_returns_false_when_no_database() {
        let dir = TempDir::new().unwrap();
        set_env("some-id");
        assert!(!is_single_work_item_complete(dir.path()));
        unset_env();
    }

    // -- is_work_item_complete (explicit ID variant) --

    #[test]
    fn test_work_item_complete_returns_false_when_db_open_fails() {
        let dir = TempDir::new().unwrap();
        // Create .claude-reliability as a file (not directory) so store creation fails
        let store_dir = dir.path().join(".claude-reliability");
        std::fs::write(&store_dir, "not a directory").unwrap();
        assert!(!is_work_item_complete(dir.path(), "some-id"));
    }

    #[test]
    fn test_work_item_complete_returns_true_for_complete() {
        let (dir, store) = make_store();
        let task = store.create_task("Done", "Done", Priority::Medium).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();
        assert!(is_work_item_complete(dir.path(), &task.id));
    }

    #[test]
    fn test_work_item_complete_returns_false_for_open() {
        let (dir, store) = make_store();
        let task = store.create_task("Open", "Open", Priority::Medium).unwrap();
        assert!(!is_work_item_complete(dir.path(), &task.id));
    }

    #[test]
    fn test_work_item_complete_returns_false_for_unknown_id() {
        let (dir, _store) = make_store();
        assert!(!is_work_item_complete(dir.path(), "unknown-id"));
    }

    // -- validate_work_item (explicit ID variant) --

    #[test]
    fn test_validate_work_item_succeeds_for_open() {
        let (dir, store) = make_store();
        let task = store.create_task("Open", "Open", Priority::High).unwrap();
        let result = validate_work_item(dir.path(), &task.id);
        assert!(result.is_ok());
        let (id, title) = result.unwrap();
        assert_eq!(id, task.id);
        assert_eq!(title, "Open");
    }

    #[test]
    fn test_validate_work_item_errors_for_complete() {
        let (dir, store) = make_store();
        let task = store.create_task("Done", "Done", Priority::Medium).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();
        let result = validate_work_item(dir.path(), &task.id);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already closed"));
    }

    #[test]
    fn test_validate_work_item_errors_for_nonexistent() {
        let (dir, _store) = make_store();
        let result = validate_work_item(dir.path(), "nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }
}
