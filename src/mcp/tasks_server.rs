//! MCP server for task management.
//!
//! This module provides an MCP server that exposes task management
//! functionality through the Model Context Protocol.

// The rmcp `#[tool(aggr)]` macro requires ownership of input structs,
// making pass-by-value necessary for all tool handler functions.
#![allow(clippy::needless_pass_by_value)]

use crate::beads_sync;
use crate::command::RealCommandRunner;
use crate::session;
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
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Instructions for the MCP server, shown to agents using this server.
const INSTRUCTIONS: &str = r#"Work tracking server. Use these tools to create, update, list, and manage work items with dependencies, notes, how-to guides, and questions requiring user input.

## Bulk Operations

When working with multiple work items, use the bulk-tasks binary for better performance:

### Create multiple work items (3+)
```bash
${CLAUDE_PLUGIN_ROOT}/bin/bulk-tasks create <<'EOF'
{
  "tasks": [
    {"id": "t1", "title": "First item", "description": "...", "priority": 1},
    {"id": "t2", "title": "Second item", "priority": 2, "depends_on": ["t1"]},
    {"id": "t3", "title": "Third item", "priority": 2, "depends_on": ["t1", "t2"]}
  ]
}
EOF
```
The `id` fields are temporary identifiers for setting up dependencies. Actual IDs are returned in output.

### Add dependencies to existing work items
```bash
${CLAUDE_PLUGIN_ROOT}/bin/bulk-tasks add-deps <<'EOF'
{"dependencies": [{"task": "item-id-1", "depends_on": "item-id-2"}]}
EOF
```

### List work items with filtering
```bash
${CLAUDE_PLUGIN_ROOT}/bin/bulk-tasks list <<'EOF'
{"status": "open", "priority": 1, "ready_only": true}
EOF
```
All fields optional. Empty `{}` returns all work items.

### Search work items
```bash
${CLAUDE_PLUGIN_ROOT}/bin/bulk-tasks search <<'EOF'
{"query": "search term"}
EOF
```

Priority values: 0=critical, 1=high, 2=medium, 3=low, 4=backlog

## How-To Guides

How-to guides capture reusable procedures that can be linked to work items. When you retrieve a work item with `get_task`, linked how-tos are included with their full instructions.

**When to create a how-to:**
- When you discover a procedure that could be reused (e.g., "How to run tests", "How to deploy")
- When a work item requires specific steps that should be documented
- When the user asks to "document how to do X" or "create a guide for Y"

**Linking how-tos to work items:**
- Use `link_task_to_howto` to associate guidance with a work item
- When you fetch the work item later, the full how-to instructions appear automatically
- Multiple work items can share the same how-to guide
"#;

/// MCP server for task management.
#[derive(Clone)]
pub struct TasksServer {
    store: Arc<SqliteTaskStore>,
    /// Base directory for session state (problem mode, etc.)
    base_dir: PathBuf,
}

impl TasksServer {
    /// Create a new tasks server with the given database path.
    ///
    /// Uses the current working directory as the base directory for session state.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be initialized.
    pub fn new(db_path: &Path) -> crate::error::Result<Self> {
        let store = SqliteTaskStore::new(db_path)?;
        let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Ok(Self { store: Arc::new(store), base_dir })
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
        let base_dir = project_dir.to_path_buf();
        Ok(Self { store: Arc::new(store), base_dir })
    }
}

// Tool input schemas

/// Input for creating a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTaskInput {
    /// Work item title (required).
    pub title: String,
    /// Work item description.
    #[serde(default)]
    pub description: String,
    /// Priority: 0=critical, 1=high, 2=medium (default), 3=low, 4=backlog.
    #[serde(default = "default_priority")]
    pub priority: u8,
}

const fn default_priority() -> u8 {
    2
}

/// Input for getting a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTaskInput {
    /// Work item ID.
    pub id: String,
}

/// Input for updating a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTaskInput {
    /// Work item ID.
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

/// Input for deleting a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteTaskInput {
    /// Work item ID.
    pub id: String,
}

/// Input for listing work items.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTasksInput {
    /// Filter by status (optional).
    pub status: Option<String>,
    /// Filter by exact priority (optional).
    pub priority: Option<u8>,
    /// Filter by maximum priority (optional).
    pub max_priority: Option<u8>,
    /// Only show work items that are ready to work on (optional).
    #[serde(default)]
    pub ready_only: bool,
    /// Maximum number of work items to return (optional).
    pub limit: Option<usize>,
    /// Number of work items to skip before returning results (optional).
    pub offset: Option<usize>,
}

/// Input for adding a dependency.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddDependencyInput {
    /// Work item ID that will have the dependency.
    pub task_id: String,
    /// Work item ID that must be completed first.
    pub depends_on: String,
}

/// Input for removing a dependency.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveDependencyInput {
    /// Work item ID that has the dependency.
    pub task_id: String,
    /// Work item ID to remove as dependency.
    pub depends_on: String,
}

/// Input for adding a note.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddNoteInput {
    /// Work item ID.
    pub task_id: String,
    /// Note content.
    pub content: String,
}

/// Input for getting notes.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNotesInput {
    /// Work item ID.
    pub task_id: String,
}

/// Input for searching work items.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchTasksInput {
    /// Search query.
    pub query: String,
}

/// Input for getting audit log.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAuditLogInput {
    /// Filter by work item ID (optional).
    pub task_id: Option<String>,
    /// Limit number of entries (optional).
    pub limit: Option<usize>,
}

/// Input for creating a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateHowToInput {
    /// How-to title (required).
    pub title: String,
    /// Instructions for how to perform the work.
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

/// Input for linking a work item to a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkTaskToHowToInput {
    /// Work item ID.
    pub task_id: String,
    /// How-to ID.
    pub howto_id: String,
}

/// Input for unlinking a work item from a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkTaskFromHowToInput {
    /// Work item ID.
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

/// Input for linking a work item to a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkTaskToQuestionInput {
    /// Work item ID.
    pub task_id: String,
    /// Question ID.
    pub question_id: String,
}

/// Input for unlinking a work item from a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkTaskFromQuestionInput {
    /// Work item ID.
    pub task_id: String,
    /// Question ID.
    pub question_id: String,
}

/// Input for getting blocking questions for a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBlockingQuestionsInput {
    /// Work item ID.
    pub task_id: String,
}

/// Input for starting work on a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkOnInput {
    /// Work item ID to start working on.
    pub task_id: String,
}

/// Input for requesting specific work items.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RequestTasksInput {
    /// Work item IDs to mark as requested.
    pub task_ids: Vec<String>,
}

/// Input for signaling a problem that requires user intervention.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SignalProblemInput {
    /// A description of the problem that requires user input.
    pub reason: String,
}

// Output types - defined at module level to avoid items_after_statements

/// Work item output representation.
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

/// Note output with work item ID for serialization.
#[derive(Debug, Serialize)]
struct NoteWithTaskOutput {
    id: i64,
    task_id: String,
    content: String,
    created_at: String,
}

/// Full work item with notes and how-tos for `get_task` response.
#[derive(Debug, Serialize)]
struct FullTaskOutput {
    #[serde(flatten)]
    task: TaskOutput,
    notes: Vec<NoteOutput>,
    /// Full how-to guides linked to this work item (not just IDs).
    howtos: Vec<HowToOutput>,
}

/// Work item suggestion for `what_should_i_work_on` response.
#[derive(Debug, Serialize)]
struct TaskSuggestion {
    #[serde(flatten)]
    task: TaskOutput,
    notes: Vec<NoteOutput>,
    /// Full how-to guides linked to this work item.
    howtos: Vec<HowToOutput>,
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
    /// Create a new work item.
    #[tool(description = "Create a new work item with title, description, and priority")]
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

    /// Get a work item by ID.
    #[tool(description = "Get a work item by its ID, including dependencies, notes, and guidance")]
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

                // Fetch full how-to content for each linked how-to
                let howtos: Vec<HowToOutput> = guidance
                    .iter()
                    .filter_map(|id| {
                        self.store.get_howto(id).ok().flatten().map(|h| HowToOutput::from_howto(&h))
                    })
                    .collect();

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
                    howtos,
                };

                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Work item not found: {}",
                input.id
            ))])),
        }
    }

    /// Update a work item's fields.
    #[tool(description = "Update a work item's title, description, priority, or status")]
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

        // Check if we're marking this task as complete - we may need to close a beads issue
        let is_completing = status == Some(Status::Complete);

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
                // If completing a task with a beads marker, close the beads issue
                if is_completing {
                    if let Some(beads_id) = beads_sync::extract_beads_id(&task.description) {
                        let runner = RealCommandRunner::new();
                        // Silently attempt to close - don't fail the task update if this fails
                        let _ = beads_sync::close_beads_issue(&runner, &self.base_dir, beads_id);
                    }
                }

                let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();
                let output = TaskOutput::from_task(&task, deps, guidance);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Work item not found: {}",
                input.id
            ))])),
        }
    }

    /// Delete a work item.
    #[tool(description = "Delete a work item by its ID")]
    fn delete_task(
        &self,
        #[tool(aggr)] input: DeleteTaskInput,
    ) -> Result<CallToolResult, McpError> {
        let deleted = self
            .store
            .delete_task(&input.id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if deleted {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Work item deleted: {}",
                input.id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Work item not found: {}",
                input.id
            ))]))
        }
    }

    /// List work items with optional filters.
    #[tool(
        description = "List work items, optionally filtered by status, priority, or ready state"
    )]
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

        let filter = TaskFilter {
            status,
            priority,
            max_priority,
            ready_only: input.ready_only,
            limit: input.limit,
            offset: input.offset,
        };

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
    #[tool(description = "Add a dependency (first work item depends on second work item)")]
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
    #[tool(description = "Remove a dependency between work items")]
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
    #[tool(description = "Add a note to a work item")]
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
    #[tool(description = "Get all notes attached to a work item")]
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
    #[tool(description = "Full-text search across work item titles, descriptions, and notes")]
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
    #[tool(description = "Get the audit log, optionally filtered by work item ID")]
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
    #[tool(description = "Pick a random work item from the highest priority unblocked items")]
    fn what_should_i_work_on(&self) -> Result<CallToolResult, McpError> {
        let task =
            self.store.pick_task().map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match task {
            Some(task) => {
                let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
                let notes = self.store.get_notes(&task.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();

                // Fetch full how-to content for each linked how-to
                let howtos: Vec<HowToOutput> = guidance
                    .iter()
                    .filter_map(|id| {
                        self.store
                            .get_howto(id)
                            .ok()
                            .flatten()
                            .map(|h| HowToOutput::from_howto(&h))
                    })
                    .collect();

                let output = TaskSuggestion {
                    task: TaskOutput::from_task(&task, deps, guidance),
                    notes: notes
                        .into_iter()
                        .map(|n| NoteOutput { id: n.id, content: n.content, created_at: n.created_at })
                        .collect(),
                    howtos,
                    message: format!(
                        "Suggested work item: {} (priority: {})",
                        task.title,
                        priority_label(task.priority)
                    ),
                };

                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(
                "No work items available. All items are either complete, blocked, or the list is empty.".to_string(),
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
    #[tool(description = "Link a work item to a how-to guide for guidance")]
    fn link_task_to_howto(
        &self,
        #[tool(aggr)] input: LinkTaskToHowToInput,
    ) -> Result<CallToolResult, McpError> {
        self.store
            .link_task_to_howto(&input.task_id, &input.howto_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked work item {} to how-to {}",
            input.task_id, input.howto_id
        ))]))
    }

    /// Unlink a task from a how-to guide.
    #[tool(description = "Remove a guidance link between a work item and how-to")]
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
                "Unlinked work item {} from how-to {}",
                input.task_id, input.howto_id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No link found between work item {} and how-to {}",
                input.task_id, input.howto_id
            ))]))
        }
    }

    /// Create a new question that may block tasks.
    #[tool(description = "Create a question requiring user input that can block work items")]
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
        description = "Link a work item to a question - item will be blocked until the question is answered"
    )]
    fn link_task_to_question(
        &self,
        #[tool(aggr)] input: LinkTaskToQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        self.store
            .link_task_to_question(&input.task_id, &input.question_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked work item {} to question {} - item is blocked until question is answered",
            input.task_id, input.question_id
        ))]))
    }

    /// Unlink a task from a question.
    #[tool(description = "Remove a blocking link between a work item and a question")]
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
                "Unlinked work item {} from question {}",
                input.task_id, input.question_id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No link found between work item {} and question {}",
                input.task_id, input.question_id
            ))]))
        }
    }

    /// Get blocking questions for a task.
    #[tool(description = "Get all unanswered questions that are blocking a specific work item")]
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
        description = "Get all work items that are blocked by unanswered questions (and not blocked by dependencies)"
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
        description = "Mark a work item as in-progress. Use this before making any code changes to track what you're working on."
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
                "Work item not found: {}",
                input.task_id
            ))])),
        }
    }

    /// Mark specific tasks as requested by the user.
    #[tool(
        description = "Mark work items as requested by the user. Requested items must be completed before the agent can stop."
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
            "Marked {updated} work item(s) as requested."
        ))]))
    }

    /// Mark all open tasks as requested and enable request mode.
    #[tool(
        description = "Mark all open work items as requested and enable request mode. New items will also be automatically requested until the agent successfully stops."
    )]
    fn request_all_open(&self) -> Result<CallToolResult, McpError> {
        let updated = self
            .store
            .request_all_open()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Marked {updated} work item(s) as requested. Request mode enabled - new items will be automatically requested."
        ))]))
    }

    /// Get all incomplete requested tasks (tasks that must be completed before stopping).
    #[tool(
        description = "Get all incomplete requested work items. These are items the user has requested that must be completed (or blocked on a question) before the agent can stop."
    )]
    fn get_incomplete_requested_tasks(&self) -> Result<CallToolResult, McpError> {
        let tasks = self
            .store
            .get_incomplete_requested_tasks()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if tasks.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No incomplete requested work items. The agent may stop when ready.".to_string(),
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

    /// Signal that you've encountered a problem requiring user intervention.
    ///
    /// Use this when you've hit a blocker that you cannot resolve on your own.
    /// After calling this, explain the problem clearly, then stop. On your next
    /// stop attempt, you will be allowed to exit.
    #[tool(
        description = "Signal that you've encountered a problem requiring user intervention. Call this when you're stuck on something that needs user input, then explain the problem and stop."
    )]
    fn signal_problem(
        &self,
        #[tool(aggr)] input: SignalProblemInput,
    ) -> Result<CallToolResult, McpError> {
        session::enter_problem_mode(&self.base_dir)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Problem mode activated. Reason recorded: {}\n\n\
             Please explain the problem clearly to the user, then stop. \
             Your next stop attempt will be permitted.",
            input.reason
        ))]))
    }
}

#[rmcp::tool(tool_box)]
impl rmcp::ServerHandler for TasksServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "tasks-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(INSTRUCTIONS.to_string()),
        }
    }
}
