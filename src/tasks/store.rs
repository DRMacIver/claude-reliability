//! Task store trait and `SQLite` implementation.

use crate::error::Result;
use crate::paths;
use crate::tasks::id::generate_task_id;
use crate::tasks::models::{AuditEntry, HowTo, Note, Priority, Question, Status, Task};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::hash_map::RandomState;
use std::collections::HashSet;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

/// Trait for task storage operations.
///
/// All methods return a `Result` and may fail with database errors.
#[allow(clippy::missing_errors_doc)]
pub trait TaskStore {
    // Task CRUD
    /// Create a new task with the given title, description, and priority.
    fn create_task(&self, title: &str, description: &str, priority: Priority) -> Result<Task>;

    /// Get a task by ID.
    fn get_task(&self, id: &str) -> Result<Option<Task>>;

    /// Update a task's fields.
    fn update_task(&self, id: &str, update: TaskUpdate) -> Result<Option<Task>>;

    /// Delete a task by ID.
    fn delete_task(&self, id: &str) -> Result<bool>;

    /// List tasks with optional filters.
    fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>>;

    // Dependencies
    /// Add a dependency (task depends on another task).
    fn add_dependency(&self, task_id: &str, depends_on: &str) -> Result<()>;

    /// Remove a dependency.
    fn remove_dependency(&self, task_id: &str, depends_on: &str) -> Result<bool>;

    /// Get all dependencies for a task.
    fn get_dependencies(&self, task_id: &str) -> Result<Vec<String>>;

    /// Get all tasks that depend on the given task.
    fn get_dependents(&self, task_id: &str) -> Result<Vec<String>>;

    // Notes
    /// Add a note to a task.
    fn add_note(&self, task_id: &str, content: &str) -> Result<Note>;

    /// Get all notes for a task.
    fn get_notes(&self, task_id: &str) -> Result<Vec<Note>>;

    /// Delete a note by ID.
    fn delete_note(&self, note_id: i64) -> Result<bool>;

    // Search
    /// Full-text search across tasks and notes.
    fn search_tasks(&self, query: &str) -> Result<Vec<Task>>;

    // Audit
    /// Get audit log entries, optionally filtered by task ID.
    fn get_audit_log(&self, task_id: Option<&str>, limit: Option<usize>)
        -> Result<Vec<AuditEntry>>;

    // Utility
    /// Get tasks that are ready to work on (open, not blocked by dependencies).
    fn get_ready_tasks(&self) -> Result<Vec<Task>>;

    /// Pick a random task from the highest priority ready tasks.
    fn pick_task(&self) -> Result<Option<Task>>;

    // How-to CRUD
    /// Create a new how-to guide with the given title and instructions.
    fn create_howto(&self, title: &str, instructions: &str) -> Result<HowTo>;

    /// Get a how-to by ID.
    fn get_howto(&self, id: &str) -> Result<Option<HowTo>>;

    /// Update a how-to's fields.
    fn update_howto(&self, id: &str, update: HowToUpdate) -> Result<Option<HowTo>>;

    /// Delete a how-to by ID.
    fn delete_howto(&self, id: &str) -> Result<bool>;

    /// List all how-tos.
    fn list_howtos(&self) -> Result<Vec<HowTo>>;

    /// Full-text search across how-tos.
    fn search_howtos(&self, query: &str) -> Result<Vec<HowTo>>;

    // Task-HowTo guidance
    /// Link a task to a how-to guide for guidance.
    fn link_task_to_howto(&self, task_id: &str, howto_id: &str) -> Result<()>;

    /// Unlink a task from a how-to guide.
    fn unlink_task_from_howto(&self, task_id: &str, howto_id: &str) -> Result<bool>;

    /// Get all how-to IDs linked to a task (guidance).
    fn get_task_guidance(&self, task_id: &str) -> Result<Vec<String>>;

    // Questions (for user input)
    /// Create a new question.
    fn create_question(&self, text: &str) -> Result<Question>;

    /// Get a question by ID.
    fn get_question(&self, id: &str) -> Result<Option<Question>>;

    /// Answer a question.
    fn answer_question(&self, id: &str, answer: &str) -> Result<Option<Question>>;

    /// Delete a question by ID.
    fn delete_question(&self, id: &str) -> Result<bool>;

    /// List questions, optionally filtering to only unanswered ones.
    fn list_questions(&self, unanswered_only: bool) -> Result<Vec<Question>>;

    /// Full-text search across questions.
    fn search_questions(&self, query: &str) -> Result<Vec<Question>>;

    // Task-Question relationships (blocking)
    /// Link a task to a question (task is blocked until question is answered).
    fn link_task_to_question(&self, task_id: &str, question_id: &str) -> Result<()>;

    /// Unlink a task from a question.
    fn unlink_task_from_question(&self, task_id: &str, question_id: &str) -> Result<bool>;

    /// Get all question IDs linked to a task.
    fn get_task_questions(&self, task_id: &str) -> Result<Vec<String>>;

    /// Get all unanswered questions that are blocking a task.
    fn get_blocking_questions(&self, task_id: &str) -> Result<Vec<Question>>;

    /// Get all tasks blocked by unanswered questions (and not blocked by dependencies).
    fn get_question_blocked_tasks(&self) -> Result<Vec<Task>>;

    /// Check if any task is currently in progress.
    fn has_in_progress_task(&self) -> Result<bool>;

    /// Get all tasks that are currently in progress.
    fn get_in_progress_tasks(&self) -> Result<Vec<Task>>;

    // Request mode operations

    /// Mark multiple tasks as requested (bulk operation).
    fn request_tasks(&self, task_ids: &[&str]) -> Result<usize>;

    /// Mark all open tasks as requested and enable request mode.
    /// When request mode is active, newly created tasks are automatically requested.
    fn request_all_open(&self) -> Result<usize>;

    /// Check if request mode is currently active.
    fn is_request_mode_active(&self) -> Result<bool>;

    /// Clear request mode (called when stop is allowed).
    fn clear_request_mode(&self) -> Result<()>;

    /// Get incomplete requested work items (for stop hook).
    /// Returns items that are requested but not complete/abandoned, and not blocked on questions.
    /// Also includes items that are dependencies of requested items (transitive).
    /// Results are ordered by priority (ascending) then dependent count (descending),
    /// and limited to the top 5.
    fn get_incomplete_requested_work(&self) -> Result<Vec<Task>>;
}

/// Fields that can be updated on a task.
#[derive(Debug, Default, Clone)]
pub struct TaskUpdate {
    /// New title (if Some).
    pub title: Option<String>,
    /// New description (if Some).
    pub description: Option<String>,
    /// New priority (if Some).
    pub priority: Option<Priority>,
    /// New status (if Some).
    pub status: Option<Status>,
    /// New `in_progress` flag (if Some).
    pub in_progress: Option<bool>,
    /// New `requested` flag (if Some).
    pub requested: Option<bool>,
}

impl TaskUpdate {
    /// Check if any fields are set for update.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.priority.is_none()
            && self.status.is_none()
            && self.in_progress.is_none()
            && self.requested.is_none()
    }
}

/// Filter options for listing tasks.
#[derive(Debug, Default, Clone)]
pub struct TaskFilter {
    /// Filter by status.
    pub status: Option<Status>,
    /// Filter by priority.
    pub priority: Option<Priority>,
    /// Filter by maximum priority (inclusive, lower number = higher priority).
    pub max_priority: Option<Priority>,
    /// Include only tasks that are not blocked.
    pub ready_only: bool,
    /// Maximum number of tasks to return.
    pub limit: Option<usize>,
    /// Number of tasks to skip before returning results.
    pub offset: Option<usize>,
}

/// Error when a circular dependency would be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CircularDependency {
    /// The task that would have the new dependency.
    pub task_id: String,
    /// The task that would be depended on.
    pub depends_on: String,
}

impl std::fmt::Display for CircularDependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "adding dependency {} -> {} would create a cycle", self.task_id, self.depends_on)
    }
}

impl std::error::Error for CircularDependency {}

/// Error when a referenced task is not found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskNotFound(pub String);

impl std::fmt::Display for TaskNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "task not found: {}", self.0)
    }
}

impl std::error::Error for TaskNotFound {}

/// Error when a referenced how-to is not found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HowToNotFound(pub String);

impl std::fmt::Display for HowToNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "how-to not found: {}", self.0)
    }
}

impl std::error::Error for HowToNotFound {}

/// Error when a referenced question is not found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionNotFound(pub String);

impl std::fmt::Display for QuestionNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "question not found: {}", self.0)
    }
}

impl std::error::Error for QuestionNotFound {}

/// Fields that can be updated on a how-to.
#[derive(Debug, Default, Clone)]
pub struct HowToUpdate {
    /// New title (if Some).
    pub title: Option<String>,
    /// New instructions (if Some).
    pub instructions: Option<String>,
}

impl HowToUpdate {
    /// Check if any fields are set for update.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.title.is_none() && self.instructions.is_none()
    }
}

/// Generate an ISO 8601 timestamp string for the current time.
fn now_timestamp() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// SQLite-based task store.
#[derive(Debug, Clone)]
pub struct SqliteTaskStore {
    db_path: PathBuf,
}

impl SqliteTaskStore {
    /// Create a new `SQLite` task store at the given database path.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be initialized.
    pub fn new(db_path: impl AsRef<Path>) -> Result<Self> {
        let store = Self { db_path: db_path.as_ref().to_path_buf() };
        store.init_schema()?;
        Ok(store)
    }

    /// Create a new `SQLite` task store for the given project directory.
    ///
    /// The database will be at `<project_dir>/.claude-reliability/working-memory.sqlite3`.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be initialized.
    pub fn for_project(project_dir: &Path) -> Result<Self> {
        let db_path = paths::project_db_path(project_dir);
        Self::new(db_path)
    }

    /// Get the database path.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Open a connection to the database.
    fn open(&self) -> Result<Connection> {
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&self.db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Ok(conn)
    }

    /// Initialize the database schema.
    #[allow(clippy::too_many_lines)]
    fn init_schema(&self) -> Result<()> {
        let conn = self.open()?;

        conn.execute_batch(
            r"
            -- Core tasks table
            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT DEFAULT '',
                priority INTEGER NOT NULL DEFAULT 2 CHECK (priority >= 0 AND priority <= 4),
                status TEXT NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open', 'complete', 'abandoned', 'stuck', 'blocked')),
                in_progress INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Dependencies (task depends_on another task)
            CREATE TABLE IF NOT EXISTS task_dependencies (
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                depends_on TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (task_id, depends_on),
                CHECK (task_id != depends_on)
            );

            -- Notes attached to tasks
            CREATE TABLE IF NOT EXISTS task_notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Immutable audit log
            CREATE TABLE IF NOT EXISTS task_audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                operation TEXT NOT NULL,
                task_id TEXT,
                old_value TEXT,
                new_value TEXT,
                details TEXT
            );

            -- Indexes for common queries
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_priority ON tasks(priority);
            CREATE INDEX IF NOT EXISTS idx_tasks_status_priority ON tasks(status, priority);
            CREATE INDEX IF NOT EXISTS idx_task_dependencies_depends_on ON task_dependencies(depends_on);
            CREATE INDEX IF NOT EXISTS idx_task_notes_task_id ON task_notes(task_id);
            CREATE INDEX IF NOT EXISTS idx_task_audit_task_id ON task_audit_log(task_id);

            -- FTS5 for full-text search on tasks
            CREATE VIRTUAL TABLE IF NOT EXISTS tasks_fts USING fts5(
                id, title, description,
                content='tasks', content_rowid='rowid'
            );

            -- FTS5 for notes
            CREATE VIRTUAL TABLE IF NOT EXISTS task_notes_fts USING fts5(
                task_id, content,
                content='task_notes', content_rowid='id'
            );

            -- Triggers to keep FTS in sync
            CREATE TRIGGER IF NOT EXISTS tasks_ai AFTER INSERT ON tasks BEGIN
                INSERT INTO tasks_fts(rowid, id, title, description)
                VALUES (NEW.rowid, NEW.id, NEW.title, NEW.description);
            END;

            CREATE TRIGGER IF NOT EXISTS tasks_ad AFTER DELETE ON tasks BEGIN
                INSERT INTO tasks_fts(tasks_fts, rowid, id, title, description)
                VALUES ('delete', OLD.rowid, OLD.id, OLD.title, OLD.description);
            END;

            CREATE TRIGGER IF NOT EXISTS tasks_au AFTER UPDATE ON tasks BEGIN
                INSERT INTO tasks_fts(tasks_fts, rowid, id, title, description)
                VALUES ('delete', OLD.rowid, OLD.id, OLD.title, OLD.description);
                INSERT INTO tasks_fts(rowid, id, title, description)
                VALUES (NEW.rowid, NEW.id, NEW.title, NEW.description);
            END;

            CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON task_notes BEGIN
                INSERT INTO task_notes_fts(rowid, task_id, content)
                VALUES (NEW.id, NEW.task_id, NEW.content);
            END;

            CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON task_notes BEGIN
                INSERT INTO task_notes_fts(task_notes_fts, rowid, task_id, content)
                VALUES ('delete', OLD.id, OLD.task_id, OLD.content);
            END;

            -- How-to guides table
            CREATE TABLE IF NOT EXISTS howtos (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                instructions TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Task guidance (links tasks to how-tos)
            CREATE TABLE IF NOT EXISTS task_guidance (
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                howto_id TEXT NOT NULL REFERENCES howtos(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (task_id, howto_id)
            );

            -- Index for looking up guidance by how-to
            CREATE INDEX IF NOT EXISTS idx_task_guidance_howto_id ON task_guidance(howto_id);

            -- FTS5 for full-text search on how-tos
            CREATE VIRTUAL TABLE IF NOT EXISTS howtos_fts USING fts5(
                id, title, instructions,
                content='howtos', content_rowid='rowid'
            );

            -- Triggers to keep how-to FTS in sync
            CREATE TRIGGER IF NOT EXISTS howtos_ai AFTER INSERT ON howtos BEGIN
                INSERT INTO howtos_fts(rowid, id, title, instructions)
                VALUES (NEW.rowid, NEW.id, NEW.title, NEW.instructions);
            END;

            CREATE TRIGGER IF NOT EXISTS howtos_ad AFTER DELETE ON howtos BEGIN
                INSERT INTO howtos_fts(howtos_fts, rowid, id, title, instructions)
                VALUES ('delete', OLD.rowid, OLD.id, OLD.title, OLD.instructions);
            END;

            CREATE TRIGGER IF NOT EXISTS howtos_au AFTER UPDATE ON howtos BEGIN
                INSERT INTO howtos_fts(howtos_fts, rowid, id, title, instructions)
                VALUES ('delete', OLD.rowid, OLD.id, OLD.title, OLD.instructions);
                INSERT INTO howtos_fts(rowid, id, title, instructions)
                VALUES (NEW.rowid, NEW.id, NEW.title, NEW.instructions);
            END;

            -- Questions (for user input that blocks tasks)
            CREATE TABLE IF NOT EXISTS questions (
                id TEXT PRIMARY KEY,
                text TEXT NOT NULL,
                answer TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                answered_at TEXT
            );

            -- Task-Question relationships (task is blocked by question)
            CREATE TABLE IF NOT EXISTS task_questions (
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                question_id TEXT NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (task_id, question_id)
            );

            -- Index for looking up tasks by question
            CREATE INDEX IF NOT EXISTS idx_task_questions_question_id ON task_questions(question_id);

            -- FTS5 for full-text search on questions
            CREATE VIRTUAL TABLE IF NOT EXISTS questions_fts USING fts5(
                id, text,
                content='questions', content_rowid='rowid'
            );

            -- Triggers to keep question FTS in sync
            CREATE TRIGGER IF NOT EXISTS questions_ai AFTER INSERT ON questions BEGIN
                INSERT INTO questions_fts(rowid, id, text)
                VALUES (NEW.rowid, NEW.id, NEW.text);
            END;

            CREATE TRIGGER IF NOT EXISTS questions_ad AFTER DELETE ON questions BEGIN
                INSERT INTO questions_fts(questions_fts, rowid, id, text)
                VALUES ('delete', OLD.rowid, OLD.id, OLD.text);
            END;

            CREATE TRIGGER IF NOT EXISTS questions_au AFTER UPDATE ON questions BEGIN
                INSERT INTO questions_fts(questions_fts, rowid, id, text)
                VALUES ('delete', OLD.rowid, OLD.id, OLD.text);
                INSERT INTO questions_fts(rowid, id, text)
                VALUES (NEW.rowid, NEW.id, NEW.text);
            END;

            -- Metadata table for tracking versions and settings
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            ",
        )?;

        // Migration: add in_progress column if it doesn't exist (for existing databases)
        let has_in_progress: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('tasks') WHERE name = 'in_progress'",
            [],
            |row| row.get(0),
        )?;
        if !has_in_progress {
            conn.execute(
                "ALTER TABLE tasks ADD COLUMN in_progress INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }

        // Migration: add requested column if it doesn't exist (for existing databases)
        let has_requested: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('tasks') WHERE name = 'requested'",
            [],
            |row| row.get(0),
        )?;
        if !has_requested {
            conn.execute("ALTER TABLE tasks ADD COLUMN requested INTEGER NOT NULL DEFAULT 0", [])?;
        }

        // Sync built-in how-tos
        crate::tasks::builtin_howtos::sync_builtin_howtos(&conn)?;

        Ok(())
    }

    /// Log an operation to the audit log.
    fn log_audit(
        conn: &Connection,
        operation: &str,
        task_id: Option<&str>,
        old_value: Option<&str>,
        new_value: Option<&str>,
        details: Option<&str>,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO task_audit_log (operation, task_id, old_value, new_value, details)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![operation, task_id, old_value, new_value, details],
        )?;
        Ok(())
    }

    /// Check if adding a dependency would create a cycle.
    fn would_create_cycle(conn: &Connection, task_id: &str, depends_on: &str) -> Result<bool> {
        // DFS from depends_on to see if we can reach task_id
        let mut visited = HashSet::new();
        let mut stack = vec![depends_on.to_string()];

        while let Some(current) = stack.pop() {
            if current == task_id {
                return Ok(true);
            }
            if visited.insert(current.clone()) {
                let mut stmt =
                    conn.prepare("SELECT depends_on FROM task_dependencies WHERE task_id = ?1")?;
                let deps: Vec<String> =
                    stmt.query_map(params![&current], |row| row.get(0))?.flatten().collect();
                stack.extend(deps);
            }
        }

        Ok(false)
    }

    /// Parse a task from a row.
    fn parse_task(row: &rusqlite::Row) -> rusqlite::Result<Task> {
        let priority_val: u8 = row.get(3)?;
        let status_str: String = row.get(4)?;
        let in_progress_val: i64 = row.get(5)?;
        let requested_val: i64 = row.get(6)?;

        Ok(Task {
            id: row.get(0)?,
            title: row.get(1)?,
            description: row.get(2)?,
            priority: Priority::from_u8(priority_val).unwrap_or(Priority::Medium),
            status: Status::from_str(&status_str).unwrap_or(Status::Open),
            in_progress: in_progress_val != 0,
            requested: requested_val != 0,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    }

    /// Parse a how-to from a row.
    fn parse_howto(row: &rusqlite::Row) -> rusqlite::Result<HowTo> {
        Ok(HowTo {
            id: row.get(0)?,
            title: row.get(1)?,
            instructions: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }

    /// Update task status to blocked/unblocked based on dependencies.
    #[allow(clippy::unused_self)]
    fn update_blocked_status(&self, conn: &Connection, task_id: &str) -> Result<()> {
        // Check if any dependencies are incomplete
        let has_incomplete_deps: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM task_dependencies d
                    JOIN tasks t ON d.depends_on = t.id
                    WHERE d.task_id = ?1 AND t.status NOT IN ('complete', 'abandoned')
                )",
                params![task_id],
                |row| row.get(0),
            )
            .unwrap_or(false);

        // Get current task status
        let current_status: Option<String> = conn
            .query_row("SELECT status FROM tasks WHERE id = ?1", params![task_id], |row| row.get(0))
            .optional()?;

        if let Some(status) = current_status {
            let should_be_blocked = has_incomplete_deps;
            let is_blocked = status == "blocked";

            // Only auto-transition between open and blocked
            if should_be_blocked && status == "open" {
                conn.execute(
                    "UPDATE tasks SET status = 'blocked', updated_at = datetime('now') WHERE id = ?1",
                    params![task_id],
                )?;
            } else if !should_be_blocked && is_blocked {
                conn.execute(
                    "UPDATE tasks SET status = 'open', updated_at = datetime('now') WHERE id = ?1",
                    params![task_id],
                )?;
            }
        }

        Ok(())
    }

    /// Update blocked status for all tasks that depend on the given task.
    fn update_dependents_blocked_status(&self, conn: &Connection, task_id: &str) -> Result<()> {
        let mut stmt =
            conn.prepare("SELECT task_id FROM task_dependencies WHERE depends_on = ?1")?;
        let dependents: Vec<String> =
            stmt.query_map(params![task_id], |row| row.get(0))?.flatten().collect();

        for dependent in dependents {
            self.update_blocked_status(conn, &dependent)?;
        }

        Ok(())
    }

    /// Check if request mode is active (internal helper that takes a connection).
    fn is_request_mode_active_internal(conn: &Connection) -> bool {
        conn.query_row("SELECT value FROM metadata WHERE key = 'request_mode_active'", [], |row| {
            row.get::<_, String>(0)
        })
        .ok()
        .is_some_and(|v| v == "true")
    }

    /// Get all task IDs that are dependencies of the given tasks (recursive).
    fn get_transitive_dependencies(
        conn: &Connection,
        task_ids: &[String],
    ) -> Result<HashSet<String>> {
        let mut all_deps = HashSet::new();
        let mut to_process: Vec<String> = task_ids.to_vec();

        while let Some(task_id) = to_process.pop() {
            let mut stmt =
                conn.prepare("SELECT depends_on FROM task_dependencies WHERE task_id = ?1")?;
            let deps: Vec<String> =
                stmt.query_map(params![&task_id], |row| row.get(0))?.flatten().collect();

            for dep in deps {
                if all_deps.insert(dep.clone()) {
                    to_process.push(dep);
                }
            }
        }

        Ok(all_deps)
    }
}

impl TaskStore for SqliteTaskStore {
    fn create_task(&self, title: &str, description: &str, priority: Priority) -> Result<Task> {
        let conn = self.open()?;
        let id = generate_task_id(title);

        // Check if request mode is active - if so, auto-request new tasks
        let request_mode_active = Self::is_request_mode_active_internal(&conn);

        conn.execute(
            "INSERT INTO tasks (id, title, description, priority, requested) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, title, description, priority.as_u8(), i32::from(request_mode_active)],
        )?;

        let task = conn.query_row(
            "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
             FROM tasks WHERE id = ?1",
            params![&id],
            Self::parse_task,
        )?;

        let task_json = serde_json::to_string(&task).unwrap_or_default();
        Self::log_audit(&conn, "create", Some(&id), None, Some(&task_json), None)?;

        Ok(task)
    }

    fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let conn = self.open()?;
        let task = conn
            .query_row(
                "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
                 FROM tasks WHERE id = ?1",
                params![id],
                Self::parse_task,
            )
            .optional()?;
        Ok(task)
    }

    fn update_task(&self, id: &str, update: TaskUpdate) -> Result<Option<Task>> {
        if update.is_empty() {
            return self.get_task(id);
        }

        let conn = self.open()?;

        // Get current task for audit log
        let old_task: Option<Task> = conn
            .query_row(
                "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
                 FROM tasks WHERE id = ?1",
                params![id],
                Self::parse_task,
            )
            .optional()?;

        if old_task.is_none() {
            return Ok(None);
        }

        // Build dynamic UPDATE statement
        let mut updates = vec!["updated_at = datetime('now')"];
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref title) = update.title {
            updates.push("title = ?");
            values.push(Box::new(title.clone()));
        }
        if let Some(ref description) = update.description {
            updates.push("description = ?");
            values.push(Box::new(description.clone()));
        }
        if let Some(priority) = update.priority {
            updates.push("priority = ?");
            values.push(Box::new(priority.as_u8()));
        }
        if let Some(status) = update.status {
            updates.push("status = ?");
            values.push(Box::new(status.as_str().to_string()));
            // Auto-clear in_progress when status becomes complete, blocked, or abandoned
            if matches!(status, Status::Complete | Status::Blocked | Status::Abandoned) {
                updates.push("in_progress = 0");
            }
        }
        if let Some(in_progress) = update.in_progress {
            updates.push("in_progress = ?");
            values.push(Box::new(i32::from(in_progress)));
        }
        if let Some(requested) = update.requested {
            updates.push("requested = ?");
            values.push(Box::new(i32::from(requested)));
        }

        values.push(Box::new(id.to_string()));

        let sql = format!("UPDATE tasks SET {} WHERE id = ?", updates.join(", "));

        let params: Vec<&dyn rusqlite::ToSql> = values.iter().map(AsRef::as_ref).collect();
        conn.execute(&sql, params.as_slice())?;

        // Get updated task
        let new_task = conn.query_row(
            "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
             FROM tasks WHERE id = ?1",
            params![id],
            Self::parse_task,
        )?;

        // Log audit
        let old_json = serde_json::to_string(&old_task).unwrap_or_default();
        let new_json = serde_json::to_string(&new_task).unwrap_or_default();
        Self::log_audit(&conn, "update", Some(id), Some(&old_json), Some(&new_json), None)?;

        // If status changed, update dependents
        if update.status.is_some() {
            self.update_dependents_blocked_status(&conn, id)?;
        }

        Ok(Some(new_task))
    }

    fn delete_task(&self, id: &str) -> Result<bool> {
        let conn = self.open()?;

        // Get task for audit log
        let task: Option<Task> = conn
            .query_row(
                "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
                 FROM tasks WHERE id = ?1",
                params![id],
                Self::parse_task,
            )
            .optional()?;

        if task.is_none() {
            return Ok(false);
        }

        let task_json = serde_json::to_string(&task).unwrap_or_default();

        // Get dependents before deleting (for status update)
        let mut stmt =
            conn.prepare("SELECT task_id FROM task_dependencies WHERE depends_on = ?1")?;
        let dependents: Vec<String> =
            stmt.query_map(params![id], |row| row.get(0))?.flatten().collect();

        let rows = conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;

        if rows > 0 {
            Self::log_audit(&conn, "delete", Some(id), Some(&task_json), None, None)?;

            // Update blocked status for former dependents
            for dependent in &dependents {
                self.update_blocked_status(&conn, dependent)?;
            }
        }

        Ok(rows > 0)
    }

    fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        let conn = self.open()?;

        let mut conditions = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(status) = filter.status {
            conditions.push("status = ?");
            params_vec.push(Box::new(status.as_str().to_string()));
        }

        if let Some(priority) = filter.priority {
            conditions.push("priority = ?");
            params_vec.push(Box::new(priority.as_u8()));
        }

        if let Some(max_priority) = filter.max_priority {
            conditions.push("priority <= ?");
            params_vec.push(Box::new(max_priority.as_u8()));
        }

        if filter.ready_only {
            conditions.push("status = 'open'");
            conditions.push(
                "NOT EXISTS (
                SELECT 1 FROM task_dependencies d
                JOIN tasks dep ON d.depends_on = dep.id
                WHERE d.task_id = tasks.id AND dep.status NOT IN ('complete', 'abandoned')
            )",
            );
            // Also exclude tasks blocked by unanswered questions
            conditions.push(
                "NOT EXISTS (
                SELECT 1 FROM task_questions tq
                JOIN questions q ON tq.question_id = q.id
                WHERE tq.task_id = tasks.id AND q.answer IS NULL
            )",
            );
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        // Build LIMIT/OFFSET clause
        let limit_clause = match (filter.limit, filter.offset) {
            (Some(limit), Some(offset)) => format!("LIMIT {limit} OFFSET {offset}"),
            (Some(limit), None) => format!("LIMIT {limit}"),
            (None, Some(offset)) => format!("LIMIT -1 OFFSET {offset}"), // SQLite requires LIMIT with OFFSET
            (None, None) => String::new(),
        };

        // Order by: status (open/stuck/blocked/complete/abandoned), unblocked, requested,
        // priority, blocking count, created_at
        // This prioritizes tasks that are best to work on
        let sql = format!(
            "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
             FROM tasks {where_clause}
             ORDER BY
                 -- Status order: open (0), stuck (1), blocked (2), complete (3), abandoned (4)
                 CASE status
                     WHEN 'open' THEN 0
                     WHEN 'stuck' THEN 1
                     WHEN 'blocked' THEN 2
                     WHEN 'complete' THEN 3
                     WHEN 'abandoned' THEN 4
                     ELSE 5
                 END ASC,
                 -- Unblocked first (count of open dependencies + blocking questions)
                 (
                     (SELECT COUNT(*) FROM task_dependencies d
                      JOIN tasks dep ON d.depends_on = dep.id
                      WHERE d.task_id = tasks.id AND dep.status NOT IN ('complete', 'abandoned'))
                     +
                     (SELECT COUNT(*) FROM task_questions tq
                      JOIN questions q ON tq.question_id = q.id
                      WHERE tq.task_id = tasks.id AND q.answer IS NULL)
                 ) ASC,
                 -- Requested first
                 CASE WHEN requested = 1 THEN 0 ELSE 1 END ASC,
                 -- Higher priority first (lower number = higher priority)
                 priority ASC,
                 -- Tasks that block more others first
                 (SELECT COUNT(*) FROM task_dependencies d WHERE d.depends_on = tasks.id) DESC,
                 -- Older tasks first
                 created_at ASC
             {limit_clause}"
        );

        let params: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(AsRef::as_ref).collect();
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt.query_map(params.as_slice(), Self::parse_task)?.flatten().collect();

        Ok(tasks)
    }

    fn add_dependency(&self, task_id: &str, depends_on: &str) -> Result<()> {
        let conn = self.open()?;

        // Verify both tasks exist
        let task_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
            params![task_id],
            |row| row.get(0),
        )?;
        if !task_exists {
            return Err(crate::error::Error::Task(Box::new(TaskNotFound(task_id.to_string()))));
        }

        let dep_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
            params![depends_on],
            |row| row.get(0),
        )?;
        if !dep_exists {
            return Err(crate::error::Error::Task(Box::new(TaskNotFound(depends_on.to_string()))));
        }

        // Check for cycles
        if Self::would_create_cycle(&conn, task_id, depends_on)? {
            return Err(crate::error::Error::Task(Box::new(CircularDependency {
                task_id: task_id.to_string(),
                depends_on: depends_on.to_string(),
            })));
        }

        // Add dependency
        conn.execute(
            "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on) VALUES (?1, ?2)",
            params![task_id, depends_on],
        )?;

        Self::log_audit(
            &conn,
            "add_dependency",
            Some(task_id),
            None,
            None,
            Some(&format!("depends_on: {depends_on}")),
        )?;

        // Update blocked status
        self.update_blocked_status(&conn, task_id)?;

        Ok(())
    }

    fn remove_dependency(&self, task_id: &str, depends_on: &str) -> Result<bool> {
        let conn = self.open()?;

        let rows = conn.execute(
            "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on = ?2",
            params![task_id, depends_on],
        )?;

        if rows > 0 {
            Self::log_audit(
                &conn,
                "remove_dependency",
                Some(task_id),
                None,
                None,
                Some(&format!("removed: {depends_on}")),
            )?;

            // Update blocked status
            self.update_blocked_status(&conn, task_id)?;
        }

        Ok(rows > 0)
    }

    fn get_dependencies(&self, task_id: &str) -> Result<Vec<String>> {
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT depends_on FROM task_dependencies WHERE task_id = ?1")?;
        let deps = stmt.query_map(params![task_id], |row| row.get(0))?.flatten().collect();
        Ok(deps)
    }

    fn get_dependents(&self, task_id: &str) -> Result<Vec<String>> {
        let conn = self.open()?;
        let mut stmt =
            conn.prepare("SELECT task_id FROM task_dependencies WHERE depends_on = ?1")?;
        let deps = stmt.query_map(params![task_id], |row| row.get(0))?.flatten().collect();
        Ok(deps)
    }

    fn add_note(&self, task_id: &str, content: &str) -> Result<Note> {
        let conn = self.open()?;

        // Verify task exists
        let task_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
            params![task_id],
            |row| row.get(0),
        )?;
        if !task_exists {
            return Err(crate::error::Error::Task(Box::new(TaskNotFound(task_id.to_string()))));
        }

        conn.execute(
            "INSERT INTO task_notes (task_id, content) VALUES (?1, ?2)",
            params![task_id, content],
        )?;

        let note_id = conn.last_insert_rowid();
        let note = conn.query_row(
            "SELECT id, task_id, content, created_at FROM task_notes WHERE id = ?1",
            params![note_id],
            |row| {
                Ok(Note {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                })
            },
        )?;

        Self::log_audit(
            &conn,
            "add_note",
            Some(task_id),
            None,
            Some(content),
            Some(&format!("note_id: {note_id}")),
        )?;

        Ok(note)
    }

    fn get_notes(&self, task_id: &str) -> Result<Vec<Note>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, content, created_at FROM task_notes
             WHERE task_id = ?1 ORDER BY created_at ASC",
        )?;

        let notes = stmt
            .query_map(params![task_id], |row| {
                Ok(Note {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .flatten()
            .collect();

        Ok(notes)
    }

    fn delete_note(&self, note_id: i64) -> Result<bool> {
        let conn = self.open()?;

        // Get note info for audit
        let note_info: Option<(String, String)> = conn
            .query_row(
                "SELECT task_id, content FROM task_notes WHERE id = ?1",
                params![note_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let rows = conn.execute("DELETE FROM task_notes WHERE id = ?1", params![note_id])?;

        if rows > 0 {
            if let Some((task_id, content)) = note_info {
                Self::log_audit(
                    &conn,
                    "delete_note",
                    Some(&task_id),
                    Some(&content),
                    None,
                    Some(&format!("note_id: {note_id}")),
                )?;
            }
        }

        Ok(rows > 0)
    }

    fn search_tasks(&self, query: &str) -> Result<Vec<Task>> {
        let conn = self.open()?;

        // Search tasks FTS
        let mut task_ids: HashSet<String> = HashSet::new();

        // Search in tasks
        let mut stmt = conn.prepare("SELECT id FROM tasks_fts WHERE tasks_fts MATCH ?1")?;
        let ids: Vec<String> =
            stmt.query_map(params![query], |row| row.get(0))?.flatten().collect();
        task_ids.extend(ids);

        // Search in notes
        let mut stmt =
            conn.prepare("SELECT task_id FROM task_notes_fts WHERE task_notes_fts MATCH ?1")?;
        let ids: Vec<String> =
            stmt.query_map(params![query], |row| row.get(0))?.flatten().collect();
        task_ids.extend(ids);

        // Fetch full task records
        let mut tasks = Vec::new();
        for id in task_ids {
            if let Some(task) = self.get_task(&id)? {
                tasks.push(task);
            }
        }

        // Sort by priority, then created_at
        tasks.sort_by(|a, b| {
            a.priority.cmp(&b.priority).then_with(|| a.created_at.cmp(&b.created_at))
        });

        Ok(tasks)
    }

    #[allow(clippy::cast_possible_wrap)]
    fn get_audit_log(
        &self,
        task_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<AuditEntry>> {
        let conn = self.open()?;

        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::ToSql>>) = match (task_id, limit) {
            (Some(id), Some(lim)) => (
                "SELECT id, timestamp, operation, task_id, old_value, new_value, details
                 FROM task_audit_log WHERE task_id = ?1
                 ORDER BY timestamp DESC LIMIT ?2"
                    .to_string(),
                vec![Box::new(id.to_string()), Box::new(lim as i64)],
            ),
            (Some(id), None) => (
                "SELECT id, timestamp, operation, task_id, old_value, new_value, details
                 FROM task_audit_log WHERE task_id = ?1
                 ORDER BY timestamp DESC"
                    .to_string(),
                vec![Box::new(id.to_string())],
            ),
            (None, Some(lim)) => (
                "SELECT id, timestamp, operation, task_id, old_value, new_value, details
                 FROM task_audit_log ORDER BY timestamp DESC LIMIT ?1"
                    .to_string(),
                vec![Box::new(lim as i64)],
            ),
            (None, None) => (
                "SELECT id, timestamp, operation, task_id, old_value, new_value, details
                 FROM task_audit_log ORDER BY timestamp DESC"
                    .to_string(),
                vec![],
            ),
        };

        let params: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(AsRef::as_ref).collect();
        let mut stmt = conn.prepare(&sql)?;

        let entries = stmt
            .query_map(params.as_slice(), |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    operation: row.get(2)?,
                    task_id: row.get(3)?,
                    old_value: row.get(4)?,
                    new_value: row.get(5)?,
                    details: row.get(6)?,
                })
            })?
            .flatten()
            .collect();

        Ok(entries)
    }

    fn get_ready_tasks(&self) -> Result<Vec<Task>> {
        self.list_tasks(TaskFilter { ready_only: true, ..Default::default() })
    }

    #[allow(clippy::cast_possible_truncation)]
    fn pick_task(&self) -> Result<Option<Task>> {
        let ready = self.get_ready_tasks()?;
        if ready.is_empty() {
            return Ok(None);
        }

        // Find the highest priority (lowest number)
        let min_priority = ready.iter().map(|t| t.priority).min().unwrap();

        // Filter to only tasks at that priority
        let top_priority: Vec<_> =
            ready.into_iter().filter(|t| t.priority == min_priority).collect();

        // Pick randomly using time-seeded hash
        let state = RandomState::new();
        let mut hasher = state.build_hasher();
        hasher.write_u64(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64),
        );
        let index = (hasher.finish() as usize) % top_priority.len();

        Ok(Some(top_priority.into_iter().nth(index).unwrap()))
    }

    fn create_howto(&self, title: &str, instructions: &str) -> Result<HowTo> {
        let conn = self.open()?;
        let id = generate_task_id(title);

        conn.execute(
            "INSERT INTO howtos (id, title, instructions) VALUES (?1, ?2, ?3)",
            params![&id, title, instructions],
        )?;

        let howto = conn
            .query_row(
                "SELECT id, title, instructions, created_at, updated_at FROM howtos WHERE id = ?1",
                params![&id],
                Self::parse_howto,
            )
            .optional()?
            .expect("just inserted");

        Self::log_audit(
            &conn,
            "create_howto",
            Some(&id),
            None,
            Some(&serde_json::to_string(&howto).unwrap_or_default()),
            Some(&format!("Created how-to: {title}")),
        )?;

        Ok(howto)
    }

    fn get_howto(&self, id: &str) -> Result<Option<HowTo>> {
        let conn = self.open()?;
        let howto = conn
            .query_row(
                "SELECT id, title, instructions, created_at, updated_at FROM howtos WHERE id = ?1",
                params![id],
                Self::parse_howto,
            )
            .optional()?;
        Ok(howto)
    }

    fn update_howto(&self, id: &str, update: HowToUpdate) -> Result<Option<HowTo>> {
        if update.is_empty() {
            return self.get_howto(id);
        }

        let conn = self.open()?;

        // Get the old value for audit
        let old_howto: Option<HowTo> = conn
            .query_row(
                "SELECT id, title, instructions, created_at, updated_at FROM howtos WHERE id = ?1",
                params![id],
                Self::parse_howto,
            )
            .optional()?;

        if old_howto.is_none() {
            return Ok(None);
        }

        let mut updates = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref title) = update.title {
            updates.push("title = ?");
            params_vec.push(Box::new(title.clone()));
        }
        if let Some(ref instructions) = update.instructions {
            updates.push("instructions = ?");
            params_vec.push(Box::new(instructions.clone()));
        }

        updates.push("updated_at = datetime('now')");

        let sql = format!("UPDATE howtos SET {} WHERE id = ?", updates.join(", "));
        params_vec.push(Box::new(id.to_string()));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(AsRef::as_ref).collect();
        conn.execute(&sql, params_refs.as_slice())?;

        let new_howto = conn
            .query_row(
                "SELECT id, title, instructions, created_at, updated_at FROM howtos WHERE id = ?1",
                params![id],
                Self::parse_howto,
            )
            .optional()?;

        if let Some(ref new) = new_howto {
            Self::log_audit(
                &conn,
                "update_howto",
                Some(id),
                Some(&serde_json::to_string(&old_howto).unwrap_or_default()),
                Some(&serde_json::to_string(new).unwrap_or_default()),
                None,
            )?;
        }

        Ok(new_howto)
    }

    fn delete_howto(&self, id: &str) -> Result<bool> {
        let conn = self.open()?;

        // Get the howto for audit logging
        let old_howto: Option<HowTo> = conn
            .query_row(
                "SELECT id, title, instructions, created_at, updated_at FROM howtos WHERE id = ?1",
                params![id],
                Self::parse_howto,
            )
            .optional()?;

        let deleted = conn.execute("DELETE FROM howtos WHERE id = ?1", params![id])? > 0;

        if deleted {
            Self::log_audit(
                &conn,
                "delete_howto",
                Some(id),
                Some(&serde_json::to_string(&old_howto).unwrap_or_default()),
                None,
                None,
            )?;
        }

        Ok(deleted)
    }

    fn list_howtos(&self) -> Result<Vec<HowTo>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, instructions, created_at, updated_at FROM howtos ORDER BY title",
        )?;
        let howtos = stmt.query_map([], Self::parse_howto)?.flatten().collect();
        Ok(howtos)
    }

    fn search_howtos(&self, query: &str) -> Result<Vec<HowTo>> {
        let conn = self.open()?;

        // Search using FTS5
        let fts_query = query
            .split_whitespace()
            .map(|word| format!("\"{word}\"*"))
            .collect::<Vec<_>>()
            .join(" ");

        let mut stmt = conn.prepare(
            "SELECT h.id, h.title, h.instructions, h.created_at, h.updated_at
             FROM howtos h
             JOIN howtos_fts fts ON h.id = fts.id
             WHERE howtos_fts MATCH ?1
             ORDER BY rank",
        )?;

        let howtos = stmt.query_map(params![&fts_query], Self::parse_howto)?.flatten().collect();
        Ok(howtos)
    }

    fn link_task_to_howto(&self, task_id: &str, howto_id: &str) -> Result<()> {
        let conn = self.open()?;

        // Verify task exists
        let task_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
            params![task_id],
            |row| row.get(0),
        )?;
        if !task_exists {
            return Err(crate::error::Error::Task(Box::new(TaskNotFound(task_id.to_string()))));
        }

        // Verify howto exists
        let howto_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM howtos WHERE id = ?1)",
            params![howto_id],
            |row| row.get(0),
        )?;
        if !howto_exists {
            return Err(crate::error::Error::Task(Box::new(HowToNotFound(howto_id.to_string()))));
        }

        // Insert (ignore if already exists)
        conn.execute(
            "INSERT OR IGNORE INTO task_guidance (task_id, howto_id) VALUES (?1, ?2)",
            params![task_id, howto_id],
        )?;

        Self::log_audit(
            &conn,
            "link_guidance",
            Some(task_id),
            None,
            None,
            Some(&format!("Linked task {task_id} to how-to {howto_id}")),
        )?;

        Ok(())
    }

    fn unlink_task_from_howto(&self, task_id: &str, howto_id: &str) -> Result<bool> {
        let conn = self.open()?;

        let rows_affected = conn.execute(
            "DELETE FROM task_guidance WHERE task_id = ?1 AND howto_id = ?2",
            params![task_id, howto_id],
        )?;
        let deleted = rows_affected > 0;

        if deleted {
            Self::log_audit(
                &conn,
                "unlink_guidance",
                Some(task_id),
                None,
                None,
                Some(&format!("Unlinked task {task_id} from how-to {howto_id}")),
            )?;
        }

        Ok(deleted)
    }

    fn get_task_guidance(&self, task_id: &str) -> Result<Vec<String>> {
        let conn = self.open()?;
        let mut stmt = conn
            .prepare("SELECT howto_id FROM task_guidance WHERE task_id = ?1 ORDER BY howto_id")?;
        let ids: Vec<String> =
            stmt.query_map(params![task_id], |row| row.get(0))?.flatten().collect();
        Ok(ids)
    }

    fn create_question(&self, text: &str) -> Result<Question> {
        let conn = self.open()?;
        let id = generate_task_id(text);
        let created_at = now_timestamp();

        conn.execute(
            "INSERT INTO questions (id, text, created_at) VALUES (?1, ?2, ?3)",
            params![id, text, created_at],
        )?;

        Self::log_audit(&conn, "create_question", Some(&id), None, None, Some(text))?;

        Ok(Question { id, text: text.to_string(), answer: None, created_at, answered_at: None })
    }

    fn get_question(&self, id: &str) -> Result<Option<Question>> {
        let conn = self.open()?;
        let question = conn
            .query_row(
                "SELECT id, text, answer, created_at, answered_at FROM questions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Question {
                        id: row.get(0)?,
                        text: row.get(1)?,
                        answer: row.get(2)?,
                        created_at: row.get(3)?,
                        answered_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(question)
    }

    fn answer_question(&self, id: &str, answer: &str) -> Result<Option<Question>> {
        let conn = self.open()?;

        let rows_affected = conn.execute(
            "UPDATE questions SET answer = ?2, answered_at = datetime('now') WHERE id = ?1",
            params![id, answer],
        )?;

        if rows_affected == 0 {
            return Ok(None);
        }

        Self::log_audit(&conn, "answer_question", Some(id), None, Some(answer), None)?;

        self.get_question(id)
    }

    fn delete_question(&self, id: &str) -> Result<bool> {
        let conn = self.open()?;
        let rows_affected = conn.execute("DELETE FROM questions WHERE id = ?1", params![id])?;
        let deleted = rows_affected > 0;

        if deleted {
            Self::log_audit(&conn, "delete_question", Some(id), None, None, None)?;
        }

        Ok(deleted)
    }

    fn list_questions(&self, unanswered_only: bool) -> Result<Vec<Question>> {
        let conn = self.open()?;
        let sql = if unanswered_only {
            "SELECT id, text, answer, created_at, answered_at FROM questions WHERE answer IS NULL ORDER BY created_at"
        } else {
            "SELECT id, text, answer, created_at, answered_at FROM questions ORDER BY created_at"
        };
        let mut stmt = conn.prepare(sql)?;
        let questions: Vec<Question> = stmt
            .query_map([], |row| {
                Ok(Question {
                    id: row.get(0)?,
                    text: row.get(1)?,
                    answer: row.get(2)?,
                    created_at: row.get(3)?,
                    answered_at: row.get(4)?,
                })
            })?
            .flatten()
            .collect();
        Ok(questions)
    }

    fn search_questions(&self, query: &str) -> Result<Vec<Question>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT q.id, q.text, q.answer, q.created_at, q.answered_at
             FROM questions q
             JOIN questions_fts fts ON q.id = fts.id
             WHERE questions_fts MATCH ?1
             ORDER BY rank",
        )?;
        let questions: Vec<Question> = stmt
            .query_map(params![query], |row| {
                Ok(Question {
                    id: row.get(0)?,
                    text: row.get(1)?,
                    answer: row.get(2)?,
                    created_at: row.get(3)?,
                    answered_at: row.get(4)?,
                })
            })?
            .flatten()
            .collect();
        Ok(questions)
    }

    fn link_task_to_question(&self, task_id: &str, question_id: &str) -> Result<()> {
        let conn = self.open()?;

        // Verify task exists
        let task_exists: bool = conn
            .query_row("SELECT 1 FROM tasks WHERE id = ?1", params![task_id], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if !task_exists {
            return Err(crate::error::Error::Task(Box::new(TaskNotFound(task_id.to_string()))));
        }

        // Verify question exists
        let question_exists: bool = conn
            .query_row("SELECT 1 FROM questions WHERE id = ?1", params![question_id], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if !question_exists {
            return Err(crate::error::Error::Task(Box::new(QuestionNotFound(
                question_id.to_string(),
            ))));
        }

        conn.execute(
            "INSERT OR IGNORE INTO task_questions (task_id, question_id) VALUES (?1, ?2)",
            params![task_id, question_id],
        )?;

        Ok(())
    }

    fn unlink_task_from_question(&self, task_id: &str, question_id: &str) -> Result<bool> {
        let conn = self.open()?;
        let rows_affected = conn.execute(
            "DELETE FROM task_questions WHERE task_id = ?1 AND question_id = ?2",
            params![task_id, question_id],
        )?;
        let deleted = rows_affected > 0;
        Ok(deleted)
    }

    fn get_task_questions(&self, task_id: &str) -> Result<Vec<String>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT question_id FROM task_questions WHERE task_id = ?1 ORDER BY question_id",
        )?;
        let ids: Vec<String> =
            stmt.query_map(params![task_id], |row| row.get(0))?.flatten().collect();
        Ok(ids)
    }

    fn get_blocking_questions(&self, task_id: &str) -> Result<Vec<Question>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT q.id, q.text, q.answer, q.created_at, q.answered_at
             FROM questions q
             JOIN task_questions tq ON q.id = tq.question_id
             WHERE tq.task_id = ?1 AND q.answer IS NULL
             ORDER BY q.created_at",
        )?;
        let questions: Vec<Question> = stmt
            .query_map(params![task_id], |row| {
                Ok(Question {
                    id: row.get(0)?,
                    text: row.get(1)?,
                    answer: row.get(2)?,
                    created_at: row.get(3)?,
                    answered_at: row.get(4)?,
                })
            })?
            .flatten()
            .collect();
        Ok(questions)
    }

    fn get_question_blocked_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.open()?;
        // Get tasks that:
        // 1. Are open (not complete, abandoned, etc.)
        // 2. Have at least one unanswered question linked
        // 3. Are NOT blocked by incomplete dependencies
        let mut stmt = conn.prepare(
            "SELECT DISTINCT t.id, t.title, t.description, t.priority, t.status,
                    t.in_progress, t.requested, t.created_at, t.updated_at
             FROM tasks t
             JOIN task_questions tq ON t.id = tq.task_id
             JOIN questions q ON tq.question_id = q.id
             WHERE t.status = 'open'
               AND q.answer IS NULL
               AND NOT EXISTS (
                   SELECT 1 FROM task_dependencies td
                   JOIN tasks dep ON td.depends_on = dep.id
                   WHERE td.task_id = t.id
                     AND dep.status NOT IN ('complete', 'abandoned')
               )
             ORDER BY t.priority, t.created_at",
        )?;
        let tasks: Vec<Task> = stmt.query_map([], Self::parse_task)?.flatten().collect();
        Ok(tasks)
    }

    fn has_in_progress_task(&self) -> Result<bool> {
        let conn = self.open()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM tasks WHERE in_progress = 1", [], |row| {
                row.get(0)
            })?;
        Ok(count > 0)
    }

    fn get_in_progress_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
             FROM tasks
             WHERE in_progress = 1
             ORDER BY priority, created_at",
        )?;
        let tasks: Vec<Task> = stmt.query_map([], Self::parse_task)?.flatten().collect();
        Ok(tasks)
    }

    fn request_tasks(&self, task_ids: &[&str]) -> Result<usize> {
        if task_ids.is_empty() {
            return Ok(0);
        }

        let conn = self.open()?;

        // Build IN clause with placeholders
        let placeholders: Vec<&str> = task_ids.iter().map(|_| "?").collect();
        let sql = format!(
            "UPDATE tasks SET requested = 1, updated_at = datetime('now') WHERE id IN ({})",
            placeholders.join(", ")
        );

        let params: Vec<&dyn rusqlite::ToSql> =
            task_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let updated = conn.execute(&sql, params.as_slice())?;

        // Log audit for each task
        for task_id in task_ids {
            Self::log_audit(
                &conn,
                "request",
                Some(task_id),
                None,
                None,
                Some("marked as requested"),
            )?;
        }

        Ok(updated)
    }

    fn request_all_open(&self) -> Result<usize> {
        let conn = self.open()?;

        // Mark all open/stuck tasks as requested
        let updated = conn.execute(
            "UPDATE tasks SET requested = 1, updated_at = datetime('now')
             WHERE status IN ('open', 'stuck', 'blocked')",
            [],
        )?;

        // Enable request mode for future tasks
        conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('request_mode_active', 'true')",
            [],
        )?;

        Self::log_audit(&conn, "request_all", None, None, None, Some("enabled request mode"))?;

        Ok(updated)
    }

    fn is_request_mode_active(&self) -> Result<bool> {
        let conn = self.open()?;
        Ok(Self::is_request_mode_active_internal(&conn))
    }

    fn clear_request_mode(&self) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM metadata WHERE key = 'request_mode_active'", [])?;
        Self::log_audit(
            &conn,
            "clear_request_mode",
            None,
            None,
            None,
            Some("disabled request mode"),
        )?;
        Ok(())
    }

    fn get_incomplete_requested_work(&self) -> Result<Vec<Task>> {
        let conn = self.open()?;

        // First, get all directly requested incomplete tasks
        let mut stmt = conn.prepare(
            "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
             FROM tasks
             WHERE requested = 1
               AND status NOT IN ('complete', 'abandoned')
             ORDER BY priority, created_at",
        )?;
        let direct_requested: Vec<Task> = stmt.query_map([], Self::parse_task)?.flatten().collect();

        if direct_requested.is_empty() {
            return Ok(vec![]);
        }

        // Get task IDs
        let task_ids: Vec<String> = direct_requested.iter().map(|t| t.id.clone()).collect();

        // Get transitive dependencies (tasks that requested tasks depend on)
        let dep_ids = Self::get_transitive_dependencies(&conn, &task_ids)?;

        // Filter to incomplete dependencies that are not blocked on questions
        let mut result: Vec<Task> = direct_requested
            .into_iter()
            .filter(|t| !Self::is_blocked_on_question_only(&conn, &t.id))
            .collect();

        // Add incomplete dependencies (treating them as transitively requested)
        for dep_id in dep_ids {
            let task = conn
                .query_row(
                    "SELECT id, title, description, priority, status, in_progress, requested, created_at, updated_at
                     FROM tasks WHERE id = ?1",
                    params![&dep_id],
                    Self::parse_task,
                )
                .optional()?;

            if let Some(task) = task {
                // Include if incomplete and not blocked on a question
                if !matches!(task.status, Status::Complete | Status::Abandoned)
                    && !Self::is_blocked_on_question_only(&conn, &task.id)
                    && !result.iter().any(|t| t.id == task.id)
                {
                    result.push(task);
                }
            }
        }

        // Sort by priority (ascending) then dependent count (descending)
        result.sort_by(|a, b| {
            let a_priority = a.priority.as_u8();
            let b_priority = b.priority.as_u8();
            let priority_cmp = a_priority.cmp(&b_priority);
            if priority_cmp != std::cmp::Ordering::Equal {
                return priority_cmp;
            }
            // For same priority, sort by dependent count (descending)
            let a_deps = self.get_dependents(&a.id).map(|d| d.len()).unwrap_or(0);
            let b_deps = self.get_dependents(&b.id).map(|d| d.len()).unwrap_or(0);
            b_deps.cmp(&a_deps)
        });

        // Limit to top 5
        result.truncate(5);

        Ok(result)
    }
}

impl SqliteTaskStore {
    /// Check if a task is blocked only by unanswered questions (not by dependencies).
    fn is_blocked_on_question_only(conn: &Connection, task_id: &str) -> bool {
        // Check if task has unanswered questions
        let has_unanswered_questions: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM task_questions tq
                    JOIN questions q ON tq.question_id = q.id
                    WHERE tq.task_id = ?1 AND q.answer IS NULL
                )",
                params![task_id],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_unanswered_questions {
            return false;
        }

        // Check if task has incomplete dependencies (other than questions)
        let has_incomplete_deps: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM task_dependencies d
                    JOIN tasks t ON d.depends_on = t.id
                    WHERE d.task_id = ?1 AND t.status NOT IN ('complete', 'abandoned')
                )",
                params![task_id],
                |row| row.get(0),
            )
            .unwrap_or(false);

        // Blocked on question only if has questions but no other blockers
        !has_incomplete_deps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::id::{disable_deterministic_ids, enable_deterministic_ids};
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, SqliteTaskStore) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let store = SqliteTaskStore::new(&db_path).unwrap();
        (dir, store)
    }

    #[test]
    fn test_now_timestamp_format() {
        let ts = now_timestamp();
        // Format should be "YYYY-MM-DD HH:MM:SS"
        assert_eq!(ts.len(), 19);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], " ");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
    }

    #[test]
    fn test_create_and_get_task() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Test Task", "A description", Priority::High).unwrap();
        assert!(task.id.starts_with("test-task-"));
        assert_eq!(task.title, "Test Task");
        assert_eq!(task.description, "A description");
        assert_eq!(task.priority, Priority::High);
        assert_eq!(task.status, Status::Open);

        let fetched = store.get_task(&task.id).unwrap().unwrap();
        assert_eq!(fetched.id, task.id);
        assert_eq!(fetched.title, task.title);

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_nonexistent_task() {
        let (_dir, store) = create_test_store();
        let task = store.get_task("nonexistent").unwrap();
        assert!(task.is_none());
    }

    #[test]
    fn test_update_task() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Original", "", Priority::Medium).unwrap();

        let updated = store
            .update_task(
                &task.id,
                TaskUpdate {
                    title: Some("Updated".to_string()),
                    priority: Some(Priority::Critical),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(updated.title, "Updated");
        assert_eq!(updated.priority, Priority::Critical);

        disable_deterministic_ids();
    }

    #[test]
    fn test_update_nonexistent_task() {
        let (_dir, store) = create_test_store();
        let result = store
            .update_task(
                "nonexistent",
                TaskUpdate { title: Some("Test".to_string()), ..Default::default() },
            )
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_empty_does_nothing() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Test", "", Priority::Medium).unwrap();
        let result = store.update_task(&task.id, TaskUpdate::default()).unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap().title, "Test");

        disable_deterministic_ids();
    }

    #[test]
    fn test_delete_task() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("To Delete", "", Priority::Medium).unwrap();
        assert!(store.delete_task(&task.id).unwrap());
        assert!(store.get_task(&task.id).unwrap().is_none());

        // Delete again returns false
        assert!(!store.delete_task(&task.id).unwrap());

        disable_deterministic_ids();
    }

    #[test]
    fn test_list_tasks_no_filter() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        store.create_task("Task 1", "", Priority::Low).unwrap();
        store.create_task("Task 2", "", Priority::High).unwrap();
        store.create_task("Task 3", "", Priority::Medium).unwrap();

        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 3);
        // Should be ordered by priority (High < Medium < Low)
        assert_eq!(tasks[0].priority, Priority::High);
        assert_eq!(tasks[1].priority, Priority::Medium);
        assert_eq!(tasks[2].priority, Priority::Low);

        disable_deterministic_ids();
    }

    #[test]
    fn test_list_tasks_with_status_filter() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Open Task", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Complete Task", "", Priority::Medium).unwrap();

        store
            .update_task(
                &task2.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        let open_tasks = store
            .list_tasks(TaskFilter { status: Some(Status::Open), ..Default::default() })
            .unwrap();
        assert_eq!(open_tasks.len(), 1);
        assert_eq!(open_tasks[0].id, task1.id);

        let complete_tasks = store
            .list_tasks(TaskFilter { status: Some(Status::Complete), ..Default::default() })
            .unwrap();
        assert_eq!(complete_tasks.len(), 1);
        assert_eq!(complete_tasks[0].id, task2.id);

        disable_deterministic_ids();
    }

    #[test]
    fn test_list_tasks_with_priority_filter() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        store.create_task("High", "", Priority::High).unwrap();
        store.create_task("Medium", "", Priority::Medium).unwrap();
        store.create_task("Low", "", Priority::Low).unwrap();

        let high_tasks = store
            .list_tasks(TaskFilter { priority: Some(Priority::High), ..Default::default() })
            .unwrap();
        assert_eq!(high_tasks.len(), 1);
        assert_eq!(high_tasks[0].title, "High");

        disable_deterministic_ids();
    }

    #[test]
    fn test_list_tasks_with_max_priority_filter() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        store.create_task("Critical", "", Priority::Critical).unwrap();
        store.create_task("High", "", Priority::High).unwrap();
        store.create_task("Medium", "", Priority::Medium).unwrap();
        store.create_task("Low", "", Priority::Low).unwrap();

        let tasks = store
            .list_tasks(TaskFilter { max_priority: Some(Priority::High), ..Default::default() })
            .unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().all(|t| t.priority <= Priority::High));

        disable_deterministic_ids();
    }

    #[test]
    fn test_list_tasks_with_paging() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        // Create 5 tasks
        for i in 1..=5 {
            store.create_task(&format!("Task {i}"), "", Priority::Medium).unwrap();
        }

        // Test limit only
        let tasks = store.list_tasks(TaskFilter { limit: Some(2), ..Default::default() }).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, "Task 1");
        assert_eq!(tasks[1].title, "Task 2");

        // Test offset only
        let tasks = store.list_tasks(TaskFilter { offset: Some(2), ..Default::default() }).unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].title, "Task 3");

        // Test limit and offset together
        let tasks = store
            .list_tasks(TaskFilter { limit: Some(2), offset: Some(1), ..Default::default() })
            .unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, "Task 2");
        assert_eq!(tasks[1].title, "Task 3");

        // Test offset beyond results
        let tasks =
            store.list_tasks(TaskFilter { offset: Some(10), ..Default::default() }).unwrap();
        assert_eq!(tasks.len(), 0);

        disable_deterministic_ids();
    }

    #[test]
    fn test_list_tasks_sort_order() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        // Create tasks with various properties to test sort order:
        // 1. Open status first
        // 2. Unblocked first
        // 3. Requested first
        // 4. Higher priority first (lower number)
        // 5. Tasks that block more others first
        // 6. Older tasks first

        // Create tasks in a specific order to test sorting
        // All same priority, same creation order - will test other factors

        // Task A: open, unblocked, requested, blocks 2 others
        let task_a = store.create_task("A - Best to work on", "", Priority::Medium).unwrap();
        store
            .update_task(&task_a.id, TaskUpdate { requested: Some(true), ..Default::default() })
            .unwrap();

        // Task B: open, unblocked, not requested
        let _task_b =
            store.create_task("B - Unblocked not requested", "", Priority::Medium).unwrap();

        // Task C: open, blocked by A, requested
        let task_c = store.create_task("C - Blocked but requested", "", Priority::Medium).unwrap();
        store.add_dependency(&task_c.id, &task_a.id).unwrap();
        store
            .update_task(&task_c.id, TaskUpdate { requested: Some(true), ..Default::default() })
            .unwrap();

        // Task D: completed
        let task_d = store.create_task("D - Completed", "", Priority::Medium).unwrap();
        store
            .update_task(
                &task_d.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        // Task E: open, higher priority than A/B
        let _task_e = store.create_task("E - High priority", "", Priority::High).unwrap();

        // Make task A block two others (C already depends on A, add another)
        let task_f = store.create_task("F - Also blocked by A", "", Priority::Medium).unwrap();
        store.add_dependency(&task_f.id, &task_a.id).unwrap();

        // Get all tasks with no filter
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();

        // Expected order:
        // Status: open (0) > stuck (1) > blocked (2) > complete (3) > abandoned (4)
        // Within same status: unblocked > blocked by deps
        // Within same block state: requested > not requested
        // Within same requested: higher priority (lower number)
        // Within same priority: blocks more others > blocks fewer
        // Within same blocking count: older > newer
        //
        // 1. A - open, unblocked, REQUESTED, blocks 2 (medium priority)
        // 2. E - open, unblocked, not requested, high priority, blocks 0
        // 3. B - open, unblocked, not requested, medium priority, blocks 0
        // 4. C - status=blocked, requested
        // 5. F - status=blocked, not requested
        // 6. D - completed

        assert_eq!(tasks.len(), 6, "Should have 6 tasks");

        // First should be A (open, unblocked, REQUESTED - requested beats priority)
        assert_eq!(
            tasks[0].title, "A - Best to work on",
            "Requested unblocked task should be first (requested beats priority)"
        );

        // Second should be E (open, unblocked, not requested, high priority)
        assert_eq!(tasks[1].title, "E - High priority", "High priority unblocked should be second");

        // Third should be B (open, unblocked, not requested, medium priority, blocks none)
        assert_eq!(
            tasks[2].title, "B - Unblocked not requested",
            "Unblocked non-requested medium priority should be third"
        );

        // Fourth and fifth should be C and F (status=blocked, sorted by requested)
        // C is requested, F is not, so C should come first
        assert_eq!(
            tasks[3].title, "C - Blocked but requested",
            "Blocked requested should be fourth"
        );
        assert_eq!(
            tasks[4].title, "F - Also blocked by A",
            "Blocked non-requested should be fifth"
        );

        // Last should be D (completed)
        assert_eq!(tasks[5].title, "D - Completed", "Completed should be last");

        disable_deterministic_ids();
    }

    #[test]
    fn test_dependencies_basic() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();

        store.add_dependency(&task2.id, &task1.id).unwrap();

        let deps = store.get_dependencies(&task2.id).unwrap();
        assert_eq!(deps, vec![task1.id.as_str()]);

        let dependents = store.get_dependents(&task1.id).unwrap();
        assert_eq!(dependents, vec![task2.id.as_str()]);

        disable_deterministic_ids();
    }

    #[test]
    fn test_dependency_blocks_task() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();

        store.add_dependency(&task2.id, &task1.id).unwrap();

        let task2_updated = store.get_task(&task2.id).unwrap().unwrap();
        assert_eq!(task2_updated.status, Status::Blocked);

        // Complete task1
        store
            .update_task(
                &task1.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        let task2_updated = store.get_task(&task2.id).unwrap().unwrap();
        assert_eq!(task2_updated.status, Status::Open);

        disable_deterministic_ids();
    }

    #[test]
    fn test_dependency_cycle_detection() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();
        let task3 = store.create_task("Task 3", "", Priority::Medium).unwrap();

        // Create chain: task3 -> task2 -> task1
        store.add_dependency(&task2.id, &task1.id).unwrap();
        store.add_dependency(&task3.id, &task2.id).unwrap();

        // Try to create cycle: task1 -> task3
        let result = store.add_dependency(&task1.id, &task3.id);
        assert!(result.is_err());

        disable_deterministic_ids();
    }

    #[test]
    fn test_dependency_nonexistent_task() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();

        let result = store.add_dependency(&task.id, "nonexistent");
        assert!(result.is_err());

        let result = store.add_dependency("nonexistent", &task.id);
        assert!(result.is_err());

        disable_deterministic_ids();
    }

    #[test]
    fn test_remove_dependency() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();

        store.add_dependency(&task2.id, &task1.id).unwrap();
        assert_eq!(store.get_task(&task2.id).unwrap().unwrap().status, Status::Blocked);

        assert!(store.remove_dependency(&task2.id, &task1.id).unwrap());
        assert_eq!(store.get_task(&task2.id).unwrap().unwrap().status, Status::Open);

        // Remove again returns false
        assert!(!store.remove_dependency(&task2.id, &task1.id).unwrap());

        disable_deterministic_ids();
    }

    #[test]
    fn test_notes() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();

        let first_note = store.add_note(&task.id, "First note").unwrap();
        assert_eq!(first_note.content, "First note");
        assert_eq!(first_note.task_id, task.id);

        let _second_note = store.add_note(&task.id, "Second note").unwrap();

        let all_notes = store.get_notes(&task.id).unwrap();
        assert_eq!(all_notes.len(), 2);
        assert_eq!(all_notes[0].content, "First note");
        assert_eq!(all_notes[1].content, "Second note");

        assert!(store.delete_note(first_note.id).unwrap());
        assert!(!store.delete_note(first_note.id).unwrap());

        let remaining_notes = store.get_notes(&task.id).unwrap();
        assert_eq!(remaining_notes.len(), 1);

        disable_deterministic_ids();
    }

    #[test]
    fn test_note_nonexistent_task() {
        let (_dir, store) = create_test_store();
        let result = store.add_note("nonexistent", "Note");
        assert!(result.is_err());
    }

    #[test]
    fn test_search_tasks() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store
            .create_task("Fix authentication bug", "Users cannot login", Priority::High)
            .unwrap();
        store.create_task("Add new feature", "Implement dashboard", Priority::Medium).unwrap();
        store.add_note(&task1.id, "Related to OAuth").unwrap();

        // Search by title
        let results = store.search_tasks("authentication").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, task1.id);

        // Search by description
        let results = store.search_tasks("dashboard").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Add new feature");

        // Search by note
        let results = store.search_tasks("OAuth").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, task1.id);

        disable_deterministic_ids();
    }

    #[test]
    fn test_search_tasks_sorted_by_priority() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        // Create tasks with different priorities but same search term
        store.create_task("Low priority common", "", Priority::Low).unwrap();
        store.create_task("High priority common", "", Priority::High).unwrap();
        store.create_task("Medium priority common", "", Priority::Medium).unwrap();

        // Search should return results sorted by priority
        let results = store.search_tasks("common").unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].priority, Priority::High);
        assert_eq!(results[1].priority, Priority::Medium);
        assert_eq!(results[2].priority, Priority::Low);

        disable_deterministic_ids();
    }

    #[test]
    fn test_audit_log() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Test", "", Priority::Medium).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { title: Some("Updated".to_string()), ..Default::default() },
            )
            .unwrap();
        store.add_note(&task.id, "A note").unwrap();
        store.delete_task(&task.id).unwrap();

        let log = store.get_audit_log(None, None).unwrap();
        assert!(log.len() >= 4);

        let ops: Vec<_> = log.iter().map(|e| e.operation.as_str()).collect();
        assert!(ops.contains(&"create"));
        assert!(ops.contains(&"update"));
        assert!(ops.contains(&"add_note"));
        assert!(ops.contains(&"delete"));

        disable_deterministic_ids();
    }

    #[test]
    fn test_audit_log_filtered() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();

        store
            .update_task(
                &task1.id,
                TaskUpdate { title: Some("Updated 1".to_string()), ..Default::default() },
            )
            .unwrap();
        store
            .update_task(
                &task2.id,
                TaskUpdate { title: Some("Updated 2".to_string()), ..Default::default() },
            )
            .unwrap();

        let log = store.get_audit_log(Some(&task1.id), None).unwrap();
        assert!(log.iter().all(|e| e.task_id.as_ref() == Some(&task1.id)));

        disable_deterministic_ids();
    }

    #[test]
    fn test_audit_log_limited() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        for i in 0..10 {
            store.create_task(&format!("Task {i}"), "", Priority::Medium).unwrap();
        }

        let log = store.get_audit_log(None, Some(5)).unwrap();
        assert_eq!(log.len(), 5);

        disable_deterministic_ids();
    }

    #[test]
    fn test_audit_log_with_task_id_and_limit() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();

        // Create multiple updates for the same task
        for i in 0..5 {
            store
                .update_task(
                    &task.id,
                    TaskUpdate { title: Some(format!("Update {i}")), ..Default::default() },
                )
                .unwrap();
        }

        // Get audit log with both task_id and limit
        let log = store.get_audit_log(Some(&task.id), Some(3)).unwrap();
        assert_eq!(log.len(), 3);
        assert!(log.iter().all(|e| e.task_id.as_ref() == Some(&task.id)));

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_ready_tasks() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Ready 1", "", Priority::High).unwrap();
        let task2 = store.create_task("Ready 2", "", Priority::Low).unwrap();
        let task3 = store.create_task("Blocked", "", Priority::Medium).unwrap();

        store.add_dependency(&task3.id, &task1.id).unwrap();

        let ready = store.get_ready_tasks().unwrap();
        assert_eq!(ready.len(), 2);
        let ids: Vec<_> = ready.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&task1.id.as_str()));
        assert!(ids.contains(&task2.id.as_str()));
        assert!(!ids.contains(&task3.id.as_str()));

        disable_deterministic_ids();
    }

    #[test]
    fn test_pick_task() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        store.create_task("High 1", "", Priority::High).unwrap();
        store.create_task("High 2", "", Priority::High).unwrap();
        store.create_task("Low", "", Priority::Low).unwrap();

        let picked = store.pick_task().unwrap().unwrap();
        // Should pick from high priority tasks
        assert_eq!(picked.priority, Priority::High);

        disable_deterministic_ids();
    }

    #[test]
    fn test_pick_task_empty() {
        let (_dir, store) = create_test_store();
        let picked = store.pick_task().unwrap();
        assert!(picked.is_none());
    }

    #[test]
    fn test_for_project() {
        let dir = TempDir::new().unwrap();
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        // Database should be in <project>/.claude-reliability/
        let path_str = store.db_path().to_string_lossy();
        assert!(path_str.contains(".claude-reliability"));
    }

    #[test]
    fn test_task_update_is_empty() {
        let update = TaskUpdate::default();
        assert!(update.is_empty());

        let update = TaskUpdate { title: Some("Test".to_string()), ..Default::default() };
        assert!(!update.is_empty());
    }

    #[test]
    fn test_circular_dependency_display() {
        let err = CircularDependency { task_id: "a".to_string(), depends_on: "b".to_string() };
        assert!(err.to_string().contains("cycle"));
    }

    #[test]
    fn test_task_not_found_display() {
        let err = TaskNotFound("test-123".to_string());
        assert!(err.to_string().contains("test-123"));
    }

    #[test]
    fn test_delete_task_updates_dependents() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Dependency", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Dependent", "", Priority::Medium).unwrap();

        store.add_dependency(&task2.id, &task1.id).unwrap();
        assert_eq!(store.get_task(&task2.id).unwrap().unwrap().status, Status::Blocked);

        // Delete the dependency task
        store.delete_task(&task1.id).unwrap();

        // Dependent should now be unblocked
        assert_eq!(store.get_task(&task2.id).unwrap().unwrap().status, Status::Open);

        disable_deterministic_ids();
    }

    #[test]
    fn test_deleting_task_cascades_notes() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        store.add_note(&task.id, "Note 1").unwrap();
        store.add_note(&task.id, "Note 2").unwrap();

        assert_eq!(store.get_notes(&task.id).unwrap().len(), 2);

        store.delete_task(&task.id).unwrap();

        // Notes should be deleted via cascade
        assert!(store.get_notes(&task.id).unwrap().is_empty());

        disable_deterministic_ids();
    }

    #[test]
    fn test_update_description() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "Old desc", Priority::Medium).unwrap();

        let updated = store
            .update_task(
                &task.id,
                TaskUpdate { description: Some("New desc".to_string()), ..Default::default() },
            )
            .unwrap()
            .unwrap();

        assert_eq!(updated.description, "New desc");

        disable_deterministic_ids();
    }

    #[test]
    fn test_update_status() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();

        let updated = store
            .update_task(&task.id, TaskUpdate { status: Some(Status::Stuck), ..Default::default() })
            .unwrap()
            .unwrap();

        assert_eq!(updated.status, Status::Stuck);

        disable_deterministic_ids();
    }

    #[test]
    fn test_migration_adds_in_progress_column() {
        use rusqlite::Connection;

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");

        // Create a database with old schema (without in_progress column)
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                r"
                CREATE TABLE tasks (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    description TEXT DEFAULT '',
                    priority INTEGER NOT NULL DEFAULT 2,
                    status TEXT NOT NULL DEFAULT 'open',
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                INSERT INTO tasks (id, title) VALUES ('test-1234', 'Old Task');
                ",
            )
            .unwrap();
        }

        // Open with new code - should migrate
        let store = SqliteTaskStore::new(&db_path).unwrap();

        // Verify the column was added and task still exists
        let task = store.get_task("test-1234").unwrap().unwrap();
        assert_eq!(task.title, "Old Task");
        assert!(!task.in_progress); // Default value
    }

    #[test]
    fn test_in_progress_flag() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        // Create two tasks
        let task1 = store.create_task("Task 1", "", Priority::Medium).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::High).unwrap();

        // Initially no tasks in progress
        assert!(!store.has_in_progress_task().unwrap());
        assert!(store.get_in_progress_tasks().unwrap().is_empty());

        // Mark task1 as in progress
        store
            .update_task(&task1.id, TaskUpdate { in_progress: Some(true), ..Default::default() })
            .unwrap();

        assert!(store.has_in_progress_task().unwrap());
        let in_progress = store.get_in_progress_tasks().unwrap();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].id, task1.id);
        assert!(in_progress[0].in_progress);

        // Mark task2 as in progress too
        store
            .update_task(&task2.id, TaskUpdate { in_progress: Some(true), ..Default::default() })
            .unwrap();

        let in_progress = store.get_in_progress_tasks().unwrap();
        assert_eq!(in_progress.len(), 2);

        // Complete task1 - should auto-clear in_progress
        let updated = store
            .update_task(
                &task1.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap()
            .unwrap();
        assert!(!updated.in_progress);

        // Only task2 should be in progress now
        let in_progress = store.get_in_progress_tasks().unwrap();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].id, task2.id);

        disable_deterministic_ids();
    }

    // HowTo tests

    #[test]
    fn test_create_howto() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let howto = store.create_howto("Deploy to Prod", "Run the deploy script").unwrap();
        assert!(howto.id.starts_with("deploy-to-prod-"));
        assert_eq!(howto.title, "Deploy to Prod");
        assert_eq!(howto.instructions, "Run the deploy script");

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_howto() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let howto = store.create_howto("Test Guide", "Testing instructions").unwrap();
        let retrieved = store.get_howto(&howto.id).unwrap();

        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, howto.id);
        assert_eq!(retrieved.title, "Test Guide");

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_howto_not_found() {
        let (_dir, store) = create_test_store();
        let result = store.get_howto("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_howto() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let howto = store.create_howto("Original", "Original instructions").unwrap();

        let updated = store
            .update_howto(
                &howto.id,
                HowToUpdate { title: Some("Updated".to_string()), instructions: None },
            )
            .unwrap()
            .unwrap();

        assert_eq!(updated.title, "Updated");
        assert_eq!(updated.instructions, "Original instructions");

        disable_deterministic_ids();
    }

    #[test]
    fn test_update_howto_instructions() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let howto = store.create_howto("Guide", "Old").unwrap();

        let updated = store
            .update_howto(
                &howto.id,
                HowToUpdate { title: None, instructions: Some("New instructions".to_string()) },
            )
            .unwrap()
            .unwrap();

        assert_eq!(updated.instructions, "New instructions");

        disable_deterministic_ids();
    }

    #[test]
    fn test_update_howto_empty_does_nothing() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let howto = store.create_howto("Guide", "Instructions").unwrap();
        let updated = store.update_howto(&howto.id, HowToUpdate::default()).unwrap().unwrap();

        assert_eq!(updated.title, howto.title);
        assert_eq!(updated.instructions, howto.instructions);

        disable_deterministic_ids();
    }

    #[test]
    fn test_update_howto_not_found() {
        let (_dir, store) = create_test_store();
        let result = store
            .update_howto(
                "nonexistent",
                HowToUpdate { title: Some("X".to_string()), ..Default::default() },
            )
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_howto() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let howto = store.create_howto("To Delete", "Instructions").unwrap();
        assert!(store.delete_howto(&howto.id).unwrap());
        assert!(store.get_howto(&howto.id).unwrap().is_none());

        disable_deterministic_ids();
    }

    #[test]
    fn test_delete_howto_not_found() {
        let (_dir, store) = create_test_store();
        assert!(!store.delete_howto("nonexistent").unwrap());
    }

    #[test]
    fn test_list_howtos() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        // Store starts with 1 built-in how-to
        let initial_count = store.list_howtos().unwrap().len();

        store.create_howto("B Guide", "B").unwrap();
        store.create_howto("A Guide", "A").unwrap();

        let howtos = store.list_howtos().unwrap();
        assert_eq!(howtos.len(), initial_count + 2);
        // Ordered by title - A Guide should be first of user-created
        let user_howtos: Vec<_> =
            howtos.into_iter().filter(|h| !h.id.starts_with("builtin-")).collect();
        assert_eq!(user_howtos.len(), 2);
        assert_eq!(user_howtos[0].title, "A Guide");
        assert_eq!(user_howtos[1].title, "B Guide");

        disable_deterministic_ids();
    }

    #[test]
    fn test_search_howtos() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        store.create_howto("Unique Widget Guide", "Run widget.sh").unwrap();
        store.create_howto("Testing Guide", "Run pytest").unwrap();

        // Search for something unique that won't match built-in how-tos
        let results = store.search_howtos("widget").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Unique Widget Guide");

        disable_deterministic_ids();
    }

    #[test]
    fn test_link_task_to_howto() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Fix Bug", "", Priority::High).unwrap();
        let howto = store.create_howto("Debugging Guide", "Use debugger").unwrap();

        store.link_task_to_howto(&task.id, &howto.id).unwrap();

        let guidance = store.get_task_guidance(&task.id).unwrap();
        assert_eq!(guidance.len(), 1);
        assert_eq!(guidance[0], howto.id);

        disable_deterministic_ids();
    }

    #[test]
    fn test_link_task_to_howto_task_not_found() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let howto = store.create_howto("Guide", "Instructions").unwrap();
        let result = store.link_task_to_howto("nonexistent", &howto.id);

        assert!(result.is_err());

        disable_deterministic_ids();
    }

    #[test]
    fn test_link_task_to_howto_howto_not_found() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        let result = store.link_task_to_howto(&task.id, "nonexistent");

        assert!(result.is_err());

        disable_deterministic_ids();
    }

    #[test]
    fn test_unlink_task_from_howto() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        let howto = store.create_howto("Guide", "Instructions").unwrap();

        store.link_task_to_howto(&task.id, &howto.id).unwrap();
        assert!(store.unlink_task_from_howto(&task.id, &howto.id).unwrap());

        let guidance = store.get_task_guidance(&task.id).unwrap();
        assert!(guidance.is_empty());

        disable_deterministic_ids();
    }

    #[test]
    fn test_unlink_task_from_howto_not_linked() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        let howto = store.create_howto("Guide", "Instructions").unwrap();

        assert!(!store.unlink_task_from_howto(&task.id, &howto.id).unwrap());

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_task_guidance_multiple() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Complex Task", "", Priority::High).unwrap();
        let howto1 = store.create_howto("Guide A", "A").unwrap();
        let howto2 = store.create_howto("Guide B", "B").unwrap();

        store.link_task_to_howto(&task.id, &howto1.id).unwrap();
        store.link_task_to_howto(&task.id, &howto2.id).unwrap();

        let guidance = store.get_task_guidance(&task.id).unwrap();
        assert_eq!(guidance.len(), 2);

        disable_deterministic_ids();
    }

    #[test]
    fn test_howto_not_found_display() {
        let err = HowToNotFound("test-123".to_string());
        assert!(err.to_string().contains("test-123"));
        assert!(err.to_string().contains("how-to not found"));
    }

    #[test]
    fn test_howto_update_is_empty() {
        let empty = HowToUpdate::default();
        assert!(empty.is_empty());

        let with_title = HowToUpdate { title: Some("Title".to_string()), ..Default::default() };
        assert!(!with_title.is_empty());

        let with_instructions =
            HowToUpdate { instructions: Some("Instructions".to_string()), ..Default::default() };
        assert!(!with_instructions.is_empty());
    }

    #[test]
    fn test_howto_cascade_delete_guidance() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        let howto = store.create_howto("Guide", "Instructions").unwrap();

        store.link_task_to_howto(&task.id, &howto.id).unwrap();

        // Delete the how-to - guidance should be cascade deleted
        store.delete_howto(&howto.id).unwrap();

        let guidance = store.get_task_guidance(&task.id).unwrap();
        assert!(guidance.is_empty());

        disable_deterministic_ids();
    }

    #[test]
    fn test_task_cascade_delete_guidance() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        let howto = store.create_howto("Guide", "Instructions").unwrap();

        store.link_task_to_howto(&task.id, &howto.id).unwrap();

        // Delete the task - guidance should be cascade deleted
        store.delete_task(&task.id).unwrap();

        // HowTo should still exist
        assert!(store.get_howto(&howto.id).unwrap().is_some());

        disable_deterministic_ids();
    }

    #[test]
    fn test_question_not_found_display() {
        let err = QuestionNotFound("q123".to_string());
        assert_eq!(format!("{err}"), "question not found: q123");
    }

    #[test]
    fn test_delete_question() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let question = store.create_question("Test question?").unwrap();
        assert!(store.get_question(&question.id).unwrap().is_some());

        let deleted = store.delete_question(&question.id).unwrap();
        assert!(deleted);
        assert!(store.get_question(&question.id).unwrap().is_none());

        // Deleting again should return false
        let deleted_again = store.delete_question(&question.id).unwrap();
        assert!(!deleted_again);

        disable_deterministic_ids();
    }

    #[test]
    fn test_list_questions_all() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let q1 = store.create_question("Question 1?").unwrap();
        let q2 = store.create_question("Question 2?").unwrap();
        store.answer_question(&q1.id, "Answer 1").unwrap();

        // List all questions
        let all = store.list_questions(false).unwrap();
        assert_eq!(all.len(), 2);

        // List only unanswered
        let unanswered = store.list_questions(true).unwrap();
        assert_eq!(unanswered.len(), 1);
        assert_eq!(unanswered[0].id, q2.id);

        disable_deterministic_ids();
    }

    #[test]
    fn test_search_questions() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        store.create_question("What API format should we use?").unwrap();
        store.create_question("How should authentication work?").unwrap();
        store.create_question("What database should we use?").unwrap();

        let results = store.search_questions("API").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].text.contains("API"));

        let results = store.search_questions("should").unwrap();
        assert_eq!(results.len(), 3);

        disable_deterministic_ids();
    }

    #[test]
    fn test_unlink_task_from_question() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        let question = store.create_question("Question?").unwrap();

        store.link_task_to_question(&task.id, &question.id).unwrap();

        // Should be able to unlink
        let unlinked = store.unlink_task_from_question(&task.id, &question.id).unwrap();
        assert!(unlinked);

        // Unlinking again should return false
        let unlinked_again = store.unlink_task_from_question(&task.id, &question.id).unwrap();
        assert!(!unlinked_again);

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_task_questions() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        let q1 = store.create_question("Question 1?").unwrap();
        let q2 = store.create_question("Question 2?").unwrap();

        store.link_task_to_question(&task.id, &q1.id).unwrap();
        store.link_task_to_question(&task.id, &q2.id).unwrap();

        let question_ids = store.get_task_questions(&task.id).unwrap();
        assert_eq!(question_ids.len(), 2);
        assert!(question_ids.contains(&q1.id));
        assert!(question_ids.contains(&q2.id));

        disable_deterministic_ids();
    }

    #[test]
    fn test_link_task_to_question_task_not_found() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let question = store.create_question("Question?").unwrap();

        let result = store.link_task_to_question("nonexistent-task-1234", &question.id);
        assert!(result.is_err());

        disable_deterministic_ids();
    }

    #[test]
    fn test_link_task_to_question_question_not_found() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();

        let result = store.link_task_to_question(&task.id, "nonexistent-question-1234");
        assert!(result.is_err());

        disable_deterministic_ids();
    }

    #[test]
    fn test_answer_question_not_found() {
        let (_dir, store) = create_test_store();

        // Answering a nonexistent question should return None
        let result = store.answer_question("nonexistent-question-1234", "An answer").unwrap();
        assert!(result.is_none());
    }

    // ========== Requested Tasks Tests ==========

    #[test]
    fn test_request_tasks() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task 1", "", Priority::High).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::Medium).unwrap();
        let _task3 = store.create_task("Task 3", "", Priority::Low).unwrap();

        // Initially, no tasks are requested
        assert!(!task1.requested);
        assert!(!task2.requested);

        // Request specific tasks
        let updated = store.request_tasks(&[&task1.id, &task2.id]).unwrap();
        assert_eq!(updated, 2);

        // Verify tasks are now requested
        let t1 = store.get_task(&task1.id).unwrap().unwrap();
        let t2 = store.get_task(&task2.id).unwrap().unwrap();
        assert!(t1.requested);
        assert!(t2.requested);

        disable_deterministic_ids();
    }

    #[test]
    fn test_request_tasks_empty() {
        let (_dir, store) = create_test_store();

        // Requesting empty list should return 0
        let updated = store.request_tasks(&[]).unwrap();
        assert_eq!(updated, 0);
    }

    #[test]
    fn test_request_all_open() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Open task", "", Priority::High).unwrap();
        let task2 = store.create_task("Another open task", "", Priority::Medium).unwrap();
        store
            .update_task(
                &task2.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        // Request all open tasks
        let updated = store.request_all_open().unwrap();
        assert_eq!(updated, 1); // Only task1 is open

        // Verify request mode is active
        assert!(store.is_request_mode_active().unwrap());

        // Verify task1 is requested, task2 is not
        let t1 = store.get_task(&task1.id).unwrap().unwrap();
        let t2 = store.get_task(&task2.id).unwrap().unwrap();
        assert!(t1.requested);
        assert!(!t2.requested);

        disable_deterministic_ids();
    }

    #[test]
    fn test_clear_request_mode() {
        let (_dir, store) = create_test_store();

        // Enable request mode
        store.request_all_open().unwrap();
        assert!(store.is_request_mode_active().unwrap());

        // Clear request mode
        store.clear_request_mode().unwrap();
        assert!(!store.is_request_mode_active().unwrap());
    }

    #[test]
    fn test_get_incomplete_requested_work() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Requested open", "", Priority::High).unwrap();
        let task2 = store.create_task("Requested complete", "", Priority::Medium).unwrap();
        let task3 = store.create_task("Not requested", "", Priority::Low).unwrap();

        // Request task1 and task2
        store.request_tasks(&[&task1.id, &task2.id]).unwrap();

        // Complete task2
        store
            .update_task(
                &task2.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        // Get incomplete requested tasks
        let incomplete = store.get_incomplete_requested_work().unwrap();

        // Only task1 should be returned (open and requested)
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].id, task1.id);

        // task3 is not requested, so it shouldn't be in the list
        assert!(!incomplete.iter().any(|t| t.id == task3.id));

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_incomplete_requested_work_transitive() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        // Create a chain: task1 depends on task2, task2 depends on task3
        let task3 = store.create_task("Dependency 2", "", Priority::Low).unwrap();
        let task2 = store.create_task("Dependency 1", "", Priority::Medium).unwrap();
        let task1 = store.create_task("Main task", "", Priority::High).unwrap();

        store.add_dependency(&task1.id, &task2.id).unwrap();
        store.add_dependency(&task2.id, &task3.id).unwrap();

        // Only request task1
        store.request_tasks(&[&task1.id]).unwrap();

        // Get incomplete requested tasks - should include task2 and task3 transitively
        let incomplete = store.get_incomplete_requested_work().unwrap();

        assert_eq!(incomplete.len(), 3);
        let ids: Vec<_> = incomplete.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&task1.id.as_str()));
        assert!(ids.contains(&task2.id.as_str()));
        assert!(ids.contains(&task3.id.as_str()));

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_incomplete_requested_work_blocked_on_question() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task with question", "", Priority::High).unwrap();
        let task2 = store.create_task("Normal task", "", Priority::Medium).unwrap();

        // Request both tasks
        store.request_tasks(&[&task1.id, &task2.id]).unwrap();

        // Block task1 with an unanswered question
        let question = store.create_question("What should I do?").unwrap();
        store.link_task_to_question(&task1.id, &question.id).unwrap();

        // Get incomplete requested tasks - task1 should be excluded (blocked on question)
        let incomplete = store.get_incomplete_requested_work().unwrap();

        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].id, task2.id);

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_incomplete_requested_work_blocked_on_question_and_dep() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task1 = store.create_task("Task with question and dep", "", Priority::High).unwrap();
        let task2 = store.create_task("Dependency task", "", Priority::Medium).unwrap();

        // Create dependency
        store.add_dependency(&task1.id, &task2.id).unwrap();

        // Request task1
        store.request_tasks(&[&task1.id]).unwrap();

        // Block task1 with an unanswered question
        let question = store.create_question("What should I do?").unwrap();
        store.link_task_to_question(&task1.id, &question.id).unwrap();

        // Get incomplete requested tasks
        // task1 has both a question AND a dependency, so it's NOT blocked on question only
        // task2 is a transitive dependency
        let incomplete = store.get_incomplete_requested_work().unwrap();

        assert_eq!(incomplete.len(), 2);
        let ids: Vec<_> = incomplete.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&task1.id.as_str()));
        assert!(ids.contains(&task2.id.as_str()));

        disable_deterministic_ids();
    }

    #[test]
    fn test_get_incomplete_requested_work_ordering_and_limit() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        // Create 7 tasks with different priorities
        let t_critical = store.create_task("Critical", "", Priority::Critical).unwrap();
        let t_high1 = store.create_task("High 1", "", Priority::High).unwrap();
        let t_high2 = store.create_task("High 2", "", Priority::High).unwrap();
        let t_medium1 = store.create_task("Medium 1", "", Priority::Medium).unwrap();
        let t_medium2 = store.create_task("Medium 2", "", Priority::Medium).unwrap();
        let t_low = store.create_task("Low", "", Priority::Low).unwrap();
        let t_backlog = store.create_task("Backlog", "", Priority::Backlog).unwrap();

        // Make High 2 have more dependents than High 1
        let dep1 = store.create_task("Dep of High 2 - a", "", Priority::Medium).unwrap();
        let dep2 = store.create_task("Dep of High 2 - b", "", Priority::Medium).unwrap();
        store.add_dependency(&dep1.id, &t_high2.id).unwrap();
        store.add_dependency(&dep2.id, &t_high2.id).unwrap();

        // Request all 7 tasks
        store
            .request_tasks(&[
                &t_critical.id,
                &t_high1.id,
                &t_high2.id,
                &t_medium1.id,
                &t_medium2.id,
                &t_low.id,
                &t_backlog.id,
            ])
            .unwrap();

        let incomplete = store.get_incomplete_requested_work().unwrap();

        // Should be limited to 5
        assert_eq!(incomplete.len(), 5);

        // First item should be critical priority
        assert_eq!(incomplete[0].id, t_critical.id);

        // Next two should be high priority, with high2 first (more dependents)
        assert_eq!(incomplete[1].id, t_high2.id);
        assert_eq!(incomplete[2].id, t_high1.id);

        // Then medium priority items
        assert!(incomplete[3].priority == Priority::Medium);
        assert!(incomplete[4].priority == Priority::Medium);

        // Low and backlog should be excluded (truncated)
        let ids: Vec<_> = incomplete.iter().map(|t| t.id.as_str()).collect();
        assert!(!ids.contains(&t_low.id.as_str()));
        assert!(!ids.contains(&t_backlog.id.as_str()));

        disable_deterministic_ids();
    }

    #[test]
    fn test_update_task_requested_field() {
        enable_deterministic_ids();
        let (_dir, store) = create_test_store();

        let task = store.create_task("Task", "", Priority::Medium).unwrap();
        assert!(!task.requested);

        // Update via TaskUpdate
        store
            .update_task(&task.id, TaskUpdate { requested: Some(true), ..Default::default() })
            .unwrap();

        let updated = store.get_task(&task.id).unwrap().unwrap();
        assert!(updated.requested);

        // Set back to false
        store
            .update_task(&task.id, TaskUpdate { requested: Some(false), ..Default::default() })
            .unwrap();

        let updated = store.get_task(&task.id).unwrap().unwrap();
        assert!(!updated.requested);

        disable_deterministic_ids();
    }
}
