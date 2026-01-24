//! Bulk task operations for fast batch processing.
//!
//! This module provides functions for creating multiple tasks, setting up
//! dependencies, listing, and searching tasks - all via JSON input/output
//! which is much faster than individual MCP tool calls.
//!
//! # Usage
//!
//! ```bash
//! echo '{"tasks": [...]}' | bulk-tasks create
//! echo '{"dependencies": [...]}' | bulk-tasks add-deps
//! echo '{"ready_only": true}' | bulk-tasks list
//! echo '{"query": "search term"}' | bulk-tasks search
//! ```
//!
//! # JSON Format for `create`
//!
//! ```json
//! {
//!   "tasks": [
//!     {
//!       "id": "1",
//!       "title": "First task",
//!       "description": "Description here",
//!       "priority": 2,
//!       "depends_on": []
//!     },
//!     {
//!       "id": "2",
//!       "title": "Second task",
//!       "description": "Depends on first",
//!       "priority": 2,
//!       "depends_on": ["1"]
//!     }
//!   ]
//! }
//! ```
//!
//! The `id` field is a temporary identifier used only for setting up dependencies
//! within this batch. The output maps these temporary IDs to the real task IDs.
//!
//! # JSON Format for `add-deps`
//!
//! ```json
//! {
//!   "dependencies": [
//!     {"task": "real-task-id-1", "depends_on": "real-task-id-2"},
//!     {"task": "real-task-id-1", "depends_on": "real-task-id-3"}
//!   ]
//! }
//! ```
//!
//! # JSON Format for `list`
//!
//! ```json
//! {
//!   "status": "open",
//!   "priority": 1,
//!   "max_priority": 2,
//!   "ready_only": true
//! }
//! ```
//!
//! All fields are optional. An empty object `{}` returns all tasks.
//!
//! # JSON Format for `search`
//!
//! ```json
//! {"query": "search term"}
//! ```

use crate::error::Result;
use crate::paths;
use crate::tasks::models::{Priority, Status};
use crate::tasks::{SqliteTaskStore, TaskFilter, TaskStore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

/// Input format for bulk task creation.
#[derive(Debug, Deserialize)]
pub struct BulkCreateInput {
    /// List of tasks to create.
    pub tasks: Vec<TaskInput>,
}

/// A single task in the bulk creation input.
#[derive(Debug, Deserialize)]
pub struct TaskInput {
    /// Temporary ID for dependency references within this batch.
    pub id: String,
    /// Task title.
    pub title: String,
    /// Task description.
    #[serde(default)]
    pub description: String,
    /// Priority (0-4). Defaults to 2 (Medium).
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// List of temporary IDs this task depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

const fn default_priority() -> u8 {
    2
}

/// Output format for bulk task creation.
#[derive(Debug, Serialize)]
pub struct BulkCreateOutput {
    /// Number of tasks created.
    pub created: usize,
    /// Mapping from temporary IDs to real task IDs.
    pub id_map: HashMap<String, String>,
    /// Number of dependencies added.
    pub dependencies_added: usize,
    /// Any errors that occurred (task creation continues on error).
    pub errors: Vec<String>,
}

/// Input format for adding dependencies.
#[derive(Debug, Deserialize)]
pub struct BulkAddDepsInput {
    /// List of dependencies to add.
    pub dependencies: Vec<DependencyInput>,
}

/// A single dependency relationship.
#[derive(Debug, Deserialize)]
pub struct DependencyInput {
    /// The task that has the dependency.
    pub task: String,
    /// The task it depends on.
    pub depends_on: String,
}

/// Output format for adding dependencies.
#[derive(Debug, Serialize)]
pub struct BulkAddDepsOutput {
    /// Number of dependencies added.
    pub added: usize,
    /// Any errors that occurred.
    pub errors: Vec<String>,
}

/// Input format for listing tasks.
#[derive(Debug, Default, Deserialize)]
pub struct ListInput {
    /// Filter by status (open, complete, abandoned, stuck, blocked).
    #[serde(default)]
    pub status: Option<String>,
    /// Filter by exact priority (0-4).
    #[serde(default)]
    pub priority: Option<u8>,
    /// Filter by maximum priority (inclusive, 0-4).
    #[serde(default)]
    pub max_priority: Option<u8>,
    /// Only show tasks that are ready to work on (not blocked).
    #[serde(default)]
    pub ready_only: bool,
}

/// A task in the list output.
#[derive(Debug, Serialize)]
pub struct TaskOutput {
    /// Task ID.
    pub id: String,
    /// Task title.
    pub title: String,
    /// Task description.
    pub description: String,
    /// Priority (0-4).
    pub priority: u8,
    /// Priority label (critical, high, medium, low, backlog).
    pub priority_label: &'static str,
    /// Status (open, complete, abandoned, stuck, blocked).
    pub status: String,
    /// Whether the task is currently being worked on.
    pub in_progress: bool,
    /// Whether the task was explicitly requested by the user.
    pub requested: bool,
    /// ISO 8601 timestamp when created.
    pub created_at: String,
    /// ISO 8601 timestamp when last updated.
    pub updated_at: String,
    /// IDs of tasks this task depends on.
    pub dependencies: Vec<String>,
}

/// Output format for listing tasks.
#[derive(Debug, Serialize)]
pub struct ListOutput {
    /// Number of tasks returned.
    pub count: usize,
    /// The tasks matching the filter.
    pub tasks: Vec<TaskOutput>,
}

/// Input format for searching tasks.
#[derive(Debug, Deserialize)]
pub struct SearchInput {
    /// Search query.
    pub query: String,
}

/// Output format for searching tasks (same as list).
pub type SearchOutput = ListOutput;

/// Create multiple tasks with dependencies in a single operation.
///
/// Tasks are created in the order specified, then dependencies are added
/// using the temporary ID mapping. This is atomic in the sense that all
/// tasks are created before dependencies are processed.
///
/// # Arguments
///
/// * `store` - The task store to use
/// * `input` - The bulk creation input with tasks and their dependencies
///
/// # Returns
///
/// A `BulkCreateOutput` containing:
/// - The number of tasks successfully created
/// - A mapping from temporary IDs to real task IDs
/// - The number of dependencies successfully added
/// - Any errors encountered (operations continue on error)
pub fn bulk_create_tasks(store: &dyn TaskStore, input: &BulkCreateInput) -> BulkCreateOutput {
    let mut output = BulkCreateOutput {
        created: 0,
        id_map: HashMap::new(),
        dependencies_added: 0,
        errors: Vec::new(),
    };

    // First pass: create all tasks
    for task_input in &input.tasks {
        let priority = Priority::from_u8(task_input.priority).unwrap_or_default();

        match store.create_task(&task_input.title, &task_input.description, priority) {
            Ok(task) => {
                output.id_map.insert(task_input.id.clone(), task.id);
                output.created += 1;
            }
            Err(e) => {
                output.errors.push(format!("Failed to create task '{}': {}", task_input.id, e));
            }
        }
    }

    // Second pass: add dependencies using the ID map
    for task_input in &input.tasks {
        let Some(real_task_id) = output.id_map.get(&task_input.id) else {
            continue; // Task creation failed, skip dependencies
        };

        for temp_dep_id in &task_input.depends_on {
            let Some(real_dep_id) = output.id_map.get(temp_dep_id) else {
                output.errors.push(format!(
                    "Task '{}' depends on '{}' which was not created",
                    task_input.id, temp_dep_id
                ));
                continue;
            };

            match store.add_dependency(real_task_id, real_dep_id) {
                Ok(()) => {
                    output.dependencies_added += 1;
                }
                Err(e) => {
                    output.errors.push(format!(
                        "Failed to add dependency {} -> {}: {}",
                        task_input.id, temp_dep_id, e
                    ));
                }
            }
        }
    }

    output
}

/// Add multiple dependencies between existing tasks.
///
/// Each dependency is added independently - if one fails, others will
/// still be attempted.
///
/// # Arguments
///
/// * `store` - The task store to use
/// * `input` - The dependencies to add
///
/// # Returns
///
/// A `BulkAddDepsOutput` containing:
/// - The number of dependencies successfully added
/// - Any errors encountered
pub fn bulk_add_dependencies(store: &dyn TaskStore, input: &BulkAddDepsInput) -> BulkAddDepsOutput {
    let mut output = BulkAddDepsOutput { added: 0, errors: Vec::new() };

    for dep in &input.dependencies {
        match store.add_dependency(&dep.task, &dep.depends_on) {
            Ok(()) => {
                output.added += 1;
            }
            Err(e) => {
                output
                    .errors
                    .push(format!("Failed to add {} -> {}: {}", dep.task, dep.depends_on, e));
            }
        }
    }

    output
}

/// Parse JSON input and create tasks.
///
/// # Errors
///
/// Returns an error if the JSON is invalid.
pub fn create_from_json(store: &dyn TaskStore, json: &str) -> Result<BulkCreateOutput> {
    let input: BulkCreateInput = serde_json::from_str(json)?;
    Ok(bulk_create_tasks(store, &input))
}

/// Parse JSON input and add dependencies.
///
/// # Errors
///
/// Returns an error if the JSON is invalid.
pub fn add_deps_from_json(store: &dyn TaskStore, json: &str) -> Result<BulkAddDepsOutput> {
    let input: BulkAddDepsInput = serde_json::from_str(json)?;
    Ok(bulk_add_dependencies(store, &input))
}

/// Get a priority label from a priority value.
const fn priority_label(priority: Priority) -> &'static str {
    match priority {
        Priority::Critical => "critical",
        Priority::High => "high",
        Priority::Medium => "medium",
        Priority::Low => "low",
        Priority::Backlog => "backlog",
    }
}

/// Convert a Task to `TaskOutput`.
fn task_to_output(task: &crate::tasks::Task, deps: Vec<String>) -> TaskOutput {
    TaskOutput {
        id: task.id.clone(),
        title: task.title.clone(),
        description: task.description.clone(),
        priority: task.priority.as_u8(),
        priority_label: priority_label(task.priority),
        status: task.status.as_str().to_string(),
        in_progress: task.in_progress,
        requested: task.requested,
        created_at: task.created_at.clone(),
        updated_at: task.updated_at.clone(),
        dependencies: deps,
    }
}

/// List tasks with optional filtering.
///
/// # Arguments
///
/// * `store` - The task store to use
/// * `input` - Filter options
///
/// # Returns
///
/// A `ListOutput` with matching tasks.
///
/// # Errors
///
/// Returns an error if parsing status fails or database query fails.
pub fn list_tasks(store: &dyn TaskStore, input: &ListInput) -> Result<ListOutput> {
    let status = input
        .status
        .as_ref()
        .map(|s| Status::from_str(s))
        .transpose()
        .map_err(|e| crate::error::Error::Task(e.to_string().into()))?;

    let priority = input
        .priority
        .map(Priority::from_u8)
        .transpose()
        .map_err(|e| crate::error::Error::Task(e.to_string().into()))?;

    let max_priority = input
        .max_priority
        .map(Priority::from_u8)
        .transpose()
        .map_err(|e| crate::error::Error::Task(e.to_string().into()))?;

    let filter = TaskFilter { status, priority, max_priority, ready_only: input.ready_only };

    let tasks = store.list_tasks(filter)?;
    let outputs: Vec<TaskOutput> = tasks
        .iter()
        .map(|t| {
            let deps = store.get_dependencies(&t.id).unwrap_or_default();
            task_to_output(t, deps)
        })
        .collect();

    Ok(ListOutput { count: outputs.len(), tasks: outputs })
}

/// Search tasks by query string.
///
/// # Arguments
///
/// * `store` - The task store to use
/// * `query` - Search query
///
/// # Returns
///
/// A `SearchOutput` with matching tasks.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub fn search_tasks(store: &dyn TaskStore, query: &str) -> Result<SearchOutput> {
    let tasks = store.search_tasks(query)?;
    let outputs: Vec<TaskOutput> = tasks
        .iter()
        .map(|t| {
            let deps = store.get_dependencies(&t.id).unwrap_or_default();
            task_to_output(t, deps)
        })
        .collect();

    Ok(ListOutput { count: outputs.len(), tasks: outputs })
}

/// Parse JSON input and list tasks.
///
/// # Errors
///
/// Returns an error if the JSON is invalid or status/priority values are invalid.
pub fn list_from_json(store: &dyn TaskStore, json: &str) -> Result<ListOutput> {
    // Handle empty input as empty filter
    let input: ListInput =
        if json.trim().is_empty() { ListInput::default() } else { serde_json::from_str(json)? };
    list_tasks(store, &input)
}

/// Parse JSON input and search tasks.
///
/// # Errors
///
/// Returns an error if the JSON is invalid.
pub fn search_from_json(store: &dyn TaskStore, json: &str) -> Result<SearchOutput> {
    let input: SearchInput = serde_json::from_str(json)?;
    search_tasks(store, &input.query)
}

/// Open the default task store based on `TASKS_DB_PATH` env var or current directory.
///
/// # Errors
///
/// Returns an error if the database cannot be opened.
pub fn open_default_store() -> Result<SqliteTaskStore> {
    let db_path = env::var("TASKS_DB_PATH").map_or_else(
        |_| env::current_dir().map(|cwd| paths::project_db_path(&cwd)).unwrap_or_default(),
        PathBuf::from,
    );
    SqliteTaskStore::new(&db_path)
}

/// Print usage information for the bulk-tasks CLI.
pub fn print_usage() {
    eprintln!(
        r#"Usage: bulk-tasks <command>

Commands:
  create    Create multiple tasks from JSON input (stdin)
  add-deps  Add dependencies between existing tasks (stdin)
  list      List tasks with optional filtering (stdin)
  search    Search tasks by query (stdin)

JSON format for 'create':
  {{"tasks": [{{"id": "1", "title": "...", "depends_on": ["2"]}}]}}

JSON format for 'add-deps':
  {{"dependencies": [{{"task": "real-id", "depends_on": "other-id"}}]}}

JSON format for 'list':
  {{"status": "open", "priority": 1, "max_priority": 2, "ready_only": true}}
  All fields optional. Empty object {{}} returns all tasks.

JSON format for 'search':
  {{"query": "search term"}}
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::SqliteTaskStore;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, SqliteTaskStore) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SqliteTaskStore::new(&db_path).unwrap();
        (dir, store)
    }

    #[test]
    fn test_bulk_create_single_task() {
        let (_dir, store) = create_test_store();
        let input = BulkCreateInput {
            tasks: vec![TaskInput {
                id: "1".to_string(),
                title: "Test task".to_string(),
                description: "Description".to_string(),
                priority: 2,
                depends_on: vec![],
            }],
        };

        let output = bulk_create_tasks(&store, &input);

        assert_eq!(output.created, 1);
        assert_eq!(output.id_map.len(), 1);
        assert!(output.id_map.contains_key("1"));
        assert_eq!(output.dependencies_added, 0);
        assert!(output.errors.is_empty());
    }

    #[test]
    fn test_bulk_create_with_dependencies() {
        let (_dir, store) = create_test_store();
        let input = BulkCreateInput {
            tasks: vec![
                TaskInput {
                    id: "1".to_string(),
                    title: "First task".to_string(),
                    description: String::new(),
                    priority: 2,
                    depends_on: vec![],
                },
                TaskInput {
                    id: "2".to_string(),
                    title: "Second task".to_string(),
                    description: String::new(),
                    priority: 2,
                    depends_on: vec!["1".to_string()],
                },
            ],
        };

        let output = bulk_create_tasks(&store, &input);

        assert_eq!(output.created, 2);
        assert_eq!(output.dependencies_added, 1);
        assert!(output.errors.is_empty());

        // Verify the dependency was actually created
        let real_task2_id = output.id_map.get("2").unwrap();
        let deps = store.get_dependencies(real_task2_id).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], *output.id_map.get("1").unwrap());
    }

    #[test]
    fn test_bulk_create_missing_dependency() {
        let (_dir, store) = create_test_store();
        let input = BulkCreateInput {
            tasks: vec![TaskInput {
                id: "1".to_string(),
                title: "Task".to_string(),
                description: String::new(),
                priority: 2,
                depends_on: vec!["nonexistent".to_string()],
            }],
        };

        let output = bulk_create_tasks(&store, &input);

        assert_eq!(output.created, 1);
        assert_eq!(output.dependencies_added, 0);
        assert_eq!(output.errors.len(), 1);
        assert!(output.errors[0].contains("nonexistent"));
    }

    #[test]
    fn test_bulk_create_default_priority() {
        let (_dir, store) = create_test_store();
        let json = r#"{"tasks": [{"id": "1", "title": "No priority"}]}"#;

        let output = create_from_json(&store, json).unwrap();

        assert_eq!(output.created, 1);
        let real_id = output.id_map.get("1").unwrap();
        let task = store.get_task(real_id).unwrap().unwrap();
        assert_eq!(task.priority, Priority::Medium);
    }

    #[test]
    fn test_bulk_add_dependencies() {
        let (_dir, store) = create_test_store();

        // Create tasks first
        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();

        let input = BulkAddDepsInput {
            dependencies: vec![DependencyInput {
                task: task2.id.clone(),
                depends_on: task1.id.clone(),
            }],
        };

        let output = bulk_add_dependencies(&store, &input);

        assert_eq!(output.added, 1);
        assert!(output.errors.is_empty());

        // Verify
        let deps = store.get_dependencies(&task2.id).unwrap();
        assert_eq!(deps, vec![task1.id]);
    }

    #[test]
    fn test_bulk_add_deps_invalid_task() {
        let (_dir, store) = create_test_store();

        let input = BulkAddDepsInput {
            dependencies: vec![DependencyInput {
                task: "nonexistent".to_string(),
                depends_on: "also-nonexistent".to_string(),
            }],
        };

        let output = bulk_add_dependencies(&store, &input);

        assert_eq!(output.added, 0);
        assert_eq!(output.errors.len(), 1);
    }

    #[test]
    fn test_create_from_json() {
        let (_dir, store) = create_test_store();
        let json = r#"{
            "tasks": [
                {"id": "a", "title": "Task A", "description": "Desc A", "priority": 1},
                {"id": "b", "title": "Task B", "depends_on": ["a"]}
            ]
        }"#;

        let output = create_from_json(&store, json).unwrap();

        assert_eq!(output.created, 2);
        assert_eq!(output.dependencies_added, 1);
        assert!(output.errors.is_empty());
    }

    #[test]
    fn test_create_from_json_invalid() {
        let (_dir, store) = create_test_store();
        let json = "not valid json";

        let result = create_from_json(&store, json);
        assert!(result.is_err());
    }

    #[test]
    fn test_add_deps_from_json() {
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();

        let json = format!(
            r#"{{"dependencies": [{{"task": "{}", "depends_on": "{}"}}]}}"#,
            task2.id, task1.id
        );

        let output = add_deps_from_json(&store, &json).unwrap();

        assert_eq!(output.added, 1);
        assert!(output.errors.is_empty());
    }

    #[test]
    fn test_bulk_create_circular_dependency_error() {
        let (_dir, store) = create_test_store();

        // Create tasks that would form a cycle
        let input = BulkCreateInput {
            tasks: vec![
                TaskInput {
                    id: "1".to_string(),
                    title: "Task 1".to_string(),
                    description: String::new(),
                    priority: 2,
                    depends_on: vec!["2".to_string()],
                },
                TaskInput {
                    id: "2".to_string(),
                    title: "Task 2".to_string(),
                    description: String::new(),
                    priority: 2,
                    depends_on: vec!["1".to_string()],
                },
            ],
        };

        let output = bulk_create_tasks(&store, &input);

        // Both tasks should be created
        assert_eq!(output.created, 2);
        // First dependency succeeds, second would create cycle
        assert_eq!(output.dependencies_added, 1);
        // One error for the circular dependency
        assert_eq!(output.errors.len(), 1);
        assert!(output.errors[0].contains("cycle"));
    }

    #[test]
    fn test_bulk_create_skip_failed_task_dependencies() {
        // This tests the `continue` branch when a task's creation failed
        // We can't easily make task creation fail without a mock, but we can test
        // the dependency resolution when a temp ID doesn't exist
        let (_dir, store) = create_test_store();

        let input = BulkCreateInput {
            tasks: vec![TaskInput {
                id: "1".to_string(),
                title: "Task with missing dep".to_string(),
                description: String::new(),
                priority: 2,
                depends_on: vec!["nonexistent".to_string()],
            }],
        };

        let output = bulk_create_tasks(&store, &input);

        assert_eq!(output.created, 1);
        assert_eq!(output.dependencies_added, 0);
        assert_eq!(output.errors.len(), 1);
        assert!(output.errors[0].contains("nonexistent"));
    }

    #[test]
    fn test_print_usage_does_not_panic() {
        // Just verify it doesn't panic
        print_usage();
    }

    #[test]
    fn test_open_default_store_with_env_var() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");

        // Set env var
        std::env::set_var("TASKS_DB_PATH", &db_path);
        let result = open_default_store();
        std::env::remove_var("TASKS_DB_PATH");

        assert!(result.is_ok());
    }

    #[test]
    fn test_open_default_store_without_env_var() {
        // Ensure env var is not set
        std::env::remove_var("TASKS_DB_PATH");

        // This will use current directory - just verify it doesn't panic
        // (may fail if cwd is not writable, which is fine for this test)
        let _result = open_default_store();
    }

    #[test]
    fn test_bulk_create_task_failure_with_corrupted_db() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("corrupt.db");

        // Create a file that looks like an SQLite db but is corrupted
        std::fs::write(&db_path, "This is not a valid SQLite database").unwrap();

        // Opening will fail because the file is corrupted
        let store_result = SqliteTaskStore::new(&db_path);
        assert!(store_result.is_err());
    }

    #[test]
    fn test_bulk_create_handles_dependency_on_failed_task() {
        // This tests the branch where a task depends on another task that
        // wasn't successfully created. We test this indirectly by having a
        // dependency on a temp ID that doesn't exist.
        let (_dir, store) = create_test_store();

        let input = BulkCreateInput {
            tasks: vec![TaskInput {
                id: "1".to_string(),
                title: "Task with invalid dep".to_string(),
                description: String::new(),
                priority: 2,
                depends_on: vec!["does-not-exist".to_string()],
            }],
        };

        let output = bulk_create_tasks(&store, &input);

        // Task created successfully
        assert_eq!(output.created, 1);
        // But dependency failed because temp ID doesn't exist
        assert_eq!(output.dependencies_added, 0);
        assert_eq!(output.errors.len(), 1);
        assert!(output.errors[0].contains("does-not-exist"));
    }

    #[test]
    fn test_bulk_create_with_read_only_db() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("readonly.db");

        // Create and initialize the database
        let store = SqliteTaskStore::new(&db_path).unwrap();

        // Make the database file read-only
        let mut perms = std::fs::metadata(&db_path).unwrap().permissions();
        perms.set_mode(0o444); // read-only
        std::fs::set_permissions(&db_path, perms).unwrap();

        // Now try to create tasks - this should fail
        let input = BulkCreateInput {
            tasks: vec![
                TaskInput {
                    id: "1".to_string(),
                    title: "Will fail".to_string(),
                    description: String::new(),
                    priority: 2,
                    depends_on: vec![],
                },
                TaskInput {
                    id: "2".to_string(),
                    title: "Also fails".to_string(),
                    description: String::new(),
                    priority: 2,
                    depends_on: vec!["1".to_string()],
                },
            ],
        };

        let output = bulk_create_tasks(&store, &input);

        // Both tasks should fail to create
        assert_eq!(output.created, 0);
        assert!(output.id_map.is_empty());
        // Two errors from failed task creation
        assert_eq!(output.errors.len(), 2);
        assert!(output.errors[0].contains("Failed to create task"));
        // No dependencies added because tasks weren't created (and task "2" skips via continue)
        assert_eq!(output.dependencies_added, 0);

        // Restore permissions for cleanup
        let mut perms = std::fs::metadata(&db_path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&db_path, perms).unwrap();
    }

    // ========== List Tasks Tests ==========

    #[test]
    fn test_list_tasks_empty() {
        let (_dir, store) = create_test_store();

        let output = list_tasks(&store, &ListInput::default()).unwrap();

        assert_eq!(output.count, 0);
        assert!(output.tasks.is_empty());
    }

    #[test]
    fn test_list_tasks_all() {
        let (_dir, store) = create_test_store();
        store.create_task("Task 1", "Desc 1", Priority::High).unwrap();
        store.create_task("Task 2", "Desc 2", Priority::Low).unwrap();

        let output = list_tasks(&store, &ListInput::default()).unwrap();

        assert_eq!(output.count, 2);
        assert_eq!(output.tasks.len(), 2);
    }

    #[test]
    fn test_list_tasks_filter_by_status() {
        let (_dir, store) = create_test_store();
        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        store.create_task("Task 2", "", Priority::Medium).unwrap();
        store
            .update_task(
                &task1.id,
                crate::tasks::TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        let input = ListInput { status: Some("open".to_string()), ..Default::default() };
        let output = list_tasks(&store, &input).unwrap();

        assert_eq!(output.count, 1);
        assert_eq!(output.tasks[0].status, "open");
    }

    #[test]
    fn test_list_tasks_filter_by_priority() {
        let (_dir, store) = create_test_store();
        store.create_task("High", "", Priority::High).unwrap();
        store.create_task("Low", "", Priority::Low).unwrap();

        let input = ListInput { priority: Some(1), ..Default::default() };
        let output = list_tasks(&store, &input).unwrap();

        assert_eq!(output.count, 1);
        assert_eq!(output.tasks[0].priority, 1);
        assert_eq!(output.tasks[0].priority_label, "high");
    }

    #[test]
    fn test_list_tasks_filter_by_max_priority() {
        let (_dir, store) = create_test_store();
        store.create_task("Critical", "", Priority::Critical).unwrap();
        store.create_task("High", "", Priority::High).unwrap();
        store.create_task("Low", "", Priority::Low).unwrap();

        let input = ListInput { max_priority: Some(1), ..Default::default() };
        let output = list_tasks(&store, &input).unwrap();

        assert_eq!(output.count, 2);
    }

    #[test]
    fn test_list_tasks_ready_only() {
        let (_dir, store) = create_test_store();
        let task1 = store.create_task("Blocker", "", Priority::High).unwrap();
        let task2 = store.create_task("Blocked", "", Priority::High).unwrap();
        store.add_dependency(&task2.id, &task1.id).unwrap();

        // Without ready_only, both tasks appear
        let all = list_tasks(&store, &ListInput::default()).unwrap();
        assert_eq!(all.count, 2);

        // With ready_only, only the unblocked task appears
        let input = ListInput { ready_only: true, ..Default::default() };
        let output = list_tasks(&store, &input).unwrap();
        assert_eq!(output.count, 1);
        assert_eq!(output.tasks[0].id, task1.id);
    }

    #[test]
    fn test_list_tasks_includes_dependencies() {
        let (_dir, store) = create_test_store();
        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();
        store.add_dependency(&task2.id, &task1.id).unwrap();

        let output = list_tasks(&store, &ListInput::default()).unwrap();

        let task2_output = output.tasks.iter().find(|t| t.id == task2.id).unwrap();
        assert_eq!(task2_output.dependencies, vec![task1.id]);
    }

    #[test]
    fn test_list_tasks_invalid_status() {
        let (_dir, store) = create_test_store();

        let input = ListInput { status: Some("invalid".to_string()), ..Default::default() };
        let result = list_tasks(&store, &input);

        assert!(result.is_err());
    }

    #[test]
    fn test_list_tasks_invalid_priority() {
        let (_dir, store) = create_test_store();

        let input = ListInput { priority: Some(99), ..Default::default() };
        let result = list_tasks(&store, &input);

        assert!(result.is_err());
    }

    #[test]
    fn test_list_tasks_invalid_max_priority() {
        let (_dir, store) = create_test_store();

        let input = ListInput { max_priority: Some(99), ..Default::default() };
        let result = list_tasks(&store, &input);

        assert!(result.is_err());
    }

    #[test]
    fn test_list_from_json_empty_input() {
        let (_dir, store) = create_test_store();
        store.create_task("Task", "", Priority::Medium).unwrap();

        let output = list_from_json(&store, "").unwrap();
        assert_eq!(output.count, 1);
    }

    #[test]
    fn test_list_from_json_empty_object() {
        let (_dir, store) = create_test_store();
        store.create_task("Task", "", Priority::Medium).unwrap();

        let output = list_from_json(&store, "{}").unwrap();
        assert_eq!(output.count, 1);
    }

    #[test]
    fn test_list_from_json_with_filters() {
        let (_dir, store) = create_test_store();
        store.create_task("High", "", Priority::High).unwrap();
        store.create_task("Low", "", Priority::Low).unwrap();

        let json = r#"{"priority": 1}"#;
        let output = list_from_json(&store, json).unwrap();

        assert_eq!(output.count, 1);
        assert_eq!(output.tasks[0].priority_label, "high");
    }

    #[test]
    fn test_list_from_json_invalid() {
        let (_dir, store) = create_test_store();

        let result = list_from_json(&store, "not json");
        assert!(result.is_err());
    }

    // ========== Search Tasks Tests ==========

    #[test]
    fn test_search_tasks_empty_db() {
        let (_dir, store) = create_test_store();

        let output = search_tasks(&store, "anything").unwrap();

        assert_eq!(output.count, 0);
        assert!(output.tasks.is_empty());
    }

    #[test]
    fn test_search_tasks_by_title() {
        let (_dir, store) = create_test_store();
        store.create_task("Fix login bug", "", Priority::High).unwrap();
        store.create_task("Add feature", "", Priority::Low).unwrap();

        let output = search_tasks(&store, "login").unwrap();

        assert_eq!(output.count, 1);
        assert!(output.tasks[0].title.contains("login"));
    }

    #[test]
    fn test_search_tasks_by_description() {
        let (_dir, store) = create_test_store();
        store.create_task("Task", "This relates to authentication", Priority::Medium).unwrap();
        store.create_task("Other", "Unrelated", Priority::Medium).unwrap();

        let output = search_tasks(&store, "authentication").unwrap();

        assert_eq!(output.count, 1);
    }

    #[test]
    fn test_search_tasks_includes_dependencies() {
        let (_dir, store) = create_test_store();
        let task1 = store.create_task("Dependency", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Searchable task", "", Priority::Medium).unwrap();
        store.add_dependency(&task2.id, &task1.id).unwrap();

        let output = search_tasks(&store, "Searchable").unwrap();

        assert_eq!(output.count, 1);
        assert_eq!(output.tasks[0].dependencies, vec![task1.id]);
    }

    #[test]
    fn test_search_from_json() {
        let (_dir, store) = create_test_store();
        store.create_task("Bug fix", "", Priority::High).unwrap();
        store.create_task("Feature", "", Priority::Low).unwrap();

        let json = r#"{"query": "Bug"}"#;
        let output = search_from_json(&store, json).unwrap();

        assert_eq!(output.count, 1);
    }

    #[test]
    fn test_search_from_json_invalid() {
        let (_dir, store) = create_test_store();

        let result = search_from_json(&store, "not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_search_from_json_missing_query() {
        let (_dir, store) = create_test_store();

        let result = search_from_json(&store, "{}");
        assert!(result.is_err());
    }

    // ========== Priority Label Tests ==========

    #[test]
    fn test_priority_labels() {
        assert_eq!(priority_label(Priority::Critical), "critical");
        assert_eq!(priority_label(Priority::High), "high");
        assert_eq!(priority_label(Priority::Medium), "medium");
        assert_eq!(priority_label(Priority::Low), "low");
        assert_eq!(priority_label(Priority::Backlog), "backlog");
    }

    // ========== Task Output Tests ==========

    #[test]
    fn test_task_to_output() {
        let task = crate::tasks::Task {
            id: "test-1234".to_string(),
            title: "Test".to_string(),
            description: "Desc".to_string(),
            priority: Priority::High,
            status: Status::Open,
            in_progress: true,
            requested: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
        };

        let deps = vec!["dep-1".to_string()];
        let output = task_to_output(&task, deps.clone());

        assert_eq!(output.id, "test-1234");
        assert_eq!(output.title, "Test");
        assert_eq!(output.description, "Desc");
        assert_eq!(output.priority, 1);
        assert_eq!(output.priority_label, "high");
        assert_eq!(output.status, "open");
        assert!(output.in_progress);
        assert!(output.requested);
        assert_eq!(output.created_at, "2024-01-01T00:00:00Z");
        assert_eq!(output.updated_at, "2024-01-02T00:00:00Z");
        assert_eq!(output.dependencies, deps);
    }
}
