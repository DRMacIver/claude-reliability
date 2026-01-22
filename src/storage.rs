//! `SQLite`-based state storage for session and marker data.
//!
//! This module provides persistent storage for:
//! - Session state (JKW mode iteration, staleness tracking)
//! - Issue snapshots (for detecting issue changes)
//! - Boolean markers (problem mode, validation needed, etc.)
//!
//! All state is stored in a single `SQLite` database at
//! `~/.claude-reliability/projects/<sanitized-path>/working-memory.sqlite3`.

use crate::error::Result;
use crate::paths;
use crate::session::SessionConfig;
use crate::traits::StateStore;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Marker name constants for consistent usage across the codebase.
pub mod markers {
    /// Problem mode is active - tool use blocked until stop.
    pub const PROBLEM_MODE: &str = "problem_mode";
    /// JKW setup is required - Write/Edit blocked until session file exists.
    pub const JKW_SETUP_REQUIRED: &str = "jkw_setup_required";
    /// Validation is needed - modifying tool was used.
    pub const NEEDS_VALIDATION: &str = "needs_validation";
    /// Agent should reflect on work before stopping.
    pub const MUST_REFLECT: &str = "must_reflect";
    /// Beads warning has been given this session.
    pub const BEADS_WARNING: &str = "beads_warning";
    /// Questions have been shown for reflection.
    pub const QUESTIONS_SHOWN: &str = "questions_shown";
}

/// SQLite-based state store.
///
/// Each operation opens a new connection to the database file.
/// This avoids thread safety issues and is acceptable for the
/// low frequency of state operations.
#[derive(Debug, Clone)]
pub struct SqliteStore {
    /// Path to the database file.
    db_path: PathBuf,
}

impl SqliteStore {
    /// Create a new `SQLite` store for the given project directory.
    ///
    /// The database file will be created at
    /// `~/.claude-reliability/projects/<hash>/working-memory.sqlite3`.
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined or
    /// the database cannot be initialized.
    pub fn new(project_dir: &Path) -> Result<Self> {
        let db_path = paths::project_db_path(project_dir)
            .ok_or_else(|| crate::error::Error::Config("Cannot determine home directory".into()))?;
        let store = Self { db_path };
        store.init_schema()?;
        Ok(store)
    }

    /// Create a new `SQLite` store with a specific database path.
    ///
    /// This is primarily for testing purposes.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be initialized.
    pub fn with_path(db_path: PathBuf) -> Result<Self> {
        let store = Self { db_path };
        store.init_schema()?;
        Ok(store)
    }

    /// Get the database path.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Open a connection to the database.
    fn open(&self) -> Result<Connection> {
        // Ensure parent directory exists
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&self.db_path)?;
        // Enable foreign keys and WAL mode for better concurrency
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Ok(conn)
    }

    /// Initialize the database schema.
    fn init_schema(&self) -> Result<()> {
        let conn = self.open()?;

        conn.execute_batch(
            r"
            -- Session state for JKW mode (singleton row)
            CREATE TABLE IF NOT EXISTS session_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                iteration INTEGER NOT NULL DEFAULT 0,
                last_issue_change_iteration INTEGER NOT NULL DEFAULT 0,
                git_diff_hash TEXT
            );

            -- Issue snapshot (part of session state)
            CREATE TABLE IF NOT EXISTS issue_snapshot (
                issue_id TEXT PRIMARY KEY
            );

            -- Boolean markers (presence = true)
            CREATE TABLE IF NOT EXISTS markers (
                name TEXT PRIMARY KEY
            );
            ",
        )?;

        Ok(())
    }

    /// Migrate state from old file-based storage.
    ///
    /// This checks for old marker files and YAML state, migrates them
    /// to the `SQLite` database, and removes the old files.
    ///
    /// # Errors
    ///
    /// Returns an error if migration fails.
    pub fn migrate_from_files(&self, base_dir: &Path) -> Result<()> {
        // Migrate session state from YAML
        let yaml_path = base_dir.join(".claude/jkw-state.local.yaml");
        if yaml_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&yaml_path) {
                if let Ok(config) = serde_yaml::from_str::<SessionConfig>(&content) {
                    self.set_session_state(&config)?;
                    // Remove old file after successful migration
                    let _ = std::fs::remove_file(&yaml_path);
                }
            }
        }

        // Migrate marker files
        let marker_migrations = [
            (".claude/problem-mode.local", markers::PROBLEM_MODE),
            (".claude/jkw-setup-required.local", markers::JKW_SETUP_REQUIRED),
            (".claude/needs-validation.local", markers::NEEDS_VALIDATION),
            (".claude/must-reflect.local", markers::MUST_REFLECT),
            (".claude/beads-warning-given.local", markers::BEADS_WARNING),
        ];

        for (file_path, marker_name) in marker_migrations {
            let full_path = base_dir.join(file_path);
            if full_path.exists() {
                self.set_marker(marker_name)?;
                // Remove old file after successful migration
                let _ = std::fs::remove_file(&full_path);
            }
        }

        Ok(())
    }
}

impl StateStore for SqliteStore {
    fn get_session_state(&self) -> Result<Option<SessionConfig>> {
        let conn = self.open()?;

        let state: Option<(i64, i64, Option<String>)> = conn
            .query_row(
                "SELECT iteration, last_issue_change_iteration, git_diff_hash
                 FROM session_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        match state {
            Some((iteration, last_issue_change_iteration, git_diff_hash)) => {
                // Get issue snapshot
                let mut stmt = conn.prepare("SELECT issue_id FROM issue_snapshot")?;
                let issue_snapshot: Vec<String> =
                    stmt.query_map([], |row| row.get(0))?.flatten().collect();

                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(Some(SessionConfig {
                    iteration: iteration as u32,
                    last_issue_change_iteration: last_issue_change_iteration as u32,
                    issue_snapshot,
                    git_diff_hash,
                }))
            }
            None => Ok(None),
        }
    }

    fn set_session_state(&self, state: &SessionConfig) -> Result<()> {
        let conn = self.open()?;

        // Upsert session state
        conn.execute(
            "INSERT INTO session_state (id, iteration, last_issue_change_iteration, git_diff_hash)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
                 iteration = excluded.iteration,
                 last_issue_change_iteration = excluded.last_issue_change_iteration,
                 git_diff_hash = excluded.git_diff_hash",
            params![
                i64::from(state.iteration),
                i64::from(state.last_issue_change_iteration),
                &state.git_diff_hash,
            ],
        )?;

        // Update issue snapshot - clear and repopulate
        conn.execute("DELETE FROM issue_snapshot", [])?;
        for issue_id in &state.issue_snapshot {
            conn.execute("INSERT INTO issue_snapshot (issue_id) VALUES (?1)", params![issue_id])?;
        }

        Ok(())
    }

    fn clear_session_state(&self) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM session_state", [])?;
        conn.execute("DELETE FROM issue_snapshot", [])?;
        Ok(())
    }

    fn get_issue_snapshot(&self) -> Result<HashSet<String>> {
        let conn = self.open()?;
        let mut stmt = conn.prepare("SELECT issue_id FROM issue_snapshot")?;
        let issues: HashSet<String> = stmt.query_map([], |row| row.get(0))?.flatten().collect();
        Ok(issues)
    }

    fn set_issue_snapshot(&self, issues: &[String]) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM issue_snapshot", [])?;
        for issue_id in issues {
            conn.execute("INSERT INTO issue_snapshot (issue_id) VALUES (?1)", params![issue_id])?;
        }
        Ok(())
    }

    fn has_marker(&self, name: &str) -> bool {
        let Ok(conn) = self.open() else {
            return false;
        };
        conn.query_row("SELECT 1 FROM markers WHERE name = ?1", params![name], |_| Ok(()))
            .optional()
            .is_ok_and(|opt| opt.is_some())
    }

    fn set_marker(&self, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute("INSERT OR IGNORE INTO markers (name) VALUES (?1)", params![name])?;
        Ok(())
    }

    fn clear_marker(&self, name: &str) -> Result<()> {
        let conn = self.open()?;
        conn.execute("DELETE FROM markers WHERE name = ?1", params![name])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, SqliteStore) {
        let dir = TempDir::new().unwrap();
        let store = SqliteStore::new(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn test_new_store_creates_database() {
        let (_dir, store) = create_test_store();
        assert!(store.db_path().exists());
        // Database should be in ~/.claude-reliability/projects/<sanitized-path>/
        let path_str = store.db_path().to_string_lossy();
        assert!(path_str.contains(".claude-reliability"));
        assert!(path_str.contains("projects"));
        assert!(path_str.ends_with(paths::DATABASE_FILENAME));
    }

    #[test]
    fn test_session_state_roundtrip() {
        let (_dir, store) = create_test_store();

        // Initially no state
        assert!(store.get_session_state().unwrap().is_none());

        // Set state
        let state = SessionConfig {
            iteration: 5,
            last_issue_change_iteration: 3,
            issue_snapshot: vec!["issue-1".to_string(), "issue-2".to_string()],
            git_diff_hash: Some("abc123".to_string()),
        };
        store.set_session_state(&state).unwrap();

        // Read back
        let read_state = store.get_session_state().unwrap().unwrap();
        assert_eq!(read_state.iteration, 5);
        assert_eq!(read_state.last_issue_change_iteration, 3);
        assert_eq!(read_state.issue_snapshot, vec!["issue-1", "issue-2"]);
        assert_eq!(read_state.git_diff_hash, Some("abc123".to_string()));
    }

    #[test]
    fn test_session_state_update() {
        let (_dir, store) = create_test_store();

        // Set initial state
        let state1 = SessionConfig {
            iteration: 1,
            last_issue_change_iteration: 1,
            issue_snapshot: vec!["issue-1".to_string()],
            git_diff_hash: None,
        };
        store.set_session_state(&state1).unwrap();

        // Update state
        let state2 = SessionConfig {
            iteration: 2,
            last_issue_change_iteration: 2,
            issue_snapshot: vec!["issue-2".to_string(), "issue-3".to_string()],
            git_diff_hash: Some("def456".to_string()),
        };
        store.set_session_state(&state2).unwrap();

        // Read back - should have updated values
        let read_state = store.get_session_state().unwrap().unwrap();
        assert_eq!(read_state.iteration, 2);
        assert_eq!(read_state.issue_snapshot, vec!["issue-2", "issue-3"]);
        assert_eq!(read_state.git_diff_hash, Some("def456".to_string()));
    }

    #[test]
    fn test_clear_session_state() {
        let (_dir, store) = create_test_store();

        // Set state
        let state = SessionConfig {
            iteration: 5,
            last_issue_change_iteration: 3,
            issue_snapshot: vec!["issue-1".to_string()],
            git_diff_hash: None,
        };
        store.set_session_state(&state).unwrap();
        assert!(store.get_session_state().unwrap().is_some());

        // Clear
        store.clear_session_state().unwrap();
        assert!(store.get_session_state().unwrap().is_none());
    }

    #[test]
    fn test_issue_snapshot_separate_operations() {
        let (_dir, store) = create_test_store();

        // Initially empty
        assert!(store.get_issue_snapshot().unwrap().is_empty());

        // Set snapshot
        store.set_issue_snapshot(&["a".to_string(), "b".to_string()]).unwrap();

        let snapshot = store.get_issue_snapshot().unwrap();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.contains("a"));
        assert!(snapshot.contains("b"));

        // Update snapshot
        store.set_issue_snapshot(&["c".to_string()]).unwrap();

        let snapshot = store.get_issue_snapshot().unwrap();
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot.contains("c"));
    }

    #[test]
    fn test_markers() {
        let (_dir, store) = create_test_store();

        // Initially no markers
        assert!(!store.has_marker("test_marker"));

        // Set marker
        store.set_marker("test_marker").unwrap();
        assert!(store.has_marker("test_marker"));

        // Set again (idempotent)
        store.set_marker("test_marker").unwrap();
        assert!(store.has_marker("test_marker"));

        // Clear marker
        store.clear_marker("test_marker").unwrap();
        assert!(!store.has_marker("test_marker"));

        // Clear again (idempotent)
        store.clear_marker("test_marker").unwrap();
        assert!(!store.has_marker("test_marker"));
    }

    #[test]
    fn test_multiple_markers() {
        let (_dir, store) = create_test_store();

        store.set_marker("marker1").unwrap();
        store.set_marker("marker2").unwrap();

        assert!(store.has_marker("marker1"));
        assert!(store.has_marker("marker2"));
        assert!(!store.has_marker("marker3"));

        store.clear_marker("marker1").unwrap();
        assert!(!store.has_marker("marker1"));
        assert!(store.has_marker("marker2"));
    }

    #[test]
    fn test_marker_constants() {
        // Verify marker constants are defined
        assert!(!markers::PROBLEM_MODE.is_empty());
        assert!(!markers::JKW_SETUP_REQUIRED.is_empty());
        assert!(!markers::NEEDS_VALIDATION.is_empty());
        assert!(!markers::MUST_REFLECT.is_empty());
        assert!(!markers::BEADS_WARNING.is_empty());
    }

    #[test]
    fn test_migrate_from_files_markers() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create old marker files
        let claude_dir = base.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(base.join(".claude/problem-mode.local"), "").unwrap();
        std::fs::write(base.join(".claude/needs-validation.local"), "").unwrap();

        // Create store and migrate
        let store = SqliteStore::new(base).unwrap();
        store.migrate_from_files(base).unwrap();

        // Markers should be set in database
        assert!(store.has_marker(markers::PROBLEM_MODE));
        assert!(store.has_marker(markers::NEEDS_VALIDATION));
        assert!(!store.has_marker(markers::MUST_REFLECT));

        // Old files should be removed
        assert!(!base.join(".claude/problem-mode.local").exists());
        assert!(!base.join(".claude/needs-validation.local").exists());
    }

    #[test]
    fn test_migrate_from_files_session_state() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create old YAML state file
        let claude_dir = base.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            base.join(".claude/jkw-state.local.yaml"),
            r"iteration: 5
last_issue_change_iteration: 3
issue_snapshot:
  - issue-1
  - issue-2
git_diff_hash: abc123
",
        )
        .unwrap();

        // Create store and migrate
        let store = SqliteStore::new(base).unwrap();
        store.migrate_from_files(base).unwrap();

        // Session state should be migrated
        let state = store.get_session_state().unwrap().unwrap();
        assert_eq!(state.iteration, 5);
        assert_eq!(state.last_issue_change_iteration, 3);
        assert_eq!(state.issue_snapshot, vec!["issue-1", "issue-2"]);
        assert_eq!(state.git_diff_hash, Some("abc123".to_string()));

        // Old file should be removed
        assert!(!base.join(".claude/jkw-state.local.yaml").exists());
    }

    #[test]
    fn test_migrate_no_files() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // No old files exist
        let store = SqliteStore::new(base).unwrap();
        store.migrate_from_files(base).unwrap();

        // Should work without error
        assert!(store.get_session_state().unwrap().is_none());
    }

    #[test]
    fn test_with_path() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("custom.db");

        let store = SqliteStore::with_path(db_path.clone()).unwrap();
        assert_eq!(store.db_path(), db_path);

        // Should work
        store.set_marker("test").unwrap();
        assert!(store.has_marker("test"));
    }

    #[test]
    fn test_session_state_without_git_diff_hash() {
        let (_dir, store) = create_test_store();

        let state = SessionConfig {
            iteration: 1,
            last_issue_change_iteration: 1,
            issue_snapshot: vec![],
            git_diff_hash: None,
        };
        store.set_session_state(&state).unwrap();

        let read_state = store.get_session_state().unwrap().unwrap();
        assert!(read_state.git_diff_hash.is_none());
    }

    #[test]
    fn test_session_state_empty_snapshot() {
        let (_dir, store) = create_test_store();

        let state = SessionConfig {
            iteration: 1,
            last_issue_change_iteration: 1,
            issue_snapshot: vec![],
            git_diff_hash: None,
        };
        store.set_session_state(&state).unwrap();

        let read_state = store.get_session_state().unwrap().unwrap();
        assert!(read_state.issue_snapshot.is_empty());
    }

    #[test]
    fn test_clear_issue_snapshot_via_empty_set() {
        let (_dir, store) = create_test_store();

        // Set some issues
        store.set_issue_snapshot(&["a".to_string(), "b".to_string()]).unwrap();
        assert_eq!(store.get_issue_snapshot().unwrap().len(), 2);

        // Clear by setting empty
        store.set_issue_snapshot(&[]).unwrap();
        assert!(store.get_issue_snapshot().unwrap().is_empty());
    }

    #[test]
    fn test_has_marker_returns_false_when_open_fails() {
        // Create a store pointing to an invalid path that can't be opened
        // Using a path that's a directory, not a file, will cause open to fail
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join(".claude/test.db");

        // Create the .claude directory
        std::fs::create_dir_all(&db_path).unwrap(); // Make db_path a directory!

        // Create store with path that is actually a directory (not a file)
        let store = SqliteStore { db_path };

        // has_marker should return false when open fails (because path is a directory)
        assert!(!store.has_marker("test_marker"));
    }
}
