//! Beads to tasks synchronization.
//!
//! This module provides functionality to sync open beads issues to the tasks database.
//! It is called on session start to ensure Claude has visibility into open work.

use crate::error::Result;
use crate::paths;
use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
use crate::traits::CommandRunner;
use serde::Deserialize;
use std::path::Path;

/// A beads issue as returned by `bd list --format=json`.
#[derive(Debug, Deserialize)]
struct BeadsIssue {
    /// Unique issue ID (e.g., "project-123").
    id: String,
    /// Issue title.
    title: String,
    /// Issue description (may be empty).
    #[serde(default)]
    description: String,
    /// Priority (0-4, where 0 is highest).
    #[serde(default = "default_priority")]
    priority: u8,
    /// Issue type (bug, feature, task, etc.).
    #[serde(default)]
    r#type: String,
    /// Issue status (open, `in_progress`, complete, etc.).
    #[serde(default)]
    #[allow(dead_code)] // Status is included for potential future use
    status: String,
}

const fn default_priority() -> u8 {
    2 // Medium priority as default
}

/// Sync open beads issues to the tasks database.
///
/// Creates tasks for open beads issues that don't already exist in the tasks database.
/// Returns the number of tasks created.
///
/// # Arguments
///
/// * `runner` - Command runner for executing bd commands.
/// * `base_dir` - Base directory containing .claude/ folder.
///
/// # Errors
///
/// Returns an error if bd commands fail or database operations fail.
pub fn sync_beads_to_tasks(runner: &dyn CommandRunner, base_dir: &Path) -> Result<SyncResult> {
    // Check if beads is available
    if !crate::beads::is_beads_available_in(runner, base_dir) {
        return Ok(SyncResult { created: 0, skipped: 0, errors: Vec::new() });
    }

    // Get open issues from beads
    let output = runner.run("bd", &["list", "--status=open", "--format=json"], None)?;
    if !output.success() {
        // Beads command failed - maybe no issues
        return Ok(SyncResult { created: 0, skipped: 0, errors: Vec::new() });
    }

    // Also get in_progress issues
    let in_progress_output =
        runner.run("bd", &["list", "--status=in_progress", "--format=json"], None)?;

    // Parse issues
    let mut issues: Vec<BeadsIssue> = serde_json::from_str(&output.stdout).unwrap_or_default();
    if in_progress_output.success() {
        let in_progress: Vec<BeadsIssue> =
            serde_json::from_str(&in_progress_output.stdout).unwrap_or_default();
        issues.extend(in_progress);
    }

    if issues.is_empty() {
        return Ok(SyncResult { created: 0, skipped: 0, errors: Vec::new() });
    }

    // Open the tasks database
    let db_path = paths::project_db_path(base_dir)
        .ok_or_else(|| crate::error::Error::Config("Cannot determine home directory".into()))?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = SqliteTaskStore::new(&db_path)?;

    let mut result = SyncResult { created: 0, skipped: 0, errors: Vec::new() };

    // Get all existing tasks once to check for duplicates
    // FTS has issues with special characters like colons and hyphens, so we
    // do a simple contains check on descriptions
    let all_tasks = store.list_tasks(crate::tasks::TaskFilter::default())?;

    for issue in issues {
        // Check if task with this beads ID already exists
        // We store the beads ID in the description prefix
        let beads_marker = format!("[beads:{}]", issue.id);

        // Check if any existing task has this beads marker
        let has_existing = all_tasks.iter().any(|t| t.description.contains(&beads_marker));
        if has_existing {
            result.skipped += 1;
            continue;
        }

        // Map beads priority to task priority
        let priority = Priority::from_u8(issue.priority).unwrap_or(Priority::Medium);

        // Build description with beads reference
        let description = if issue.description.is_empty() {
            beads_marker
        } else {
            format!("{}\n\n{}", beads_marker, issue.description)
        };

        // Build title with type prefix if available
        let title = if issue.r#type.is_empty() {
            issue.title
        } else {
            format!("[{}] {}", issue.r#type, issue.title)
        };

        // Create the task
        match store.create_task(&title, &description, priority) {
            Ok(_) => result.created += 1,
            Err(e) => result.errors.push(format!("{}: {}", issue.id, e)), // coverage:ignore - requires db corruption
        }
    }

    Ok(result)
}

/// Result of syncing beads to tasks.
#[derive(Debug, Default)]
pub struct SyncResult {
    /// Number of tasks created.
    pub created: u32,
    /// Number of issues skipped (already exist as tasks).
    pub skipped: u32,
    /// Errors encountered during sync.
    pub errors: Vec<String>,
}

impl SyncResult {
    /// Check if there were any errors.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Check if any work was done.
    #[must_use]
    pub const fn has_changes(&self) -> bool {
        self.created > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockCommandRunner;
    use crate::traits::CommandOutput;
    use tempfile::TempDir;

    /// Get the database path for a project directory (for tests).
    fn test_db_path(project_dir: &Path) -> std::path::PathBuf {
        paths::project_db_path(project_dir).expect("test should have home dir")
    }

    #[test]
    fn test_sync_no_beads() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();
        // bd not available

        let result = sync_beads_to_tasks(&runner, dir.path()).unwrap();
        assert_eq!(result.created, 0);
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn test_sync_beads_available_no_issues() {
        let dir = TempDir::new().unwrap();
        // Create .beads directory
        std::fs::create_dir(dir.path().join(".beads")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput { exit_code: 0, stdout: "[]".to_string(), stderr: String::new() },
        );
        runner.expect(
            "bd",
            &["list", "--status=in_progress", "--format=json"],
            CommandOutput { exit_code: 0, stdout: "[]".to_string(), stderr: String::new() },
        );

        let result = sync_beads_to_tasks(&runner, dir.path()).unwrap();
        assert_eq!(result.created, 0);
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn test_sync_creates_tasks() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".beads")).unwrap();

        let issues_json = r#"[
            {"id": "proj-1", "title": "Fix bug", "description": "A bug", "priority": 1, "type": "bug", "status": "open"},
            {"id": "proj-2", "title": "Add feature", "description": "", "priority": 2, "type": "feature", "status": "open"}
        ]"#;

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput { exit_code: 0, stdout: issues_json.to_string(), stderr: String::new() },
        );
        runner.expect(
            "bd",
            &["list", "--status=in_progress", "--format=json"],
            CommandOutput { exit_code: 0, stdout: "[]".to_string(), stderr: String::new() },
        );

        let result = sync_beads_to_tasks(&runner, dir.path()).unwrap();
        assert_eq!(result.created, 2);
        assert_eq!(result.skipped, 0);

        // Verify tasks were created using the correct path
        let db_path = test_db_path(dir.path());
        let store = SqliteTaskStore::new(&db_path).unwrap();
        let tasks = store.list_tasks(crate::tasks::TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 2);

        // Check first task
        let bug_task = tasks.iter().find(|t| t.title.contains("Fix bug")).unwrap();
        assert!(bug_task.description.contains("[beads:proj-1]"));
        assert_eq!(bug_task.priority, Priority::High);

        // Check second task
        let feature_task = tasks.iter().find(|t| t.title.contains("Add feature")).unwrap();
        assert!(feature_task.description.contains("[beads:proj-2]"));
        assert_eq!(feature_task.priority, Priority::Medium);
    }

    #[test]
    fn test_sync_skips_existing() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".beads")).unwrap();

        // Pre-create a task with beads marker using the correct path
        let db_path = test_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::new(&db_path).unwrap();
        store.create_task("Existing task", "[beads:proj-1] Description", Priority::High).unwrap();

        let issues_json = r#"[{"id": "proj-1", "title": "Fix bug", "description": "", "priority": 1, "type": "", "status": "open"}]"#;

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput { exit_code: 0, stdout: issues_json.to_string(), stderr: String::new() },
        );
        runner.expect(
            "bd",
            &["list", "--status=in_progress", "--format=json"],
            CommandOutput { exit_code: 0, stdout: "[]".to_string(), stderr: String::new() },
        );

        let result = sync_beads_to_tasks(&runner, dir.path()).unwrap();
        assert_eq!(result.created, 0);
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn test_sync_handles_bd_failure() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".beads")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput { exit_code: 1, stdout: String::new(), stderr: "error".to_string() },
        );

        let result = sync_beads_to_tasks(&runner, dir.path()).unwrap();
        assert_eq!(result.created, 0);
    }

    #[test]
    fn test_beads_issue_default_priority() {
        let json = r#"{"id": "test-1", "title": "Test", "status": "open"}"#;
        let issue: BeadsIssue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.priority, 2); // Default medium
    }

    #[test]
    fn test_sync_result_methods() {
        let mut result = SyncResult::default();
        assert!(!result.has_errors());
        assert!(!result.has_changes());

        result.created = 1;
        assert!(result.has_changes());

        result.errors.push("error".to_string());
        assert!(result.has_errors());
    }

    #[test]
    fn test_sync_creates_task_without_type() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".beads")).unwrap();

        // Issue without a type - should use title directly without prefix
        let issues_json = r#"[{"id": "proj-1", "title": "Simple task", "description": "", "priority": 2, "type": "", "status": "open"}]"#;

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput { exit_code: 0, stdout: issues_json.to_string(), stderr: String::new() },
        );
        runner.expect(
            "bd",
            &["list", "--status=in_progress", "--format=json"],
            CommandOutput { exit_code: 0, stdout: "[]".to_string(), stderr: String::new() },
        );

        let result = sync_beads_to_tasks(&runner, dir.path()).unwrap();
        assert_eq!(result.created, 1);

        // Verify task was created with title only (no type prefix)
        let db_path = test_db_path(dir.path());
        let store = SqliteTaskStore::new(&db_path).unwrap();
        let tasks = store.list_tasks(crate::tasks::TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Simple task");
        assert!(!tasks[0].title.contains('['));
    }
}
