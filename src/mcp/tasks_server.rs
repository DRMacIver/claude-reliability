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

## Working Through Tasks

**Trust your assigned scope.** If you have been given a list of tasks to complete, that IS the scope of your work. Do not second-guess or try to reduce this scope - complete what you've been asked to do.

**Work incrementally.** You are not expected to complete all tasks in a single session. Work through them in priority order, completing as many as you can. When a session ends, you'll pick up where you left off next time.

**Use what_should_i_work_on.** When unsure which task to work on next, use `what_should_i_work_on` to automatically select the highest-priority unblocked task.

**Having many open tasks is normal.** A large backlog doesn't mean anything is wrong - it's expected. Just work through tasks one at a time. Don't create questions asking about "too much work" or which subset to focus on - simply continue with the next available task.

**Blocked tasks will wait.** If a task is blocked by dependencies, work on something else. When the dependencies are completed, the blocked task will become available.

## Autonomous Work

**You do not need user guidance on priorities.** The priority system (P0-P4) tells you what to work on. Use `what_should_i_work_on` to get the next task - don't ask the user which task to do.

**Never use emergency_stop for "too much work".** Emergency stop is for genuine blockers like missing credentials, environment issues, or unclear requirements that prevent ANY progress. Having many tasks in the backlog is NOT a blocker - just work through them.

**Sessions can end naturally.** When context runs out or you've made good progress, it's fine to stop. Work will continue in the next session. You don't need to complete everything in one session.

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

How-to guides capture reusable procedures that can be linked to work items. When you retrieve a work item with `get_work_item`, linked how-tos are included with their full instructions.

**When to create a how-to:**
- When you discover a procedure that could be reused (e.g., "How to run tests", "How to deploy")
- When a work item requires specific steps that should be documented
- When the user asks to "document how to do X" or "create a guide for Y"

**Linking how-tos to work items:**
- Use `link_work_to_howto` to associate guidance with a work item
- When you fetch the work item later, the full how-to instructions appear automatically
- Multiple work items can share the same how-to guide
"#;

/// Default result limit for list/search operations to prevent oversized responses.
const DEFAULT_RESULT_LIMIT: usize = 50;

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
pub struct CreateWorkItemInput {
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
pub struct GetWorkItemInput {
    /// Work item ID.
    pub id: String,
}

/// Input for updating a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateWorkItemInput {
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
pub struct DeleteWorkItemInput {
    /// Work item ID.
    pub id: String,
}

/// Input for listing work items.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWorkItemsInput {
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
    pub work_item_id: String,
    /// Work item ID that must be completed first.
    pub depends_on: String,
}

/// Input for removing a dependency.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveDependencyInput {
    /// Work item ID that has the dependency.
    pub work_item_id: String,
    /// Work item ID to remove as dependency.
    pub depends_on: String,
}

/// Input for adding a note.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddNoteInput {
    /// Work item ID.
    pub work_item_id: String,
    /// Note content.
    pub content: String,
}

/// Input for getting notes.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNotesInput {
    /// Work item ID.
    pub work_item_id: String,
    /// Maximum number of notes to return (optional, default 50).
    pub limit: Option<usize>,
}

/// Input for searching work items.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchWorkItemsInput {
    /// Search query.
    pub query: String,
    /// Maximum number of results to return (optional, default 50).
    pub limit: Option<usize>,
}

/// Input for getting audit log.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAuditLogInput {
    /// Filter by work item ID (optional).
    pub work_item_id: Option<String>,
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
    /// Maximum number of results to return (optional, default 50).
    pub limit: Option<usize>,
}

/// Input for linking a work item to a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkWorkToHowToInput {
    /// Work item ID.
    pub work_item_id: String,
    /// How-to ID.
    pub howto_id: String,
}

/// Input for unlinking a work item from a how-to.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkWorkFromHowToInput {
    /// Work item ID.
    pub work_item_id: String,
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
    /// Maximum number of questions to return (optional, default 50).
    pub limit: Option<usize>,
}

/// Input for searching questions.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchQuestionsInput {
    /// Search query.
    pub query: String,
    /// Maximum number of results to return (optional, default 50).
    pub limit: Option<usize>,
}

/// Input for linking a work item to a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkWorkToQuestionInput {
    /// Work item ID.
    pub work_item_id: String,
    /// Question ID.
    pub question_id: String,
}

/// Input for unlinking a work item from a question.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkWorkFromQuestionInput {
    /// Work item ID.
    pub work_item_id: String,
    /// Question ID.
    pub question_id: String,
}

/// Input for getting blocking questions for a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBlockingQuestionsInput {
    /// Work item ID.
    pub work_item_id: String,
}

/// Input for starting work on a work item.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkOnInput {
    /// Work item ID to start working on.
    pub work_item_id: String,
}

/// Input for requesting specific work items.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RequestWorkItemsInput {
    /// Work item IDs to mark as requested.
    pub work_item_ids: Vec<String>,
}

/// Input for requesting an emergency stop.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmergencyStopInput {
    /// Explanation of why you need to stop.
    pub explanation: String,
}

// Output types - defined at module level to avoid items_after_statements

/// Work item summary for list operations - minimal data to reduce response size.
#[derive(Debug, Serialize)]
struct WorkItemSummary {
    id: String,
    title: String,
    priority: u8,
    priority_label: &'static str,
    status: String,
    in_progress: bool,
    requested: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    blocked_by: Vec<String>,
}

impl WorkItemSummary {
    fn from_task(task: &crate::tasks::Task, blocked_by: Vec<String>) -> Self {
        Self {
            id: task.id.clone(),
            title: task.title.clone(),
            priority: task.priority.as_u8(),
            priority_label: priority_label(task.priority),
            status: task.status.as_str().to_string(),
            in_progress: task.in_progress,
            requested: task.requested,
            blocked_by,
        }
    }
}

/// Work item output representation (full details).
#[derive(Debug, Serialize)]
struct WorkItemOutput {
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

impl WorkItemOutput {
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
struct NoteWithWorkItemOutput {
    id: i64,
    work_item_id: String,
    content: String,
    created_at: String,
}

/// Full work item with notes and how-tos for `get_task` response.
#[derive(Debug, Serialize)]
struct FullWorkItemOutput {
    #[serde(flatten)]
    task: WorkItemOutput,
    notes: Vec<NoteOutput>,
    /// Full how-to guides linked to this work item (not just IDs).
    howtos: Vec<HowToOutput>,
}

/// Work item suggestion for `what_should_i_work_on` response.
#[derive(Debug, Serialize)]
struct WorkItemSuggestion {
    #[serde(flatten)]
    task: WorkItemOutput,
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

/// Apply a result limit, returning the truncated items and the total count.
fn apply_limit<T>(items: Vec<T>, limit: Option<usize>) -> (Vec<T>, usize) {
    let total = items.len();
    let max = limit.unwrap_or(DEFAULT_RESULT_LIMIT);
    if total <= max {
        (items, total)
    } else {
        (items.into_iter().take(max).collect(), total)
    }
}

/// Format a JSON response with optional truncation notice.
fn format_list_response<T: Serialize>(
    items: &[T],
    total: usize,
) -> Result<String, serde_json::Error> {
    if total > items.len() {
        // Wrap in an object with metadata
        let wrapper = serde_json::json!({
            "items": items,
            "showing": items.len(),
            "total": total,
            "truncated": true,
        });
        serde_json::to_string_pretty(&wrapper)
    } else {
        serde_json::to_string_pretty(items)
    }
}

// Tool implementations
// Note: rmcp macros require pass-by-value for input parameters

#[tool(tool_box)]
impl TasksServer {
    /// Create a new work item.
    #[tool(description = "Create a new work item with title, description, and priority")]
    fn create_work_item(
        &self,
        #[tool(aggr)] input: CreateWorkItemInput,
    ) -> Result<CallToolResult, McpError> {
        let priority = Priority::from_u8(input.priority)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let task = self
            .store
            .create_task(&input.title, &input.description, priority)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
        let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();
        let output = WorkItemOutput::from_task(&task, deps, guidance);
        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get a work item by ID.
    #[tool(description = "Get a work item by its ID, including dependencies, notes, and guidance")]
    fn get_work_item(
        &self,
        #[tool(aggr)] input: GetWorkItemInput,
    ) -> Result<CallToolResult, McpError> {
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

                let output = FullWorkItemOutput {
                    task: WorkItemOutput::from_task(&task, deps, guidance),
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
    fn update_work_item(
        &self,
        #[tool(aggr)] input: UpdateWorkItemInput,
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
                let output = WorkItemOutput::from_task(&task, deps, guidance);
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
    fn delete_work_item(
        &self,
        #[tool(aggr)] input: DeleteWorkItemInput,
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
    fn list_work_items(
        &self,
        #[tool(aggr)] input: ListWorkItemsInput,
    ) -> Result<CallToolResult, McpError> {
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
            limit: Some(input.limit.unwrap_or(DEFAULT_RESULT_LIMIT)),
            offset: input.offset,
        };

        let tasks = self
            .store
            .list_tasks(filter)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        // Use summary format to reduce response size
        let outputs: Vec<_> = tasks
            .iter()
            .map(|t| {
                // Only include incomplete dependencies (blockers)
                let blocked_by: Vec<String> = self
                    .store
                    .get_dependencies(&t.id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|dep_id| {
                        self.store
                            .get_task(dep_id)
                            .ok()
                            .flatten()
                            .is_some_and(|t| t.status != Status::Complete)
                    })
                    .collect();
                WorkItemSummary::from_task(t, blocked_by)
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
            .add_dependency(&input.work_item_id, &input.depends_on)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Dependency added: {} now depends on {}",
            input.work_item_id, input.depends_on
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
            .remove_dependency(&input.work_item_id, &input.depends_on)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if removed {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Dependency removed: {} no longer depends on {}",
                input.work_item_id, input.depends_on
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
            .add_note(&input.work_item_id, &input.content)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let output = NoteWithWorkItemOutput {
            id: note.id,
            work_item_id: note.task_id,
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
            .get_notes(&input.work_item_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let (notes, total) = apply_limit(notes, input.limit);

        let outputs: Vec<_> = notes
            .into_iter()
            .map(|n| NoteOutput { id: n.id, content: n.content, created_at: n.created_at })
            .collect();

        let json = format_list_response(&outputs, total)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Search tasks by text.
    #[tool(description = "Full-text search across work item titles, descriptions, and notes")]
    fn search_work_items(
        &self,
        #[tool(aggr)] input: SearchWorkItemsInput,
    ) -> Result<CallToolResult, McpError> {
        let tasks = self
            .store
            .search_tasks(&input.query)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let (tasks, total) = apply_limit(tasks, input.limit);

        // Use summary format to reduce response size
        let outputs: Vec<_> = tasks
            .iter()
            .map(|t| {
                let blocked_by: Vec<String> = self
                    .store
                    .get_dependencies(&t.id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|dep_id| {
                        self.store
                            .get_task(dep_id)
                            .ok()
                            .flatten()
                            .is_some_and(|t| t.status != Status::Complete)
                    })
                    .collect();
                WorkItemSummary::from_task(t, blocked_by)
            })
            .collect();

        let json = format_list_response(&outputs, total)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get audit log entries.
    #[tool(description = "Get the audit log, optionally filtered by work item ID")]
    fn get_audit_log(
        &self,
        #[tool(aggr)] input: GetAuditLogInput,
    ) -> Result<CallToolResult, McpError> {
        let limit = Some(input.limit.unwrap_or(DEFAULT_RESULT_LIMIT));
        let entries = self
            .store
            .get_audit_log(input.work_item_id.as_deref(), limit)
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

                let output = WorkItemSuggestion {
                    task: WorkItemOutput::from_task(&task, deps, guidance),
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

        let (howtos, total) = apply_limit(howtos, None);
        let outputs: Vec<_> = howtos.iter().map(HowToOutput::from_howto).collect();

        let json = format_list_response(&outputs, total)
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

        let (howtos, total) = apply_limit(howtos, input.limit);
        let outputs: Vec<_> = howtos.iter().map(HowToOutput::from_howto).collect();

        let json = format_list_response(&outputs, total)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Link a task to a how-to guide.
    #[tool(description = "Link a work item to a how-to guide for guidance")]
    fn link_work_to_howto(
        &self,
        #[tool(aggr)] input: LinkWorkToHowToInput,
    ) -> Result<CallToolResult, McpError> {
        self.store
            .link_task_to_howto(&input.work_item_id, &input.howto_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked work item {} to how-to {}",
            input.work_item_id, input.howto_id
        ))]))
    }

    /// Unlink a task from a how-to guide.
    #[tool(description = "Remove a guidance link between a work item and how-to")]
    fn unlink_work_from_howto(
        &self,
        #[tool(aggr)] input: UnlinkWorkFromHowToInput,
    ) -> Result<CallToolResult, McpError> {
        let removed = self
            .store
            .unlink_task_from_howto(&input.work_item_id, &input.howto_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if removed {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Unlinked work item {} from how-to {}",
                input.work_item_id, input.howto_id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No link found between work item {} and how-to {}",
                input.work_item_id, input.howto_id
            ))]))
        }
    }

    /// Create a new question that may block tasks.
    ///
    /// This method first evaluates whether the question can be auto-answered.
    /// If so, returns the answer instead of creating the question.
    #[tool(description = "Create a question requiring user input that can block work items")]
    fn create_question(
        &self,
        #[tool(aggr)] input: CreateQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        use crate::command::RealCommandRunner;
        use crate::subagent::RealSubAgent;
        use crate::traits::{CreateQuestionContext, CreateQuestionDecision, SubAgent};

        // Evaluate whether this question can be auto-answered
        let runner = RealCommandRunner::new();
        let sub_agent = RealSubAgent::new(&runner);
        let context = CreateQuestionContext { question_text: input.text.clone() };

        match sub_agent.evaluate_create_question(&context) {
            Ok(CreateQuestionDecision::AutoAnswer(answer)) => {
                // Return the auto-answer instead of creating the question
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "Question auto-answered (no user input needed):\n\n{answer}"
                ))]));
            }
            Ok(CreateQuestionDecision::Create) | Err(_) => {
                // Proceed with creating the question
            }
        }

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

        let (questions, total) = apply_limit(questions, input.limit);
        let output: Vec<QuestionOutput> =
            questions.iter().map(QuestionOutput::from_question).collect();

        let json = format_list_response(&output, total)
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

        let (questions, total) = apply_limit(questions, input.limit);
        let output: Vec<QuestionOutput> =
            questions.iter().map(QuestionOutput::from_question).collect();

        let json = format_list_response(&output, total)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Link a task to a question (task blocked until question is answered).
    #[tool(
        description = "Link a work item to a question - item will be blocked until the question is answered"
    )]
    fn link_work_to_question(
        &self,
        #[tool(aggr)] input: LinkWorkToQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        self.store
            .link_task_to_question(&input.work_item_id, &input.question_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked work item {} to question {} - item is blocked until question is answered",
            input.work_item_id, input.question_id
        ))]))
    }

    /// Unlink a task from a question.
    #[tool(description = "Remove a blocking link between a work item and a question")]
    fn unlink_work_from_question(
        &self,
        #[tool(aggr)] input: UnlinkWorkFromQuestionInput,
    ) -> Result<CallToolResult, McpError> {
        let removed = self
            .store
            .unlink_task_from_question(&input.work_item_id, &input.question_id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if removed {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Unlinked work item {} from question {}",
                input.work_item_id, input.question_id
            ))]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No link found between work item {} and question {}",
                input.work_item_id, input.question_id
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
            .get_blocking_questions(&input.work_item_id)
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
    fn get_question_blocked_work(&self) -> Result<CallToolResult, McpError> {
        let tasks = self
            .store
            .get_question_blocked_tasks()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let (tasks, total) = apply_limit(tasks, None);
        let output: Vec<WorkItemOutput> = tasks
            .iter()
            .map(|t| {
                let deps = self.store.get_dependencies(&t.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&t.id).unwrap_or_default();
                WorkItemOutput::from_task(t, deps, guidance)
            })
            .collect();

        let json = format_list_response(&output, total)
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
            .update_task(&input.work_item_id, update)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        match task {
            Some(task) => {
                let deps = self.store.get_dependencies(&task.id).unwrap_or_default();
                let guidance = self.store.get_task_guidance(&task.id).unwrap_or_default();
                let output = WorkItemOutput::from_task(&task, deps, guidance);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text(format!(
                "Work item not found: {}",
                input.work_item_id
            ))])),
        }
    }

    /// Mark specific tasks as requested by the user.
    #[tool(
        description = "Mark work items as requested by the user. Requested items must be completed before the agent can stop."
    )]
    fn request_work_items(
        &self,
        #[tool(aggr)] input: RequestWorkItemsInput,
    ) -> Result<CallToolResult, McpError> {
        let task_ids: Vec<&str> = input.work_item_ids.iter().map(String::as_str).collect();
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
    fn get_incomplete_requested_work(&self) -> Result<CallToolResult, McpError> {
        let tasks = self
            .store
            .get_incomplete_requested_work()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        if tasks.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No incomplete requested work items. The agent may stop when ready.".to_string(),
            )]));
        }

        // Use summary format to reduce response size
        let output: Vec<WorkItemSummary> = tasks
            .iter()
            .map(|t| {
                let blocked_by: Vec<String> = self
                    .store
                    .get_dependencies(&t.id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|dep_id| {
                        self.store
                            .get_task(dep_id)
                            .ok()
                            .flatten()
                            .is_some_and(|t| t.status != Status::Complete)
                    })
                    .collect();
                WorkItemSummary::from_task(t, blocked_by)
            })
            .collect();

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Request an emergency stop, validated by a sub-agent.
    ///
    /// The sub-agent evaluates whether the stop request is legitimate.
    /// Genuine blockers (missing credentials, unclear requirements) are accepted.
    /// Complaints about too much work are rejected with guidance to prioritize.
    #[tool(
        description = "Request an emergency stop when you've hit a genuine blocker that requires user intervention. A sub-agent will evaluate whether the stop is justified."
    )]
    fn emergency_stop(
        &self,
        #[tool(aggr)] input: EmergencyStopInput,
    ) -> Result<CallToolResult, McpError> {
        use crate::subagent::RealSubAgent;
        use crate::traits::{EmergencyStopContext, EmergencyStopDecision, SubAgent};

        let runner = RealCommandRunner::new();
        let sub_agent = RealSubAgent::new(&runner);

        let context = EmergencyStopContext { explanation: input.explanation };

        match sub_agent.evaluate_emergency_stop(&context) {
            Ok(EmergencyStopDecision::Accept(msg)) => {
                session::set_emergency_stop(&self.base_dir)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                let message = msg.map_or_else(
                    || {
                        "Emergency stop accepted.\n\n\
                         Please explain the problem clearly to the user, then stop."
                            .to_string()
                    },
                    |m| {
                        format!(
                            "Emergency stop accepted: {m}\n\n\
                             Please explain the problem clearly to the user, then stop."
                        )
                    },
                );

                Ok(CallToolResult::success(vec![Content::text(message)]))
            }
            Ok(EmergencyStopDecision::Reject(instructions)) => {
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Emergency stop denied. {instructions}"
                ))]))
            }
            Err(_) => {
                // On failure, default to accepting (conservative — let agent stop)
                session::set_emergency_stop(&self.base_dir)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(CallToolResult::success(vec![Content::text(
                    "Emergency stop accepted (evaluation unavailable).\n\n\
                     Please explain the problem clearly to the user, then stop."
                        .to_string(),
                )]))
            }
        }
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
