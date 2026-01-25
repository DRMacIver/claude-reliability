//! `SQLite`-based state storage for marker data.
//!
//! This module provides persistent storage for:
//! - Boolean markers (problem mode, validation needed, etc.)
//!
//! All state is stored in a single `SQLite` database at
//! `~/.claude-reliability/projects/<sanitized-path>/working-memory.sqlite3`.

use crate::error::Result;
use crate::paths;
use crate::traits::StateStore;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// Marker name constants for consistent usage across the codebase.
pub mod markers {
    /// Problem mode is active - tool use blocked until stop.
    pub const PROBLEM_MODE: &str = "problem_mode";
    /// Validation is needed - modifying tool was used.
    pub const NEEDS_VALIDATION: &str = "needs_validation";
    /// Agent should reflect on work before stopping.
    pub const MUST_REFLECT: &str = "must_reflect";
    /// Emergency stop requested by agent.
    pub const EMERGENCY_STOP: &str = "emergency_stop";
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
    /// # Errors
    ///
    /// Returns an error if the database cannot be initialized.
    pub fn new(project_dir: &Path) -> Result<Self> {
        let db_path = paths::project_db_path(project_dir);
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
    /// This checks for old marker files, migrates them to the `SQLite` database,
    /// and removes the old files.
    ///
    /// # Errors
    ///
    /// Returns an error if migration fails.
    pub fn migrate_from_files(&self, base_dir: &Path) -> Result<()> {
        // Remove legacy JKW files if they exist
        let legacy_files = [".claude/jkw-state.local.yaml", ".claude/jkw-session.local.md"];
        for file_path in legacy_files {
            let full_path = base_dir.join(file_path);
            if full_path.exists() {
                let _ = std::fs::remove_file(&full_path);
            }
        }

        // Migrate marker files
        let marker_migrations = [
            (".claude/problem-mode.local", markers::PROBLEM_MODE),
            (".claude/needs-validation.local", markers::NEEDS_VALIDATION),
            (".claude/must-reflect.local", markers::MUST_REFLECT),
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
        // Database should be in <project>/.claude-reliability/
        let path_str = store.db_path().to_string_lossy();
        assert!(path_str.contains(".claude-reliability"));
        assert!(path_str.ends_with(paths::DATABASE_FILENAME));
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
        assert!(!markers::NEEDS_VALIDATION.is_empty());
        assert!(!markers::MUST_REFLECT.is_empty());
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
    fn test_migrate_removes_legacy_jkw_files() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create old YAML state file
        let claude_dir = base.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(base.join(".claude/jkw-state.local.yaml"), "iteration: 5").unwrap();

        // Create store and migrate
        let store = SqliteStore::new(base).unwrap();
        store.migrate_from_files(base).unwrap();

        // Old file should be removed (not migrated since JKW is removed)
        assert!(!base.join(".claude/jkw-state.local.yaml").exists());
    }

    #[test]
    fn test_migrate_no_files() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // No old files exist
        let store = SqliteStore::new(base).unwrap();
        store.migrate_from_files(base).unwrap();

        // Should work without error (no markers set)
        assert!(!store.has_marker(markers::PROBLEM_MODE));
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
