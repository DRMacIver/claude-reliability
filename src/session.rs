//! Session file parsing for just-keep-working mode.
//!
//! Session state is now stored in a `SQLite` database at
//! `.claude/claude-reliability-working-memory.sqlite3`.
//!
//! The session notes file `.claude/jkw-session.local.md` remains a markdown
//! file for direct LLM editing.
//!
//! This module provides both:
//! - Path-based functions for backward compatibility (create `SqliteStore` internally)
//! - Store-based functions for testability (take a `&dyn StateStore` parameter)

use crate::error::Result;
use crate::storage::{markers, SqliteStore};
use crate::traits::StateStore;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Path for hook-managed session state (YAML).
/// Note: This is kept for backward compatibility and migration. New state
/// is stored in the `SQLite` database.
pub const SESSION_STATE_PATH: &str = ".claude/jkw-state.local.yaml";

/// Get or create a `SQLite` store for the given base directory.
///
/// This also performs migration from old file-based state if needed.
fn get_store(base_dir: &Path) -> Result<SqliteStore> {
    let store = SqliteStore::new(base_dir)?;
    // Migrate any existing file-based state
    store.migrate_from_files(base_dir)?;
    Ok(store)
}

/// Extract base directory from a path that might end with `SESSION_STATE_PATH`.
///
/// For backward compatibility, the path-based API expects paths like
/// `some/base/.claude/jkw-state.local.yaml`. This function extracts
/// `some/base` from such paths.
fn extract_base_dir(path: &Path) -> std::path::PathBuf {
    let path_str = path.to_string_lossy();
    path_str.strip_suffix(SESSION_STATE_PATH).map_or_else(
        || {
            // If not the expected path, use parent directory
            path.parent()
                .and_then(|p| p.parent())
                .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from)
        },
        |base| {
            if base.is_empty() {
                std::path::PathBuf::from(".")
            } else {
                std::path::PathBuf::from(base.trim_end_matches('/'))
            }
        },
    )
}

/// Path for LLM-editable session notes (markdown).
pub const SESSION_NOTES_PATH: &str = ".claude/jkw-session.local.md";

/// Staleness threshold - iterations without issue changes before stopping.
pub const STALENESS_THRESHOLD: u32 = 5;

/// Session configuration stored in the YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionConfig {
    /// Current iteration number.
    #[serde(default)]
    pub iteration: u32,
    /// Iteration when issues last changed.
    #[serde(default)]
    pub last_issue_change_iteration: u32,
    /// Snapshot of issue IDs from the last check.
    #[serde(default)]
    pub issue_snapshot: Vec<String>,
    /// Hash of git diff for staleness detection when beads is not available.
    /// This provides a fallback mechanism to detect progress via code changes.
    #[serde(default)]
    pub git_diff_hash: Option<String>,
}

impl SessionConfig {
    /// Get the issue snapshot as a `HashSet`.
    #[must_use]
    pub fn issue_snapshot_set(&self) -> HashSet<String> {
        self.issue_snapshot.iter().cloned().collect()
    }

    /// Calculate iterations since the last issue change.
    #[must_use]
    pub const fn iterations_since_change(&self) -> u32 {
        self.iteration.saturating_sub(self.last_issue_change_iteration)
    }

    /// Check if the session is stale (no progress for too long).
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        self.iterations_since_change() >= STALENESS_THRESHOLD
    }
}

/// Parse the session state from the database.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn parse_session_state(path: &Path) -> Result<Option<SessionConfig>> {
    let base_dir = extract_base_dir(path);
    get_store(&base_dir)?.get_session_state()
}

/// Parse the session state using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn parse_session_state_with_store(store: &dyn StateStore) -> Result<Option<SessionConfig>> {
    store.get_session_state()
}

/// Write the session state to the database.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn write_session_state(path: &Path, config: &SessionConfig) -> Result<()> {
    let base_dir = extract_base_dir(path);
    get_store(&base_dir)?.set_session_state(config)
}

/// Write the session state using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn write_session_state_with_store(
    store: &dyn StateStore,
    config: &SessionConfig,
) -> Result<()> {
    store.set_session_state(config)
}

/// Clear the session state from the database.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn cleanup_session_state(path: &Path) -> Result<()> {
    let base_dir = extract_base_dir(path);
    get_store(&base_dir)?.clear_session_state()
}

/// Clear the session state using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn cleanup_session_state_with_store(store: &dyn StateStore) -> Result<()> {
    store.clear_session_state()
}

/// Delete session state and notes file if they exist.
///
/// # Errors
///
/// Returns an error if operations fail.
pub fn cleanup_session_files(base_dir: &Path) -> Result<()> {
    // Clear database state
    get_store(base_dir)?.clear_session_state()?;

    // Also remove the notes file (still a markdown file on disk)
    let notes_path = base_dir.join(SESSION_NOTES_PATH);
    if notes_path.exists() {
        fs::remove_file(notes_path)?;
    }

    // Note: Legacy YAML file cleanup is handled by migrate_from_files() which
    // runs when get_store() is called above, so we don't need to check here.

    Ok(())
}

/// Delete session state and notes file using a provided store.
///
/// # Errors
///
/// Returns an error if operations fail.
pub fn cleanup_session_files_with_store(store: &dyn StateStore, base_dir: &Path) -> Result<()> {
    store.clear_session_state()?;

    let notes_path = base_dir.join(SESSION_NOTES_PATH);
    if notes_path.exists() {
        fs::remove_file(notes_path)?;
    }

    Ok(())
}

/// Default path for the problem mode marker file (legacy, for migration).
pub const PROBLEM_MODE_MARKER_PATH: &str = ".claude/problem-mode.local";

/// Path for JKW setup required marker file (legacy, for migration).
pub const JKW_SETUP_REQUIRED_MARKER_PATH: &str = ".claude/jkw-setup-required.local";

/// Check if JKW setup is required (marker exists in database).
///
/// When this returns true, Write/Edit operations should be blocked
/// until the JKW session file exists.
#[must_use]
pub fn is_jkw_setup_required(base_dir: &Path) -> bool {
    get_store(base_dir).map(|s| s.has_marker(markers::JKW_SETUP_REQUIRED)).unwrap_or(false)
}

/// Check if JKW setup is required using a provided store.
#[must_use]
pub fn is_jkw_setup_required_with_store(store: &dyn StateStore) -> bool {
    store.has_marker(markers::JKW_SETUP_REQUIRED)
}

/// Mark that JKW setup is required (session file doesn't exist yet).
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn set_jkw_setup_required(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.set_marker(markers::JKW_SETUP_REQUIRED)
}

/// Mark that JKW setup is required using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn set_jkw_setup_required_with_store(store: &dyn StateStore) -> Result<()> {
    store.set_marker(markers::JKW_SETUP_REQUIRED)
}

/// Clear the JKW setup required marker (session file now exists).
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn clear_jkw_setup_required(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.clear_marker(markers::JKW_SETUP_REQUIRED)
}

/// Clear the JKW setup required marker using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn clear_jkw_setup_required_with_store(store: &dyn StateStore) -> Result<()> {
    store.clear_marker(markers::JKW_SETUP_REQUIRED)
}

/// Check if the JKW session notes file exists.
#[must_use]
pub fn jkw_session_file_exists(base_dir: &Path) -> bool {
    base_dir.join(SESSION_NOTES_PATH).exists()
}

/// Check if problem mode is active (marker exists in database).
#[must_use]
pub fn is_problem_mode_active(base_dir: &Path) -> bool {
    get_store(base_dir).map(|s| s.has_marker(markers::PROBLEM_MODE)).unwrap_or(false)
}

/// Check if problem mode is active using a provided store.
#[must_use]
pub fn is_problem_mode_active_with_store(store: &dyn StateStore) -> bool {
    store.has_marker(markers::PROBLEM_MODE)
}

/// Enter problem mode by setting the marker in the database.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn enter_problem_mode(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.set_marker(markers::PROBLEM_MODE)
}

/// Enter problem mode using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn enter_problem_mode_with_store(store: &dyn StateStore) -> Result<()> {
    store.set_marker(markers::PROBLEM_MODE)
}

/// Exit problem mode by clearing the marker in the database.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn exit_problem_mode(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.clear_marker(markers::PROBLEM_MODE)
}

/// Exit problem mode using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn exit_problem_mode_with_store(store: &dyn StateStore) -> Result<()> {
    store.clear_marker(markers::PROBLEM_MODE)
}

/// Path for "needs validation" marker file (legacy, for migration).
pub const NEEDS_VALIDATION_MARKER_PATH: &str = ".claude/needs-validation.local";

/// Check if validation is needed (marker exists in database).
#[must_use]
pub fn needs_validation(base_dir: &Path) -> bool {
    get_store(base_dir).map(|s| s.has_marker(markers::NEEDS_VALIDATION)).unwrap_or(false)
}

/// Check if validation is needed using a provided store.
#[must_use]
pub fn needs_validation_with_store(store: &dyn StateStore) -> bool {
    store.has_marker(markers::NEEDS_VALIDATION)
}

/// Mark that validation is needed (modifying tool was used).
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn set_needs_validation(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.set_marker(markers::NEEDS_VALIDATION)
}

/// Mark that validation is needed using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn set_needs_validation_with_store(store: &dyn StateStore) -> Result<()> {
    store.set_marker(markers::NEEDS_VALIDATION)
}

/// Clear the needs validation marker (validation passed or user sent message).
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn clear_needs_validation(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.clear_marker(markers::NEEDS_VALIDATION)
}

/// Clear the needs validation marker using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn clear_needs_validation_with_store(store: &dyn StateStore) -> Result<()> {
    store.clear_marker(markers::NEEDS_VALIDATION)
}

/// Path for "must reflect" marker file (legacy, for migration).
pub const MUST_REFLECT_MARKER_PATH: &str = ".claude/must-reflect.local";

/// Check if the `must_reflect` marker exists.
#[must_use]
pub fn has_reflect_marker(base_dir: &Path) -> bool {
    get_store(base_dir).map(|s| s.has_marker(markers::MUST_REFLECT)).unwrap_or(false)
}

/// Check if the `must_reflect` marker exists using a provided store.
#[must_use]
pub fn has_reflect_marker_with_store(store: &dyn StateStore) -> bool {
    store.has_marker(markers::MUST_REFLECT)
}

/// Set the `must_reflect` marker.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn set_reflect_marker(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.set_marker(markers::MUST_REFLECT)
}

/// Set the `must_reflect` marker using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn set_reflect_marker_with_store(store: &dyn StateStore) -> Result<()> {
    store.set_marker(markers::MUST_REFLECT)
}

/// Clear the `must_reflect` marker.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn clear_reflect_marker(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.clear_marker(markers::MUST_REFLECT)
}

/// Clear the `must_reflect` marker using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn clear_reflect_marker_with_store(store: &dyn StateStore) -> Result<()> {
    store.clear_marker(markers::MUST_REFLECT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteStore;
    use crate::testing::MockStateStore;
    use tempfile::TempDir;

    #[test]
    fn test_parse_session_state_not_exists() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(SESSION_STATE_PATH);
        // No state set yet
        let result = parse_session_state(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_session_state_with_store() {
        let store = MockStateStore::new();
        let state = SessionConfig {
            iteration: 5,
            last_issue_change_iteration: 3,
            issue_snapshot: vec!["project-123".to_string(), "project-456".to_string()],
            git_diff_hash: None,
        };
        store.set_session_state(&state).unwrap();

        let config = parse_session_state_with_store(&store).unwrap().unwrap();
        assert_eq!(config.iteration, 5);
        assert_eq!(config.last_issue_change_iteration, 3);
        assert_eq!(config.issue_snapshot, vec!["project-123", "project-456"]);
    }

    #[test]
    fn test_parse_session_state_empty_snapshot() {
        let store = MockStateStore::new();
        let state = SessionConfig {
            iteration: 1,
            last_issue_change_iteration: 0,
            issue_snapshot: vec![],
            git_diff_hash: None,
        };
        store.set_session_state(&state).unwrap();

        let config = parse_session_state_with_store(&store).unwrap().unwrap();
        assert_eq!(config.iteration, 1);
        assert_eq!(config.last_issue_change_iteration, 0);
        assert!(config.issue_snapshot.is_empty());
    }

    #[test]
    fn test_write_session_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(SESSION_STATE_PATH);

        let config = SessionConfig {
            iteration: 3,
            last_issue_change_iteration: 2,
            issue_snapshot: vec!["issue-1".to_string(), "issue-2".to_string()],
            ..Default::default()
        };

        write_session_state(&path, &config).unwrap();

        // Read it back
        let parsed = parse_session_state(&path).unwrap().unwrap();
        assert_eq!(parsed.iteration, 3);
        assert_eq!(parsed.issue_snapshot, vec!["issue-1", "issue-2"]);
    }

    #[test]
    fn test_write_session_state_with_store() {
        let store = MockStateStore::new();

        let config = SessionConfig {
            iteration: 3,
            last_issue_change_iteration: 2,
            issue_snapshot: vec!["issue-1".to_string(), "issue-2".to_string()],
            ..Default::default()
        };

        write_session_state_with_store(&store, &config).unwrap();

        let parsed = parse_session_state_with_store(&store).unwrap().unwrap();
        assert_eq!(parsed.iteration, 3);
        assert_eq!(parsed.issue_snapshot, vec!["issue-1", "issue-2"]);
    }

    #[test]
    fn test_cleanup_session_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(SESSION_STATE_PATH);

        // Write some state first
        let config = SessionConfig { iteration: 1, ..Default::default() };
        write_session_state(&path, &config).unwrap();

        // Now cleanup
        cleanup_session_state(&path).unwrap();

        // State should be gone
        assert!(parse_session_state(&path).unwrap().is_none());
    }

    #[test]
    fn test_cleanup_session_state_with_store() {
        let store = MockStateStore::new();

        // Write state
        let config = SessionConfig { iteration: 1, ..Default::default() };
        store.set_session_state(&config).unwrap();
        assert!(store.get_session_state().unwrap().is_some());

        // Cleanup
        cleanup_session_state_with_store(&store).unwrap();
        assert!(store.get_session_state().unwrap().is_none());
    }

    #[test]
    fn test_cleanup_session_state_not_exists() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(SESSION_STATE_PATH);
        // Should not error when nothing to cleanup
        cleanup_session_state(&path).unwrap();
    }

    #[test]
    fn test_cleanup_session_files() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create session state and notes file
        let config = SessionConfig { iteration: 1, ..Default::default() };
        let state_path = base.join(SESSION_STATE_PATH);
        write_session_state(&state_path, &config).unwrap();

        let notes_path = base.join(SESSION_NOTES_PATH);
        fs::create_dir_all(notes_path.parent().unwrap()).unwrap();
        fs::write(&notes_path, "notes").unwrap();

        assert!(notes_path.exists());

        cleanup_session_files(base).unwrap();

        // Notes file should be removed
        assert!(!notes_path.exists());
        // State should be cleared from database
        assert!(parse_session_state(&state_path).unwrap().is_none());
    }

    #[test]
    fn test_cleanup_session_files_removes_legacy_yaml_via_migration() {
        // Legacy YAML file removal is now handled by migration in get_store()
        // This test verifies that cleanup_session_files works when a legacy
        // file exists - migration will clean it up when get_store() is called
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create legacy YAML file before any store access
        let state_path = base.join(SESSION_STATE_PATH);
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        fs::write(&state_path, "iteration: 1").unwrap();
        assert!(state_path.exists());

        // cleanup_session_files calls get_store() which triggers migration
        // Migration will delete the legacy file
        cleanup_session_files(base).unwrap();

        // Legacy file should be removed (by migration)
        assert!(!state_path.exists());
    }

    #[test]
    fn test_session_config_iterations_since_change() {
        let config =
            SessionConfig { iteration: 10, last_issue_change_iteration: 7, ..Default::default() };
        assert_eq!(config.iterations_since_change(), 3);
    }

    #[test]
    fn test_session_config_is_stale() {
        let stale =
            SessionConfig { iteration: 10, last_issue_change_iteration: 4, ..Default::default() };
        assert!(stale.is_stale()); // 6 iterations since change

        let not_stale =
            SessionConfig { iteration: 10, last_issue_change_iteration: 8, ..Default::default() };
        assert!(!not_stale.is_stale()); // 2 iterations since change
    }

    #[test]
    fn test_session_config_issue_snapshot_set() {
        let config = SessionConfig {
            iteration: 1,
            last_issue_change_iteration: 1,
            issue_snapshot: vec!["a".to_string(), "b".to_string(), "a".to_string()],
            ..Default::default()
        };
        let set = config.issue_snapshot_set();
        assert_eq!(set.len(), 2);
        assert!(set.contains("a"));
        assert!(set.contains("b"));
    }

    #[test]
    fn test_staleness_threshold() {
        assert_eq!(STALENESS_THRESHOLD, 5);
    }

    #[test]
    fn test_write_session_state_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        // Use a path under .claude which is where the database will be created
        let path = dir.path().join(SESSION_STATE_PATH);

        let config =
            SessionConfig { iteration: 1, last_issue_change_iteration: 1, ..Default::default() };

        write_session_state(&path, &config).unwrap();

        // .claude directory should be created for the database
        assert!(dir.path().join(".claude").exists());
    }

    #[test]
    fn test_problem_mode_not_active_by_default() {
        let dir = TempDir::new().unwrap();
        assert!(!is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_enter_problem_mode() {
        let dir = TempDir::new().unwrap();

        // Enter problem mode
        enter_problem_mode(dir.path()).unwrap();

        // Verify mode is active
        assert!(is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_enter_problem_mode_with_store() {
        let store = MockStateStore::new();

        assert!(!is_problem_mode_active_with_store(&store));

        enter_problem_mode_with_store(&store).unwrap();

        assert!(is_problem_mode_active_with_store(&store));
    }

    #[test]
    fn test_exit_problem_mode() {
        let dir = TempDir::new().unwrap();

        // Enter and then exit problem mode
        enter_problem_mode(dir.path()).unwrap();
        assert!(is_problem_mode_active(dir.path()));

        exit_problem_mode(dir.path()).unwrap();
        assert!(!is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_exit_problem_mode_with_store() {
        let store = MockStateStore::new();

        enter_problem_mode_with_store(&store).unwrap();
        assert!(is_problem_mode_active_with_store(&store));

        exit_problem_mode_with_store(&store).unwrap();
        assert!(!is_problem_mode_active_with_store(&store));
    }

    #[test]
    fn test_exit_problem_mode_when_not_active() {
        let dir = TempDir::new().unwrap();

        // Should not error when exiting without entering
        exit_problem_mode(dir.path()).unwrap();
        assert!(!is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_enter_problem_mode_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        // Base dir is the temp dir, .claude subdirectory doesn't exist yet

        enter_problem_mode(dir.path()).unwrap();

        assert!(dir.path().join(".claude").exists());
        assert!(is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_jkw_setup_not_required_by_default() {
        let dir = TempDir::new().unwrap();
        assert!(!is_jkw_setup_required(dir.path()));
    }

    #[test]
    fn test_set_jkw_setup_required() {
        let dir = TempDir::new().unwrap();

        set_jkw_setup_required(dir.path()).unwrap();

        assert!(is_jkw_setup_required(dir.path()));
    }

    #[test]
    fn test_set_jkw_setup_required_with_store() {
        let store = MockStateStore::new();

        assert!(!is_jkw_setup_required_with_store(&store));

        set_jkw_setup_required_with_store(&store).unwrap();

        assert!(is_jkw_setup_required_with_store(&store));
    }

    #[test]
    fn test_clear_jkw_setup_required() {
        let dir = TempDir::new().unwrap();

        set_jkw_setup_required(dir.path()).unwrap();
        assert!(is_jkw_setup_required(dir.path()));

        clear_jkw_setup_required(dir.path()).unwrap();
        assert!(!is_jkw_setup_required(dir.path()));
    }

    #[test]
    fn test_clear_jkw_setup_required_with_store() {
        let store = MockStateStore::new();

        set_jkw_setup_required_with_store(&store).unwrap();
        assert!(is_jkw_setup_required_with_store(&store));

        clear_jkw_setup_required_with_store(&store).unwrap();
        assert!(!is_jkw_setup_required_with_store(&store));
    }

    #[test]
    fn test_clear_jkw_setup_required_when_not_set() {
        let dir = TempDir::new().unwrap();

        // Should not error when clearing without setting
        clear_jkw_setup_required(dir.path()).unwrap();
        assert!(!is_jkw_setup_required(dir.path()));
    }

    #[test]
    fn test_set_jkw_setup_required_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        // .claude subdirectory doesn't exist yet

        set_jkw_setup_required(dir.path()).unwrap();

        assert!(dir.path().join(".claude").exists());
        assert!(is_jkw_setup_required(dir.path()));
    }

    #[test]
    fn test_jkw_session_file_exists() {
        let dir = TempDir::new().unwrap();

        // Initially doesn't exist
        assert!(!jkw_session_file_exists(dir.path()));

        // Create the file
        let session_path = dir.path().join(SESSION_NOTES_PATH);
        fs::create_dir_all(session_path.parent().unwrap()).unwrap();
        fs::write(&session_path, "# Session notes").unwrap();

        assert!(jkw_session_file_exists(dir.path()));
    }

    #[test]
    fn test_needs_validation_not_set_by_default() {
        let dir = TempDir::new().unwrap();
        assert!(!needs_validation(dir.path()));
    }

    #[test]
    fn test_set_needs_validation() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        set_needs_validation(base).unwrap();
        assert!(needs_validation(base));
    }

    #[test]
    fn test_set_needs_validation_with_store() {
        let store = MockStateStore::new();

        assert!(!needs_validation_with_store(&store));

        set_needs_validation_with_store(&store).unwrap();

        assert!(needs_validation_with_store(&store));
    }

    #[test]
    fn test_clear_needs_validation() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        set_needs_validation(base).unwrap();
        assert!(needs_validation(base));

        clear_needs_validation(base).unwrap();
        assert!(!needs_validation(base));
    }

    #[test]
    fn test_clear_needs_validation_with_store() {
        let store = MockStateStore::new();

        set_needs_validation_with_store(&store).unwrap();
        assert!(needs_validation_with_store(&store));

        clear_needs_validation_with_store(&store).unwrap();
        assert!(!needs_validation_with_store(&store));
    }

    #[test]
    fn test_clear_needs_validation_when_not_set() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Should not error when clearing without setting
        clear_needs_validation(base).unwrap();
        assert!(!needs_validation(base));
    }

    #[test]
    fn test_has_reflect_marker_not_set_by_default() {
        let dir = TempDir::new().unwrap();
        assert!(!has_reflect_marker(dir.path()));
    }

    #[test]
    fn test_set_reflect_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        set_reflect_marker(base).unwrap();
        assert!(has_reflect_marker(base));
    }

    #[test]
    fn test_set_reflect_marker_with_store() {
        let store = MockStateStore::new();

        assert!(!has_reflect_marker_with_store(&store));

        set_reflect_marker_with_store(&store).unwrap();

        assert!(has_reflect_marker_with_store(&store));
    }

    #[test]
    fn test_clear_reflect_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        set_reflect_marker(base).unwrap();
        assert!(has_reflect_marker(base));

        clear_reflect_marker(base).unwrap();
        assert!(!has_reflect_marker(base));
    }

    #[test]
    fn test_clear_reflect_marker_with_store() {
        let store = MockStateStore::new();

        set_reflect_marker_with_store(&store).unwrap();
        assert!(has_reflect_marker_with_store(&store));

        clear_reflect_marker_with_store(&store).unwrap();
        assert!(!has_reflect_marker_with_store(&store));
    }

    #[test]
    fn test_clear_reflect_marker_when_not_set() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Should not error when clearing without setting
        clear_reflect_marker(base).unwrap();
        assert!(!has_reflect_marker(base));
    }

    #[test]
    fn test_set_reflect_marker_creates_parent_directory() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        // .claude subdirectory doesn't exist yet

        set_reflect_marker(base).unwrap();

        assert!(base.join(".claude").exists());
        assert!(has_reflect_marker(base));
    }

    #[test]
    fn test_migration_from_yaml_file() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create legacy YAML file
        let yaml_path = base.join(".claude/jkw-state.local.yaml");
        fs::create_dir_all(yaml_path.parent().unwrap()).unwrap();
        fs::write(
            &yaml_path,
            r"iteration: 5
last_issue_change_iteration: 3
issue_snapshot:
  - project-123
",
        )
        .unwrap();

        // Get store (triggers migration)
        let store = get_store(base).unwrap();

        // State should be migrated
        let state = store.get_session_state().unwrap().unwrap();
        assert_eq!(state.iteration, 5);
        assert_eq!(state.last_issue_change_iteration, 3);

        // Legacy file should be removed
        assert!(!yaml_path.exists());
    }

    #[test]
    fn test_migration_from_marker_files() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create legacy marker files
        let claude_dir = base.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(base.join(".claude/problem-mode.local"), "").unwrap();
        fs::write(base.join(".claude/needs-validation.local"), "").unwrap();

        // Get store (triggers migration)
        let store = get_store(base).unwrap();

        // Markers should be migrated
        assert!(store.has_marker(markers::PROBLEM_MODE));
        assert!(store.has_marker(markers::NEEDS_VALIDATION));

        // Legacy files should be removed
        assert!(!base.join(".claude/problem-mode.local").exists());
        assert!(!base.join(".claude/needs-validation.local").exists());
    }

    #[test]
    fn test_cleanup_session_files_with_store() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let store = SqliteStore::new(base).unwrap();

        // Create session state and notes
        let config = SessionConfig { iteration: 1, ..Default::default() };
        store.set_session_state(&config).unwrap();

        let notes_path = base.join(SESSION_NOTES_PATH);
        fs::create_dir_all(notes_path.parent().unwrap()).unwrap();
        fs::write(&notes_path, "notes").unwrap();

        cleanup_session_files_with_store(&store, base).unwrap();

        assert!(store.get_session_state().unwrap().is_none());
        assert!(!notes_path.exists());
    }

    #[test]
    #[serial_test::serial]
    fn test_parse_session_state_exact_path() {
        // Test when the path is exactly SESSION_STATE_PATH (empty base)
        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // Create the .claude directory and state
        fs::create_dir_all(".claude").unwrap();
        let store = get_store(Path::new(".")).unwrap();
        let config = SessionConfig { iteration: 7, ..Default::default() };
        store.set_session_state(&config).unwrap();

        // Use just the SESSION_STATE_PATH (no leading directory)
        let path = Path::new(SESSION_STATE_PATH);
        let result = parse_session_state(path).unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap().iteration, 7);

        std::env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    fn test_parse_session_state_non_standard_path() {
        // Test fallback path when path doesn't match SESSION_STATE_PATH
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create state in the expected location
        let store = get_store(base).unwrap();
        let config = SessionConfig { iteration: 11, ..Default::default() };
        store.set_session_state(&config).unwrap();

        // Use a non-standard path (within .claude but different file)
        let non_standard_path = base.join(".claude/some-other-file.yaml");
        let result = parse_session_state(&non_standard_path).unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap().iteration, 11);
    }

    #[test]
    #[serial_test::serial]
    fn test_write_session_state_exact_path() {
        // Test when the path is exactly SESSION_STATE_PATH (empty base)
        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let config = SessionConfig { iteration: 13, ..Default::default() };
        let path = Path::new(SESSION_STATE_PATH);
        write_session_state(path, &config).unwrap();

        // Read back and verify
        let result = parse_session_state(path).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().iteration, 13);

        std::env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    fn test_write_session_state_non_standard_path() {
        // Test fallback path when path doesn't match SESSION_STATE_PATH
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let config = SessionConfig { iteration: 17, ..Default::default() };
        // Use a non-standard path (within .claude but different file)
        let non_standard_path = base.join(".claude/custom-state.yaml");
        write_session_state(&non_standard_path, &config).unwrap();

        // Read back using the non-standard path
        let result = parse_session_state(&non_standard_path).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().iteration, 17);
    }

    #[test]
    #[serial_test::serial]
    fn test_cleanup_session_state_exact_path() {
        // Test when the path is exactly SESSION_STATE_PATH (empty base)
        let dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let config = SessionConfig { iteration: 19, ..Default::default() };
        let path = Path::new(SESSION_STATE_PATH);
        write_session_state(path, &config).unwrap();
        assert!(parse_session_state(path).unwrap().is_some());

        cleanup_session_state(path).unwrap();
        assert!(parse_session_state(path).unwrap().is_none());

        std::env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    fn test_cleanup_session_state_non_standard_path() {
        // Test fallback path when path doesn't match SESSION_STATE_PATH
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create state
        let config = SessionConfig { iteration: 23, ..Default::default() };
        let non_standard_path = base.join(".claude/other-state.yaml");
        write_session_state(&non_standard_path, &config).unwrap();
        assert!(parse_session_state(&non_standard_path).unwrap().is_some());

        // Cleanup using non-standard path
        cleanup_session_state(&non_standard_path).unwrap();
        assert!(parse_session_state(&non_standard_path).unwrap().is_none());
    }
}
