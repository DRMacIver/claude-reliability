//! `UserPromptSubmit` hook for resetting session state.
//!
//! This hook runs when the user submits a new prompt. It resets the
//! self-reflection marker so that the reflection check runs again
//! on the next stop attempt after making changes.

use crate::error::Result;
use crate::reflection;
use std::path::Path;

/// Run the user prompt submit hook.
///
/// This hook resets session state when the user sends a new message,
/// including clearing the self-reflection marker.
///
/// # Arguments
///
/// * `base_dir` - Optional base directory (defaults to current directory).
///
/// # Errors
///
/// Returns an error if file operations fail.
pub fn run_user_prompt_submit_hook(base_dir: Option<&Path>) -> Result<()> {
    let base = base_dir.unwrap_or_else(|| Path::new("."));

    // Clear the reflection marker so the next stop will trigger reflection
    reflection::clear_reflection_marker_in(base)?;

    // Clear the "had uncommitted changes" marker as this is a new prompt
    reflection::clear_had_uncommitted_changes_in(base)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_user_prompt_submit_clears_reflection_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the reflection marker
        reflection::mark_reflection_done_in(base).unwrap();
        assert!(reflection::has_reflection_marker_in(base));

        // Run the hook
        run_user_prompt_submit_hook(Some(base)).unwrap();

        // Marker should be cleared
        assert!(!reflection::has_reflection_marker_in(base));
    }

    #[test]
    fn test_user_prompt_submit_clears_had_uncommitted_changes_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the "had uncommitted changes" marker
        reflection::mark_had_uncommitted_changes_in(base).unwrap();
        assert!(reflection::had_uncommitted_changes_in(base));

        // Run the hook
        run_user_prompt_submit_hook(Some(base)).unwrap();

        // Marker should be cleared
        assert!(!reflection::had_uncommitted_changes_in(base));
    }

    #[test]
    fn test_user_prompt_submit_no_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // No marker exists initially
        assert!(!reflection::has_reflection_marker_in(base));

        // Should not error when marker doesn't exist
        run_user_prompt_submit_hook(Some(base)).unwrap();
    }

    #[test]
    fn test_user_prompt_submit_default_dir() {
        // Test with default directory (current directory)
        // Just ensure it doesn't error
        let _ = run_user_prompt_submit_hook(None);
    }
}
