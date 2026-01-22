//! Task management system.
//!
//! This module provides a task tracking system with:
//! - Tasks with title, description, priority, and status
//! - Dependencies between tasks (with circular dependency detection)
//! - Notes attached to tasks
//! - Full-text search across tasks and notes
//! - Audit logging for all operations
//!
//! # Example
//!
//! ```no_run
//! use claude_reliability::tasks::{SqliteTaskStore, TaskStore, Priority};
//!
//! let store = SqliteTaskStore::new("/tmp/tasks.db").unwrap();
//!
//! // Create a task
//! let task = store.create_task("Fix login bug", "Users cannot login with OAuth", Priority::High).unwrap();
//!
//! // Add a dependency
//! let blocker = store.create_task("Deploy auth service", "", Priority::Critical).unwrap();
//! store.add_dependency(&task.id, &blocker.id).unwrap();
//!
//! // Search for tasks
//! let results = store.search_tasks("login").unwrap();
//! ```

pub mod id;
pub mod models;
pub mod store;

pub use models::{
    AuditEntry, HowTo, InvalidPriority, InvalidStatus, Note, Priority, Question, Status, Task,
};
pub use store::{
    CircularDependency, HowToNotFound, HowToUpdate, QuestionNotFound, SqliteTaskStore, TaskFilter,
    TaskNotFound, TaskStore, TaskUpdate,
};

use std::path::Path;

/// Try to suggest a task to work on next.
///
/// Opens the task database at the standard location and picks a random high-priority
/// ready task. Returns `None` if the database doesn't exist, is empty, or on any error.
///
/// Returns `Some((id, title))` of the suggested task.
#[must_use]
pub fn suggest_task(base_dir: &Path) -> Option<(String, String)> {
    let db_path = base_dir.join(".claude/claude-reliability-working-memory.sqlite3");
    if !db_path.exists() {
        return None;
    }

    let store = SqliteTaskStore::new(&db_path).ok()?;
    let task = store.pick_task().ok()??;
    Some((task.id, task.title))
}

/// Count the number of ready tasks (open and not blocked).
///
/// Returns 0 if the database doesn't exist or on any error.
#[must_use]
pub fn count_ready_tasks(base_dir: &Path) -> u32 {
    let db_path = base_dir.join(".claude/claude-reliability-working-memory.sqlite3");
    if !db_path.exists() {
        return 0;
    }

    let Ok(store) = SqliteTaskStore::new(&db_path) else {
        return 0;
    };

    store.get_ready_tasks().map(|tasks| u32::try_from(tasks.len()).unwrap_or(u32::MAX)).unwrap_or(0)
}

/// Get tasks that are blocked only by unanswered questions (not by dependencies).
///
/// Returns a list of `(task_id, task_title, blocking_questions)` tuples.
/// Returns empty vec if database doesn't exist or on any error.
#[must_use]
pub fn get_question_blocked_tasks(base_dir: &Path) -> Vec<(String, String, Vec<Question>)> {
    let db_path = base_dir.join(".claude/claude-reliability-working-memory.sqlite3");
    if !db_path.exists() {
        return Vec::new();
    }

    let Ok(store) = SqliteTaskStore::new(&db_path) else {
        return Vec::new();
    };

    // coverage:ignore - Query failure after successful open is unreachable in practice
    let Ok(tasks) = store.get_question_blocked_tasks() else {
        return Vec::new(); // coverage:ignore
    };

    tasks
        .into_iter()
        .filter_map(|task| {
            let questions = store.get_blocking_questions(&task.id).ok()?;
            Some((task.id, task.title, questions))
        })
        .collect()
}

/// List all unanswered questions.
///
/// Returns empty vec if database doesn't exist or on any error.
#[must_use]
pub fn list_unanswered_questions(base_dir: &Path) -> Vec<Question> {
    let db_path = base_dir.join(".claude/claude-reliability-working-memory.sqlite3");
    if !db_path.exists() {
        return Vec::new();
    }

    let Ok(store) = SqliteTaskStore::new(&db_path) else {
        return Vec::new();
    };

    store.list_questions(true).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_suggest_task_no_database() {
        let dir = TempDir::new().unwrap();
        let result = suggest_task(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_suggest_task_empty_database() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        // Create empty database
        let _store = SqliteTaskStore::new(&db_path).unwrap();

        let result = suggest_task(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_suggest_task_with_task() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        // Create database with a task
        let store = SqliteTaskStore::new(&db_path).unwrap();
        store.create_task("Test task", "Description", Priority::High).unwrap();

        let result = suggest_task(dir.path());
        assert!(result.is_some());
        let (id, title) = result.unwrap();
        assert!(id.starts_with("test-task-"));
        assert_eq!(title, "Test task");
    }

    #[test]
    fn test_count_ready_tasks_no_database() {
        let dir = TempDir::new().unwrap();
        let count = count_ready_tasks(dir.path());
        assert_eq!(count, 0);
    }

    #[test]
    fn test_count_ready_tasks_empty() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        let _store = SqliteTaskStore::new(&db_path).unwrap();

        let count = count_ready_tasks(dir.path());
        assert_eq!(count, 0);
    }

    #[test]
    fn test_count_ready_tasks_with_tasks() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        let store = SqliteTaskStore::new(&db_path).unwrap();
        store.create_task("Task 1", "", Priority::High).unwrap();
        store.create_task("Task 2", "", Priority::Medium).unwrap();

        let count = count_ready_tasks(dir.path());
        assert_eq!(count, 2);
    }

    #[test]
    fn test_count_ready_tasks_corrupted_database() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        // Write invalid content to the database file
        std::fs::write(&db_path, "this is not a valid sqlite database").unwrap();

        // Should return 0 when database fails to open
        let count = count_ready_tasks(dir.path());
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_question_blocked_tasks_no_database() {
        let dir = TempDir::new().unwrap();
        let result = get_question_blocked_tasks(dir.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_question_blocked_tasks_empty() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        let _store = SqliteTaskStore::new(&db_path).unwrap();

        let result = get_question_blocked_tasks(dir.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_question_blocked_tasks_with_blocked_task() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        let store = SqliteTaskStore::new(&db_path).unwrap();
        let task = store.create_task("Blocked task", "", Priority::High).unwrap();
        let question = store.create_question("What should the API return?").unwrap();
        store.link_task_to_question(&task.id, &question.id).unwrap();

        let result = get_question_blocked_tasks(dir.path());
        assert_eq!(result.len(), 1);
        let (id, title, questions) = &result[0];
        assert_eq!(id, &task.id);
        assert_eq!(title, "Blocked task");
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].text, "What should the API return?");
    }

    #[test]
    fn test_get_question_blocked_tasks_answered_question_not_blocked() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        let store = SqliteTaskStore::new(&db_path).unwrap();
        let task = store.create_task("Task with answered question", "", Priority::High).unwrap();
        let question = store.create_question("What format?").unwrap();
        store.link_task_to_question(&task.id, &question.id).unwrap();
        store.answer_question(&question.id, "JSON format").unwrap();

        // Task should not be blocked since question is answered
        let result = get_question_blocked_tasks(dir.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_get_question_blocked_tasks_corrupted_database() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        std::fs::write(&db_path, "invalid database").unwrap();

        let result = get_question_blocked_tasks(dir.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_list_unanswered_questions_no_database() {
        let dir = TempDir::new().unwrap();
        let result = list_unanswered_questions(dir.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_list_unanswered_questions_empty() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        let _store = SqliteTaskStore::new(&db_path).unwrap();

        let result = list_unanswered_questions(dir.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_list_unanswered_questions_with_questions() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        let store = SqliteTaskStore::new(&db_path).unwrap();
        let q1 = store.create_question("Question 1?").unwrap();
        let _q2 = store.create_question("Question 2?").unwrap();
        store.answer_question(&q1.id, "Answer 1").unwrap();

        // Only q2 should be returned (unanswered)
        let result = list_unanswered_questions(dir.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "Question 2?");
    }

    #[test]
    fn test_list_unanswered_questions_corrupted_database() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let db_path = claude_dir.join("claude-reliability-working-memory.sqlite3");

        std::fs::write(&db_path, "invalid database").unwrap();

        let result = list_unanswered_questions(dir.path());
        assert!(result.is_empty());
    }
}
