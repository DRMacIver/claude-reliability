//! Session state management for reliability mode.
//!
//! Session state is stored in a `SQLite` database at
//! `<project_dir>/.claude-reliability/working-memory.sqlite3`.
//!
//! This module provides both:
//! - Path-based functions for backward compatibility (create `SqliteStore` internally)
//! - Store-based functions for testability (take a `&dyn StateStore` parameter)

use crate::error::Result;
use crate::storage::{markers, SqliteStore};
use crate::traits::StateStore;
use std::path::Path;

/// Get or create a `SQLite` store for the given base directory.
///
/// This also performs migration from old file-based state if needed.
fn get_store(base_dir: &Path) -> Result<SqliteStore> {
    let store = SqliteStore::new(base_dir)?;
    // Migrate any existing file-based state
    store.migrate_from_files(base_dir)?;
    Ok(store)
}

/// Default path for the problem mode marker file (legacy, for migration).
pub const PROBLEM_MODE_MARKER_PATH: &str = ".claude/problem-mode.local";

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

/// Check if emergency stop is active (marker exists in database).
#[must_use]
pub fn is_emergency_stop_active(base_dir: &Path) -> bool {
    get_store(base_dir).map(|s| s.has_marker(markers::EMERGENCY_STOP)).unwrap_or(false)
}

/// Check if emergency stop is active using a provided store.
#[must_use]
pub fn is_emergency_stop_active_with_store(store: &dyn StateStore) -> bool {
    store.has_marker(markers::EMERGENCY_STOP)
}

/// Set the emergency stop marker.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn set_emergency_stop(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.set_marker(markers::EMERGENCY_STOP)
}

/// Set the emergency stop marker using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn set_emergency_stop_with_store(store: &dyn StateStore) -> Result<()> {
    store.set_marker(markers::EMERGENCY_STOP)
}

/// Clear the emergency stop marker.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn clear_emergency_stop(base_dir: &Path) -> Result<()> {
    get_store(base_dir)?.clear_marker(markers::EMERGENCY_STOP)
}

/// Clear the emergency stop marker using a provided store.
///
/// # Errors
///
/// Returns an error if the database operation fails.
pub fn clear_emergency_stop_with_store(store: &dyn StateStore) -> Result<()> {
    store.clear_marker(markers::EMERGENCY_STOP)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::markers;
    use crate::testing::MockStateStore;
    use std::fs;
    use tempfile::TempDir;

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
    fn test_enter_problem_mode_creates_database() {
        let dir = TempDir::new().unwrap();
        // Base dir is the temp dir, database will be created in ~/.claude-reliability/

        enter_problem_mode(dir.path()).unwrap();

        // Verify problem mode is active (database was created)
        assert!(is_problem_mode_active(dir.path()));
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
    fn test_set_reflect_marker_creates_database() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        // Database will be created in ~/.claude-reliability/

        set_reflect_marker(base).unwrap();

        // Verify marker is set (database was created)
        assert!(has_reflect_marker(base));
    }

    #[test]
    fn test_emergency_stop_not_active_by_default() {
        let dir = TempDir::new().unwrap();
        assert!(!is_emergency_stop_active(dir.path()));
    }

    #[test]
    fn test_set_emergency_stop() {
        let dir = TempDir::new().unwrap();

        set_emergency_stop(dir.path()).unwrap();
        assert!(is_emergency_stop_active(dir.path()));
    }

    #[test]
    fn test_set_emergency_stop_with_store() {
        let store = MockStateStore::new();

        assert!(!is_emergency_stop_active_with_store(&store));

        set_emergency_stop_with_store(&store).unwrap();

        assert!(is_emergency_stop_active_with_store(&store));
    }

    #[test]
    fn test_clear_emergency_stop() {
        let dir = TempDir::new().unwrap();

        set_emergency_stop(dir.path()).unwrap();
        assert!(is_emergency_stop_active(dir.path()));

        clear_emergency_stop(dir.path()).unwrap();
        assert!(!is_emergency_stop_active(dir.path()));
    }

    #[test]
    fn test_clear_emergency_stop_with_store() {
        let store = MockStateStore::new();

        set_emergency_stop_with_store(&store).unwrap();
        assert!(is_emergency_stop_active_with_store(&store));

        clear_emergency_stop_with_store(&store).unwrap();
        assert!(!is_emergency_stop_active_with_store(&store));
    }

    #[test]
    fn test_clear_emergency_stop_when_not_active() {
        let dir = TempDir::new().unwrap();

        // Should not error when clearing without setting
        clear_emergency_stop(dir.path()).unwrap();
        assert!(!is_emergency_stop_active(dir.path()));
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
}
