//! `UserPromptSubmit` hook for resetting session state.
//!
//! This hook runs when the user submits a new prompt. It resets the
//! reflection marker so that the reflection check runs again
//! on the next stop attempt after making changes.

use crate::error::Result;
use crate::session;
use std::path::Path;

/// Run the user prompt submit hook.
///
/// This hook resets session state when the user sends a new message,
/// including clearing the reflection marker and validation marker.
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

    // Clear the needs validation marker - user has seen changes
    session::clear_needs_validation(base)?;

    // Clear the reflection marker so the next stop with modifying tools will prompt again
    session::clear_reflect_marker(base)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_user_prompt_submit_clears_reflect_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the reflection marker
        session::set_reflect_marker(base).unwrap();
        assert!(session::has_reflect_marker(base));

        // Run the hook
        run_user_prompt_submit_hook(Some(base)).unwrap();

        // Marker should be cleared
        assert!(!session::has_reflect_marker(base));
    }

    #[test]
    fn test_user_prompt_submit_no_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // No marker exists initially
        assert!(!session::has_reflect_marker(base));

        // Should not error when marker doesn't exist
        run_user_prompt_submit_hook(Some(base)).unwrap();
    }

    #[test]
    fn test_user_prompt_submit_default_dir() {
        // Test with default directory (current directory)
        // Just ensure it doesn't error
        let _ = run_user_prompt_submit_hook(None);
    }

    #[test]
    fn test_user_prompt_submit_clears_needs_validation_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the needs validation marker
        session::set_needs_validation(base).unwrap();
        assert!(session::needs_validation(base));

        // Run the hook
        run_user_prompt_submit_hook(Some(base)).unwrap();

        // Marker should be cleared
        assert!(!session::needs_validation(base));
    }
}
