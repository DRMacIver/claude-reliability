//! MCP server for task management.
//!
//! This module provides an MCP server that exposes task management
//! functionality through the Model Context Protocol.

// The rmcp `#[tool(aggr)]` macro requires ownership of input structs,
// making pass-by-value necessary for all tool handler functions.
#![allow(clippy::needless_pass_by_value)]

use crate::tasks::{
    HowToUpdate, Priority, SqliteTaskStore, Status, TaskFilter, TaskStore, TaskUpdate,
};
use rmcp::model::{
    CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::tool;
use rmcp::Error as McpError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// MCP server for task management.
#[derive(Clone)]
pub struct TasksServer {
    store: Arc<SqliteTaskStore>,
}

impl TasksServer {
    /// Create a new tasks server with the given database path.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be initialized.
    pub fn new(db_path: &Path) -> crate::error::Result<Self> {
        let store = SqliteTaskStore::new(db_path)?;
        Ok(Self { store: Arc::new(store) })
    }

    /// Create a new tasks server for the given project directory.
    ///
    /// The database will be at `~/.claude-reliability/projects/<hash>/working-memory.sqlite3`.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be initialized.
    pub fn for_project(project_dir: &Path) -> crate::error::Result<Self> {
        let store = SqliteTaskStore::for_project(project_dir)?;
        Ok(Self { store: Arc::new(store) })
    }
}

// Tool input schemas

/// Input for creating a task.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTaskInput {
    /// Task title (required).
    pub title: String,
    /// Task description.
    #[serde(default)]
    pub description: String,
    /// Priority: 0=critical, 1=high, 2=medium (default), 3=low, 4=backlog.
    #[serde(default = "default_priority")]
    pub priority: u8,
}

const fn default_priority() -> u8 {
    2
}

/// Input for getting a task.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTaskInput {
    /// Task ID.
    pub id: String,
}

/// Input for updating a task.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTaskInput {
    /// Task ID.
    pub id: String,
    /// New title (optional).
    pub title: Option<String>,
    /// New description (optional).
    pub description: Option<String>,
    /// New priority (optional).
    pub priority: Option<u8>,
    /// New status: open, complete, abandoned, stuck, blocked (optional).
    pub status: Option<String>,
}

/// Input for deleting a task.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteTaskInput {
    /// Task ID.
    pub id: String,
}

/// Input for listing tasks.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTasksInput {
    /// Filter by status (optional).
    pub status: Option<String>,
    /// Filter by exact priority (optional).
    pub priority: Option<u8>,
    /// Filter by maximum priority (optional).
    pub max_priority: Option<u8>,
    /// Only show tasks that are ready to work on (optional).
    #[serde(default)]
    pub ready_only: bool,
}

/// Input for adding a dependency.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddDependencyInput {
    /// Task ID that will have the dependency.
    pub task_id: String,
    /// Task ID that must be completed first.
    pub depends_on: String,
}

/// Input for removing a dependency.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveDependencyInput {
    /// Task ID that has the dependency.
    pub task_id: String,
    /// Task ID to remove as dependency.
    pub depends_on: String,
}

/// Input for adding a note.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddNoteInput {
    /// Task ID.
    pub task_id: String,
    /// Note content.
    pub content: String,
}

/// Input for getting notes.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNotesInput {
    /// Task ID.
    pub task_id: String,
}

/// Input for searching tasks.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchTasksInput {
    /// Search query.
    pub query: String,
}

/// Input for getting audit log.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAuditLogInput {
    /// Filter by task ID (optional).
    pub task_id: Option<String>,
    /// Limit number of entries (optional).
    pub limit: Option<usize>,
}

/// Input for creating a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateHowToInput {
    /// How-to title (required).
    pub title: String,
    /// Instructions for how to perform the task.
    #[serde(default)]
    pub instructions: String,
}

/// Input for getting a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetHowToInput {
    /// How-to ID.
    pub id: String,
}

/// Input for updating a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateHowToInput {
    /// How-to ID.
    pub id: String,
    /// New title (optional).
    pub title: Option<String>,
    /// New instructions (optional).
    pub instructions: Option<String>,
}

/// Input for deleting a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteHowToInput {
    /// How-to ID.
    pub id: String,
}

/// Input for searching how-tos.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchHowTosInput {
    /// Search query.
    pub query: String,
}

/// Input for linking a task to a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkTaskToHowToInput {
    /// Task ID.
    pub task_id: String,
    /// How-to ID.
    pub howto_id: String,
}

/// Input for unlinking a task from a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkTaskFromHowToInput {
    /// Task ID.
    pub task_id: String,
    /// How-to ID.
    pub howto_id: String,
}

/// Input for creating a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateQuestionInput {
    /// The question text (required).
    pub text: String,
}

/// Input for getting a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetQuestionInput {
    /// Question ID.
    pub id: String,
}

/// Input for answering a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnswerQuestionInput {
    /// Question ID.
    pub id: String,
    /// The answer to the question.
    pub answer: String,
}

/// Input for deleting a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteQuestionInput {
    /// Question ID.
    pub id: String,
}

/// Input for listing questions.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListQuestionsInput {
    /// If true, only return unanswered questions.
    #[serde(default)]
    pub unanswered_only: bool,
}

/// Input for searching questions.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchQuestionsInput {
    /// Search query.
    pub query: String,
}

/// Input for linking a task to a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkTaskToQuestionInput {
    /// Task ID.
    pub task_id: String,
    /// Question ID.
    pub question_id: String,
}

/// Input for unlinking a task from a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkTaskFromQuestionInput {
    /// Task ID.
    pub task_id: String,
    /// Question ID.
    pub question_id: String,
}

/// Input for getting blocking questions for a task.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBlockingQuestionsInput {
    /// Task ID.
    pub task_id: String,
}

/// Input for starting work on a task.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkOnInput {
    /// Task ID to start working on.
    pub task_id: String,
}

/// Input for requesting specific tasks.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RequestTasksInput {
    /// Task IDs to mark as requested.
    pub task_ids: Vec<String>,
}

// Output types - defined at module level to avoid items_after_statements

/// Task output representation.
#[derive(Debug, Serialize)]
struct TaskOutput {
    id: String,
    title: String,
    description: String,
    priority: u8,
    priority_label: &'static str,
    status: String,
    in_progress: bool,
    requested: bool,
    created_at: String,
    updated_at: String,
    dependencies: Vec<String>,
    guidance: Vec<String>,
}

impl TaskOutput {
    fn from_task(task: &crate::tasks::Task, deps: Vec<String>, guidance: Vec<String>) -> Self {
        Self {
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
            guidance,
        }
    }
}

/// Note output for serialization.
#[derive(Debug, Serialize)]
struct NoteOutput {
    id: i64,
    content: String,
    created_at: String,
}

/// Note output with task ID for serialization.
#[derive(Debug, Serialize)]
struct NoteWithTaskOutput {
    id: i64,
    task_id: String,
    content: String,
    created_at: String,
}

/// Full task with notes for `get_task` response.
#[derive(Debug, Serialize)]
struct FullTaskOutput {
    #[serde(flatten)]
    task: TaskOutput,
    notes: Vec<NoteOutput>,
}

/// Task suggestion for `what_should_i_work_on` response.
#[derive(Debug, Serialize)]
struct TaskSuggestion {
    #[serde(flatten)]
    task: TaskOutput,
    notes: Vec<NoteOutput>,
    message: String,
}

/// How-to output representation.
#[derive(Debug, Serialize)]
struct HowToOutput {
    id: String,
    title: String,
    instructions: String,
    created_at: String,
    updated_at: String,
}

impl HowToOutput {
    fn from_howto(howto: &crate::tasks::HowTo) -> Self {
        Self {
            id: howto.id.clone(),
            title: howto.title.clone(),
            instructions: howto.instructions.clone(),
            created_at: howto.created_at.clone(),
            updated_at: howto.updated_at.clone(),
        }
    }
}

/// Question output representation.
#[derive(Debug, Serialize)]
struct QuestionOutput {
    id: String,
    text: String,
    answer: Option<String>,
    is_answered: bool,
    created_at: String,
    answered_at: Option<String>,
}

impl QuestionOutput {
    fn from_question(q: &crate::tasks::Question) -> Self {
        Self {
            id: q.id.clone(),
            text: q.text.clone(),
            answer: q.answer.clone(),
            is_answered: q.is_answered(),
            created_at: q.created_at.clone(),
            answered_at: q.answered_at.clone(),
        }
    }
}

/// Get the string label for a priority level.
const fn priority_label(priority: Priority) -> &'static str {
    match priority {
        Priority::Critical => "critical",
        Priority::High => "high",
        Priority::Medium => "medium",
        Priority::Low => "low",
        Priority::Backlog => "backlog",
    }
}

// Tool implementations
// Note: rmcp macros require pass-by-value for input parameters

#[tool(tool_box)]
impl TasksServer {
    /// Create a new task.
    #[tool(description = "Create a new task with title, description, and priority")]
    fn create_task(
        &self,
        #[tool(aggr)] input: CreateTaskInput,
    ) -> Result<CallToolResult, McpError> {
        let priority = Priority::from_u8(input.priority)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let task = self
            .store
            .create_task(&input.title, &input.description, priority)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
        let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();
        let output = TaskOutput::from_task(&task, deps, guidance);
        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get a task by ID.
    #[tool(description = "Get a task by its ID, including dependencies, notes, and guidance")]
    fn get_task(&self, #[tool(aggr)] input: GetTaskInput) -> Result<CallToolResult, McpError> {
        let task = self
            .store
            .get_task(&input.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match task {
            Some(task) => {
                let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
                let notes = self.store.get_notes(&task.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();

                let output = FullTaskOutput {
                    task: TaskOutput::from_task(&task, deps, guidance),
                    notes: notes
                        .into_iter()
                        .map(|n| NoteOutput {
                            id: n.id,
                            content: n.content,
                            created_at: n.created_at,
                        })
                        .collect(),
                };

                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Task not found: {}",
                input.id
            ))])),
        }
    }

    /// Update a task's fields.
    #[tool(description = "Update a task's title, description, priority, or status")]
    fn update_task(
        &self,
        #[tool(aggr)] input: UpdateTaskInput,
    ) -> Result<CallToolResult, McpError> {
        let priority = input
            .priority
            .map(Priority::from_u8)
            .transpose()
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let status = input
            .status
            .as_ref()
            .map(|s| Status::from_str(s))
            .transpose()
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let update = TaskUpdate {
            title: input.title,
            description: input.description,
            priority,
            status,
            in_progress: None,
            requested: None,
        };

        let task = self
            .store
            .update_task(&input.id, update)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match task {
            Some(task) => {
                let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();
                let output = TaskOutput::from_task(&task, deps, guidance);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Task not found: {}",
                input.id
            ))])),
        }
    }

    /// Delete a task.
    #[tool(description = "Delete a task by its ID")]
    fn delete_task(
        &self,
        #[tool(aggr)] input: DeleteTaskInput,
    ) -> Result<CallToolResult, McpError> {
        let deleted = self
            .store
            .delete_task(&input.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if deleted {
            Ok(CallToolResult::success(vec![Content::text(format!("Task deleted: {}", input.id))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Task not found: {}",
                input.id
            ))]))
        }
    }

    /// List tasks with optional filters.
    #[tool(description = "List tasks, optionally filtered by status, priority, or ready state")]
    fn list_tasks(&self, #[tool(aggr)] input: ListTasksInput) -> Result<CallToolResult, McpError> {
        let status = input
            .status
            .as_ref()
            .map(|s| Status::from_str(s))
            .transpose()
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let priority = input
            .priority
            .map(Priority::from_u8)
            .transpose()
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let max_priority = input
            .max_priority
            .map(Priority::from_u8)
            .transpose()
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let filter = TaskFilter { status, priority, max_priority, ready_only: input.ready_only };

        let tasks = self
            .store
            .list_tasks(filter)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let outputs: Vec<_> = tasks
            .iter()
            .map(|t| {
                let deps = self.store.get_dependencies(&t.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&t.id).unwrap_or_default();
                TaskOutput::from_task(t, deps, guidance)
            })
            .collect();

        let json = serde_json::to_string_pretty(&outputs)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Add a dependency between tasks.
    #[tool(description = "Add a dependency (task_id depends on depends_on task)")]
    fn add_dependency(
        &self,
        #[tool(aggr)] input: AddDependencyInput,
    ) -> Result<CallToolResult, McpError> {
        self.store
            .add_dependency(&input.task_id, &input.depends_on)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Dependency added: {} now depends on {}",
            input.task_id, input.depends_on
        ))]))
    }

    /// Remove a dependency between tasks.
    #[tool(description = "Remove a dependency between tasks")]
    fn remove_dependency(
        &self,
        #[tool(aggr)] input: RemoveDependencyInput,
    ) -> Result<CallToolResult, McpError> {
        let removed = self
            .store
            .remove_dependency(&input.task_id, &input.depends_on)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if removed {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Dependency removed: {} no longer depends on {}",
                input.task_id, input.depends_on
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text("Dependency not found".to_string())]))
        }
    }

    /// Add a note to a task.
    #[tool(description = "Add a note to a task")]
    fn add_note(&self, #[tool(aggr)] input: AddNoteInput) -> Result<CallToolResult, McpError> {
        let note = self
            .store
            .add_note(&input.task_id, &input.content)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output = NoteWithTaskOutput {
            id: note.id,
            task_id: note.task_id,
            content: note.content,
            created_at: note.created_at,
        };

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get all notes for a task.
    #[tool(description = "Get all notes attached to a task")]
    fn get_notes(&self, #[tool(aggr)] input: GetNotesInput) -> Result<CallToolResult, McpError> {
        let notes = self
            .store
            .get_notes(&input.task_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let outputs: Vec<_> = notes
            .into_iter()
            .map(|n| NoteOutput { id: n.id, content: n.content, created_at: n.created_at })
            .collect();

        let json = serde_json::to_string_pretty(&outputs)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Search tasks by text.
    #[tool(description = "Full-text search across task titles, descriptions, and notes")]
    fn search_tasks(
        &self,
        #[tool(aggr)] input: SearchTasksInput,
    ) -> Result<CallToolResult, McpError> {
        let tasks = self
            .store
            .search_tasks(&input.query)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let outputs: Vec<_> = tasks
            .iter()
            .map(|t| {
                let deps = self.store.get_dependencies(&t.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&t.id).unwrap_or_default();
                TaskOutput::from_task(t, deps, guidance)
            })
            .collect();

        let json = serde_json::to_string_pretty(&outputs)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get audit log entries.
    #[tool(description = "Get the audit log, optionally filtered by task ID")]
    fn get_audit_log(
        &self,
        #[tool(aggr)] input: GetAuditLogInput,
    ) -> Result<CallToolResult, McpError> {
        let entries = self
            .store
            .get_audit_log(input.task_id.as_deref(), input.limit)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let json = serde_json::to_string_pretty(&entries)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get a random task to work on from the highest priority ready tasks.
    #[tool(description = "Pick a random task from the highest priority unblocked tasks")]
    fn what_should_i_work_on(&self) -> Result<CallToolResult, McpError> {
        let task =
            self.store.pick_task().map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match task {
            Some(task) => {
                let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
                let notes = self.store.get_notes(&task.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();

                let output = TaskSuggestion {
                    task: TaskOutput::from_task(&task, deps, guidance),
                    notes: notes
                        .into_iter()
                        .map(|n| NoteOutput { id: n.id, content: n.content, created_at: n.created_at })
                        .collect(),
                    message: format!(
                        "Suggested task: {} (priority: {})",
                        task.title,
                        priority_label(task.priority)
                    ),
                };

                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(
                "No tasks available to work on. All tasks are either complete, blocked, or the task list is empty.".to_string(),
            )])),
        }
    }

    // How-to tools

    /// Create a new how-to guide.
    #[tool(description = "Create a new how-to guide with title and instructions")]
    fn create_howto(
        &self,
        #[tool(aggr)] input: CreateHowToInput,
    ) -> Result<CallToolResult, McpError> {
        let howto = self
            .store
            .create_howto(&input.title, &input.instructions)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output = HowToOutput::from_howto(&howto);
        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get a how-to by ID.
    #[tool(description = "Get a how-to guide by its ID")]
    fn get_howto(&self, #[tool(aggr)] input: GetHowToInput) -> Result<CallToolResult, McpError> {
        let howto = self
            .store
            .get_howto(&input.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match howto {
            Some(howto) => {
                let output = HowToOutput::from_howto(&howto);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "How-to not found: {}",
                input.id
            ))])),
        }
    }

    /// Update a how-to.
    #[tool(description = "Update a how-to guide's title or instructions")]
    fn update_howto(
        &self,
        #[tool(aggr)] input: UpdateHowToInput,
    ) -> Result<CallToolResult, McpError> {
        let update = HowToUpdate { title: input.title, instructions: input.instructions };

        let howto = self
            .store
            .update_howto(&input.id, update)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match howto {
            Some(howto) => {
                let output = HowToOutput::from_howto(&howto);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "How-to not found: {}",
                input.id
            ))])),
        }
    }

    /// Delete a how-to.
    #[tool(description = "Delete a how-to guide by its ID")]
    fn delete_howto(
        &self,
        #[tool(aggr)] input: DeleteHowToInput,
    ) -> Result<CallToolResult, McpError> {
        let deleted = self
            .store
            .delete_howto(&input.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if deleted {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Deleted how-to: {}",
                input.id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "How-to not found: {}",
                input.id
            ))]))
        }
    }

    /// List all how-tos.
    #[tool(description = "List all how-to guides")]
    fn list_howtos(&self) -> Result<CallToolResult, McpError> {
        let howtos =
            self.store.list_howtos().map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let outputs: Vec<_> = howtos.iter().map(HowToOutput::from_howto).collect();

        let json = serde_json::to_string_pretty(&outputs)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Search how-tos.
    #[tool(description = "Full-text search across how-to guides")]
    fn search_howtos(
        &self,
        #[tool(aggr)] input: SearchHowTosInput,
    ) -> Result<CallToolResult, McpError> {
        let howtos = self
            .store
            .search_howtos(&input.query)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let outputs: Vec<_> = howtos.iter().map(HowToOutput::from_howto).collect();

        let json = serde_json::to_string_pretty(&outputs)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Link a task to a how-to guide.
    #[tool(description = "Link a task to a how-to guide for guidance")]
    fn link_task_to_howto(
        &self,
        #[tool(aggr)] input: LinkTaskToHowToInput,
    ) -> Result<CallToolResult, McpError> {
        self.store
            .link_task_to_howto(&input.task_id, &input.howto_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked task {} to how-to {}",
            input.task_id, input.howto_id
        ))]))
    }

    /// Unlink a task from a how-to guide.
    #[tool(description = "Remove a guidance link between a task and how-to")]
    fn unlink_task_from_howto(
        &self,
        #[tool(aggr)] input: UnlinkTaskFromHowToInput,
    ) -> Result<CallToolResult, McpError> {
        let removed = self
            .store
            .unlink_task_from_howto(&input.task_id, &input.howto_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if removed {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Unlinked task {} from how-to {}",
                input.task_id, input.howto_id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No link found between task {} and how-to {}",
                input.task_id, input.howto_id
            ))]))
        }
    }

    /// Create a new question that may block tasks.
    #[tool(description = "Create a question requiring user input that can block tasks")]
    fn create_question(
        &self,
        #[tool(aggr)] input: CreateQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        let question = self
            .store
            .create_question(&input.text)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output = QuestionOutput::from_question(&question);
        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get a question by ID.
    #[tool(description = "Get a question by its ID")]
    fn get_question(
        &self,
        #[tool(aggr)] input: GetQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        let question = self
            .store
            .get_question(&input.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match question {
            Some(q) => {
                let output = QuestionOutput::from_question(&q);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Question not found: {}",
                input.id
            ))])),
        }
    }

    /// Answer a question.
    #[tool(description = "Provide an answer to a question")]
    fn answer_question(
        &self,
        #[tool(aggr)] input: AnswerQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        let question = self
            .store
            .answer_question(&input.id, &input.answer)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match question {
            Some(q) => {
                let output = QuestionOutput::from_question(&q);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Question not found: {}",
                input.id
            ))])),
        }
    }

    /// Delete a question.
    #[tool(description = "Delete a question by its ID")]
    fn delete_question(
        &self,
        #[tool(aggr)] input: DeleteQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        let deleted = self
            .store
            .delete_question(&input.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if deleted {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Deleted question: {}",
                input.id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Question not found: {}",
                input.id
            ))]))
        }
    }

    /// List questions.
    #[tool(description = "List all questions, optionally filtering to only unanswered ones")]
    fn list_questions(
        &self,
        #[tool(aggr)] input: ListQuestionsInput,
    ) -> Result<CallToolResult, McpError> {
        let questions = self
            .store
            .list_questions(input.unanswered_only)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output: Vec<QuestionOutput> =
            questions.iter().map(QuestionOutput::from_question).collect();
        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Search questions.
    #[tool(description = "Full-text search across questions")]
    fn search_questions(
        &self,
        #[tool(aggr)] input: SearchQuestionsInput,
    ) -> Result<CallToolResult, McpError> {
        let questions = self
            .store
            .search_questions(&input.query)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output: Vec<QuestionOutput> =
            questions.iter().map(QuestionOutput::from_question).collect();
        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Link a task to a question (task blocked until question is answered).
    #[tool(
        description = "Link a task to a question - task will be blocked until the question is answered"
    )]
    fn link_task_to_question(
        &self,
        #[tool(aggr)] input: LinkTaskToQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        self.store
            .link_task_to_question(&input.task_id, &input.question_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked task {} to question {} - task is blocked until question is answered",
            input.task_id, input.question_id
        ))]))
    }

    /// Unlink a task from a question.
    #[tool(description = "Remove a blocking link between a task and a question")]
    fn unlink_task_from_question(
        &self,
        #[tool(aggr)] input: UnlinkTaskFromQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        let removed = self
            .store
            .unlink_task_from_question(&input.task_id, &input.question_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if removed {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Unlinked task {} from question {}",
                input.task_id, input.question_id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No link found between task {} and question {}",
                input.task_id, input.question_id
            ))]))
        }
    }

    /// Get blocking questions for a task.
    #[tool(description = "Get all unanswered questions that are blocking a specific task")]
    fn get_blocking_questions(
        &self,
        #[tool(aggr)] input: GetBlockingQuestionsInput,
    ) -> Result<CallToolResult, McpError> {
        let questions = self
            .store
            .get_blocking_questions(&input.task_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output: Vec<QuestionOutput> =
            questions.iter().map(QuestionOutput::from_question).collect();
        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get all tasks blocked by unanswered questions.
    #[tool(
        description = "Get all tasks that are blocked by unanswered questions (and not blocked by dependencies)"
    )]
    fn get_question_blocked_tasks(&self) -> Result<CallToolResult, McpError> {
        let tasks = self
            .store
            .get_question_blocked_tasks()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output: Vec<TaskOutput> = tasks
            .iter()
            .map(|t| {
                let deps = self.store.get_dependencies(&t.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&t.id).unwrap_or_default();
                TaskOutput::from_task(t, deps, guidance)
            })
            .collect();
        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Start working on a task (sets `in_progress` to true).
    #[tool(
        description = "Mark a task as in-progress. Use this before making any code changes to track what you're working on."
    )]
    fn work_on(&self, #[tool(aggr)] input: WorkOnInput) -> Result<CallToolResult, McpError> {
        let update = TaskUpdate { in_progress: Some(true), ..Default::default() };

        let task = self
            .store
            .update_task(&input.task_id, update)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match task {
            Some(task) => {
                let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();
                let output = TaskOutput::from_task(&task, deps, guidance);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Task not found: {}",
                input.task_id
            ))])),
        }
    }

    /// Mark specific tasks as requested by the user.
    #[tool(
        description = "Mark tasks as requested by the user. Requested tasks must be completed before the agent can stop."
    )]
    fn request_tasks(
        &self,
        #[tool(aggr)] input: RequestTasksInput,
    ) -> Result<CallToolResult, McpError> {
        let task_ids: Vec<&str> = input.task_ids.iter().map(String::as_str).collect();
        let updated = self
            .store
            .request_tasks(&task_ids)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Marked {updated} task(s) as requested."
        ))]))
    }

    /// Mark all open tasks as requested and enable request mode.
    #[tool(
        description = "Mark all open tasks as requested and enable request mode. New tasks will also be automatically requested until the agent successfully stops."
    )]
    fn request_all_open(&self) -> Result<CallToolResult, McpError> {
        let updated = self
            .store
            .request_all_open()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Marked {updated} task(s) as requested. Request mode enabled - new tasks will be automatically requested."
        ))]))
    }

    /// Get all incomplete requested tasks (tasks that must be completed before stopping).
    #[tool(
        description = "Get all incomplete requested tasks. These are tasks the user has requested that must be completed (or blocked on a question) before the agent can stop."
    )]
    fn get_incomplete_requested_tasks(&self) -> Result<CallToolResult, McpError> {
        let tasks = self
            .store
            .get_incomplete_requested_tasks()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if tasks.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No incomplete requested tasks. The agent may stop when ready.".to_string(),
            )]));
        }

        let output: Vec<TaskOutput> = tasks
            .iter()
            .map(|t| {
                let deps = self.store.get_dependencies(&t.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&t.id).unwrap_or_default();
                TaskOutput::from_task(t, deps, guidance)
            })
            .collect();

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[rmcp::tool(tool_box)]
impl rmcp::ServerHandler for TasksServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation { name: "tasks-mcp".to_string(), version: env!("CARGO_PKG_VERSION").to_string() },
            instructions: Some(
                "Task management server. Use these tools to create, update, list, and manage tasks with dependencies, notes, how-to guides, and questions requiring user input.".to_string(),
            ),
        }
    }
}
