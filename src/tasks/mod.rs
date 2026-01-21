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

pub use models::{AuditEntry, InvalidPriority, InvalidStatus, Note, Priority, Status, Task};
pub use store::{
    CircularDependency, SqliteTaskStore, TaskFilter, TaskNotFound, TaskStore, TaskUpdate,
};
