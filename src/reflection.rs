//! Self-reflection tracking for the stop hook.
//!
//! This module provides marker file management for tracking whether
//! the assistant has reflected on its work during the current session.
//! The marker is set after the first reflection and cleared when the
//! user submits a new prompt.

use crate::error::Result;
use std::path::Path;

/// Marker file for tracking that reflection has been completed (relative to base).
const REFLECTION_MARKER_REL: &str = ".claude/reflection-done.local";

/// Marker file for tracking that we blocked on uncommitted changes (relative to base).
/// This is used to ensure reflection runs even if Claude commits and pushes in one command.
const HAD_CHANGES_MARKER_REL: &str = ".claude/had-changes.local";

/// Check if the reflection marker exists.
pub fn has_reflection_marker() -> bool {
    has_reflection_marker_in(Path::new("."))
}

/// Check if the reflection marker exists in the specified directory.
pub fn has_reflection_marker_in(base_dir: &Path) -> bool {
    base_dir.join(REFLECTION_MARKER_REL).exists()
}

/// Mark that reflection has been completed.
///
/// # Errors
///
/// Returns an error if the marker file cannot be written.
pub fn mark_reflection_done() -> Result<()> {
    mark_reflection_done_in(Path::new("."))
}

/// Mark that reflection has been completed in the specified directory.
///
/// # Errors
///
/// Returns an error if the marker file cannot be written.
pub fn mark_reflection_done_in(base_dir: &Path) -> Result<()> {
    let marker_path = base_dir.join(REFLECTION_MARKER_REL);
    // Ensure parent directory exists
    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(marker_path, "")?;
    Ok(())
}

/// Clear the reflection marker.
///
/// # Errors
///
/// Returns an error if the marker file cannot be removed.
pub fn clear_reflection_marker() -> Result<()> {
    clear_reflection_marker_in(Path::new("."))
}

/// Clear the reflection marker in the specified directory.
///
/// # Errors
///
/// Returns an error if the marker file cannot be removed.
pub fn clear_reflection_marker_in(base_dir: &Path) -> Result<()> {
    let marker_path = base_dir.join(REFLECTION_MARKER_REL);
    if marker_path.exists() {
        std::fs::remove_file(marker_path)?;
    }
    Ok(())
}

/// Check if we blocked on uncommitted changes this session.
pub fn had_uncommitted_changes_in(base_dir: &Path) -> bool {
    base_dir.join(HAD_CHANGES_MARKER_REL).exists()
}

/// Mark that we blocked on uncommitted changes.
///
/// # Errors
///
/// Returns an error if the marker file cannot be written.
pub fn mark_had_uncommitted_changes_in(base_dir: &Path) -> Result<()> {
    let marker_path = base_dir.join(HAD_CHANGES_MARKER_REL);
    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(marker_path, "")?;
    Ok(())
}

/// Clear the "had uncommitted changes" marker.
///
/// # Errors
///
/// Returns an error if the marker file cannot be removed.
pub fn clear_had_uncommitted_changes_in(base_dir: &Path) -> Result<()> {
    let marker_path = base_dir.join(HAD_CHANGES_MARKER_REL);
    if marker_path.exists() {
        std::fs::remove_file(marker_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_reflection_marker_not_exists() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        assert!(!has_reflection_marker_in(base));
    }

    #[test]
    fn test_reflection_marker_lifecycle() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Marker doesn't exist initially
        assert!(!has_reflection_marker_in(base));

        // Create the marker
        mark_reflection_done_in(base).unwrap();
        assert!(has_reflection_marker_in(base));

        // Clear it
        clear_reflection_marker_in(base).unwrap();
        assert!(!has_reflection_marker_in(base));
    }

    #[test]
    fn test_clear_reflection_marker_not_exists() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Should not error when marker doesn't exist
        clear_reflection_marker_in(base).unwrap();
    }

    #[test]
    fn test_mark_reflection_creates_parent_dir() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // .claude directory doesn't exist yet
        assert!(!base.join(".claude").exists());

        mark_reflection_done_in(base).unwrap();

        // .claude directory should be created
        assert!(base.join(".claude").exists());
        assert!(has_reflection_marker_in(base));
    }

    #[test]
    fn test_wrapper_functions() {
        // Test the wrapper functions that use current directory
        // First ensure we're in a clean state
        let _ = clear_reflection_marker();

        // Test has_reflection_marker
        let initial = has_reflection_marker();
        // Could be true or false depending on previous tests

        // If marker exists, clear it for clean test
        if initial {
            clear_reflection_marker().unwrap();
        }

        assert!(!has_reflection_marker());

        // Create marker
        mark_reflection_done().unwrap();
        assert!(has_reflection_marker());

        // Clean up
        clear_reflection_marker().unwrap();
        assert!(!has_reflection_marker());
    }

    #[test]
    fn test_had_uncommitted_changes_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Initially no marker
        assert!(!had_uncommitted_changes_in(base));

        // Set the marker
        mark_had_uncommitted_changes_in(base).unwrap();
        assert!(had_uncommitted_changes_in(base));

        // Clear it
        clear_had_uncommitted_changes_in(base).unwrap();
        assert!(!had_uncommitted_changes_in(base));
    }

    #[test]
    fn test_clear_had_uncommitted_changes_not_exists() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Should not error when marker doesn't exist
        clear_had_uncommitted_changes_in(base).unwrap();
    }
}
