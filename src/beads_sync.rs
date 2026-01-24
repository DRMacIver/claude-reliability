//! Beads to tasks synchronization.
//!
//! This module provides functionality to sync open beads issues to the tasks database.
//! It is called on session start to ensure Claude has visibility into open work.
//!
//! The agent should never be told about beads - this integration is transparent.

use crate::error::Result;
use crate::paths;
use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
use crate::traits::CommandRunner;
use serde::Deserialize;
use std::path::Path;

/// Directory containing beads data.
const BEADS_DIR: &str = ".beads";

/// Marker prefix for beads issues in task descriptions.
const BEADS_MARKER_PREFIX: &str = "[beads:";

/// Check if beads is available in the specified directory.
fn is_beads_available_in(runner: &dyn CommandRunner, base_dir: &Path) -> bool {
    // Check if bd CLI is available
    if !runner.is_available("bd") {
        return false;
    }

    // Check if .beads/ directory exists
    base_dir.join(BEADS_DIR).is_dir()
}

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
    if !is_beads_available_in(runner, base_dir) {
        return Ok(SyncResult::default());
    }

    // Get open issues from beads
    let output = runner.run("bd", &["list", "--status=open", "--format=json"], None)?;
    if !output.success() {
        // Beads command failed - report the error
        return Err(crate::error::Error::CommandFailed {
            command: "bd list --status=open --format=json".to_string(),
            exit_code: output.exit_code,
            stderr: output.stderr,
        });
    }

    // Also get in_progress issues
    let in_progress_output =
        runner.run("bd", &["list", "--status=in_progress", "--format=json"], None)?;

    // Parse issues - report parsing errors instead of silently ignoring
    // The JSON error from serde_json is automatically converted via the From impl
    let mut issues: Vec<BeadsIssue> = serde_json::from_str(&output.stdout)?;

    if in_progress_output.success() {
        let in_progress: Vec<BeadsIssue> = serde_json::from_str(&in_progress_output.stdout)?;
        issues.extend(in_progress);
    }

    if issues.is_empty() {
        return Ok(SyncResult::default());
    }

    // Open the tasks database
    let db_path = paths::project_db_path(base_dir);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = SqliteTaskStore::new(&db_path)?;

    sync_issues_to_store(&issues, &store)
}

/// Internal helper to sync issues to a task store.
///
/// This is extracted to allow testing the sync logic with mock stores.
fn sync_issues_to_store(issues: &[BeadsIssue], store: &dyn TaskStore) -> Result<SyncResult> {
    let mut result = SyncResult::default();

    // Get all existing tasks once to check for duplicates
    // FTS has issues with special characters like colons and hyphens, so we
    // do a simple contains check on descriptions
    let all_tasks = store.list_tasks(crate::tasks::TaskFilter::default())?;

    for issue in issues {
        // Check if task with this beads ID already exists
        // We store the beads ID in the description prefix
        let beads_marker = format!("{BEADS_MARKER_PREFIX}{}]", issue.id);

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
            format!("{beads_marker}\n\n{}", issue.description)
        };

        // Build title with type prefix if available
        let title = if issue.r#type.is_empty() {
            issue.title.clone()
        } else {
            format!("[{}] {}", issue.r#type, issue.title)
        };

        // Create the task
        match store.create_task(&title, &description, priority) {
            Ok(_) => result.created += 1,
            Err(e) => result.errors.push(format!("{}: {e}", issue.id)),
        }
    }

    Ok(result)
}

/// Extract the beads issue ID from a task description if present.
///
/// Returns the issue ID (e.g., "proj-123") if the description contains a beads marker.
pub fn extract_beads_id(description: &str) -> Option<&str> {
    let start = description.find(BEADS_MARKER_PREFIX)?;
    let after_prefix = &description[start + BEADS_MARKER_PREFIX.len()..];
    let end = after_prefix.find(']')?;
    Some(&after_prefix[..end])
}

/// Close a beads issue by ID.
///
/// # Arguments
///
/// * `runner` - Command runner for executing bd commands.
/// * `base_dir` - Base directory containing .beads/ folder.
/// * `issue_id` - The beads issue ID to close.
///
/// # Errors
///
/// Returns an error if the bd command fails.
pub fn close_beads_issue(
    runner: &dyn CommandRunner,
    base_dir: &Path,
    issue_id: &str,
) -> Result<bool> {
    // Check if beads is available
    if !is_beads_available_in(runner, base_dir) {
        return Ok(false);
    }

    let output = runner.run("bd", &["close", issue_id], None)?;
    Ok(output.success())
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
        paths::project_db_path(project_dir)
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

        // Now returns an error instead of silently succeeding
        let result = sync_beads_to_tasks(&runner, dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("bd list"));
        assert!(err.contains("exit code 1"));
    }

    #[test]
    fn test_sync_handles_invalid_json() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".beads")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput {
                exit_code: 0,
                stdout: "not valid json".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "bd",
            &["list", "--status=in_progress", "--format=json"],
            CommandOutput { exit_code: 0, stdout: "[]".to_string(), stderr: String::new() },
        );

        // Now returns an error instead of silently returning empty result
        let result = sync_beads_to_tasks(&runner, dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("JSON"));
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

    #[test]
    fn test_extract_beads_id() {
        assert_eq!(extract_beads_id("[beads:proj-123]\n\nDescription"), Some("proj-123"));
        assert_eq!(extract_beads_id("[beads:test-1]"), Some("test-1"));
        assert_eq!(extract_beads_id("Some text [beads:id] more text"), Some("id"));
        assert_eq!(extract_beads_id("No beads marker"), None);
        assert_eq!(extract_beads_id("[beads:incomplete"), None);
        assert_eq!(extract_beads_id(""), None);
    }

    #[test]
    fn test_close_beads_issue_no_beads() {
        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();
        // bd not available

        let result = close_beads_issue(&runner, dir.path(), "proj-1").unwrap();
        assert!(!result);
    }

    #[test]
    fn test_close_beads_issue_success() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".beads")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["close", "proj-1"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let result = close_beads_issue(&runner, dir.path(), "proj-1").unwrap();
        assert!(result);
    }

    #[test]
    fn test_close_beads_issue_failure() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".beads")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");
        runner.expect(
            "bd",
            &["close", "proj-1"],
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: "Issue not found".to_string(),
            },
        );

        let result = close_beads_issue(&runner, dir.path(), "proj-1").unwrap();
        assert!(!result);
    }

    /// A mock task store for testing error handling.
    struct FailingTaskStore;

    impl TaskStore for FailingTaskStore {
        fn create_task(
            &self,
            _title: &str,
            _description: &str,
            _priority: Priority,
        ) -> crate::error::Result<crate::tasks::Task> {
            Err(crate::error::Error::Config("Simulated database error".into()))
        }

        fn get_task(&self, _id: &str) -> crate::error::Result<Option<crate::tasks::Task>> {
            Ok(None)
        }

        fn update_task(
            &self,
            _id: &str,
            _update: crate::tasks::TaskUpdate,
        ) -> crate::error::Result<Option<crate::tasks::Task>> {
            Ok(None)
        }

        fn delete_task(&self, _id: &str) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn list_tasks(
            &self,
            _filter: crate::tasks::TaskFilter,
        ) -> crate::error::Result<Vec<crate::tasks::Task>> {
            Ok(vec![])
        }

        fn add_dependency(&self, _task_id: &str, _depends_on: &str) -> crate::error::Result<()> {
            Ok(())
        }

        fn remove_dependency(
            &self,
            _task_id: &str,
            _depends_on: &str,
        ) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn get_dependencies(&self, _task_id: &str) -> crate::error::Result<Vec<String>> {
            Ok(vec![])
        }

        fn get_dependents(&self, _task_id: &str) -> crate::error::Result<Vec<String>> {
            Ok(vec![])
        }

        fn add_note(
            &self,
            _task_id: &str,
            _content: &str,
        ) -> crate::error::Result<crate::tasks::Note> {
            unimplemented!()
        }

        fn get_notes(&self, _task_id: &str) -> crate::error::Result<Vec<crate::tasks::Note>> {
            Ok(vec![])
        }

        fn delete_note(&self, _note_id: i64) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn search_tasks(&self, _query: &str) -> crate::error::Result<Vec<crate::tasks::Task>> {
            Ok(vec![])
        }

        fn get_audit_log(
            &self,
            _task_id: Option<&str>,
            _limit: Option<usize>,
        ) -> crate::error::Result<Vec<crate::tasks::AuditEntry>> {
            Ok(vec![])
        }

        fn get_ready_tasks(&self) -> crate::error::Result<Vec<crate::tasks::Task>> {
            Ok(vec![])
        }

        fn pick_task(&self) -> crate::error::Result<Option<crate::tasks::Task>> {
            Ok(None)
        }

        fn create_howto(
            &self,
            _title: &str,
            _instructions: &str,
        ) -> crate::error::Result<crate::tasks::HowTo> {
            unimplemented!()
        }

        fn get_howto(&self, _id: &str) -> crate::error::Result<Option<crate::tasks::HowTo>> {
            Ok(None)
        }

        fn update_howto(
            &self,
            _id: &str,
            _update: crate::tasks::HowToUpdate,
        ) -> crate::error::Result<Option<crate::tasks::HowTo>> {
            Ok(None)
        }

        fn delete_howto(&self, _id: &str) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn list_howtos(&self) -> crate::error::Result<Vec<crate::tasks::HowTo>> {
            Ok(vec![])
        }

        fn search_howtos(&self, _query: &str) -> crate::error::Result<Vec<crate::tasks::HowTo>> {
            Ok(vec![])
        }

        fn link_task_to_howto(&self, _task_id: &str, _howto_id: &str) -> crate::error::Result<()> {
            Ok(())
        }

        fn unlink_task_from_howto(
            &self,
            _task_id: &str,
            _howto_id: &str,
        ) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn get_task_guidance(&self, _task_id: &str) -> crate::error::Result<Vec<String>> {
            Ok(vec![])
        }

        fn create_question(&self, _text: &str) -> crate::error::Result<crate::tasks::Question> {
            unimplemented!()
        }

        fn get_question(&self, _id: &str) -> crate::error::Result<Option<crate::tasks::Question>> {
            Ok(None)
        }

        fn answer_question(
            &self,
            _id: &str,
            _answer: &str,
        ) -> crate::error::Result<Option<crate::tasks::Question>> {
            Ok(None)
        }

        fn delete_question(&self, _id: &str) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn list_questions(
            &self,
            _unanswered_only: bool,
        ) -> crate::error::Result<Vec<crate::tasks::Question>> {
            Ok(vec![])
        }

        fn search_questions(
            &self,
            _query: &str,
        ) -> crate::error::Result<Vec<crate::tasks::Question>> {
            Ok(vec![])
        }

        fn link_task_to_question(
            &self,
            _task_id: &str,
            _question_id: &str,
        ) -> crate::error::Result<()> {
            Ok(())
        }

        fn unlink_task_from_question(
            &self,
            _task_id: &str,
            _question_id: &str,
        ) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn get_blocking_questions(
            &self,
            _task_id: &str,
        ) -> crate::error::Result<Vec<crate::tasks::Question>> {
            Ok(vec![])
        }

        fn get_question_blocked_tasks(&self) -> crate::error::Result<Vec<crate::tasks::Task>> {
            Ok(vec![])
        }

        fn request_tasks(&self, _task_ids: &[&str]) -> crate::error::Result<usize> {
            Ok(0)
        }

        fn request_all_open(&self) -> crate::error::Result<usize> {
            Ok(0)
        }

        fn get_incomplete_requested_tasks(&self) -> crate::error::Result<Vec<crate::tasks::Task>> {
            Ok(vec![])
        }

        fn clear_request_mode(&self) -> crate::error::Result<()> {
            Ok(())
        }

        fn get_task_questions(&self, _task_id: &str) -> crate::error::Result<Vec<String>> {
            Ok(vec![])
        }

        fn has_in_progress_task(&self) -> crate::error::Result<bool> {
            Ok(false)
        }

        fn get_in_progress_tasks(&self) -> crate::error::Result<Vec<crate::tasks::Task>> {
            Ok(vec![])
        }

        fn is_request_mode_active(&self) -> crate::error::Result<bool> {
            Ok(false)
        }
    }

    #[test]
    fn test_sync_issues_handles_create_error() {
        let issues = vec![BeadsIssue {
            id: "proj-1".to_string(),
            title: "Test issue".to_string(),
            description: "Description".to_string(),
            priority: 2,
            r#type: "bug".to_string(),
            status: "open".to_string(),
        }];

        let store = FailingTaskStore;
        let result = sync_issues_to_store(&issues, &store).unwrap();

        assert_eq!(result.created, 0);
        assert_eq!(result.skipped, 0);
        assert!(result.has_errors());
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("proj-1"));
    }
}
