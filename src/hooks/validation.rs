//! Pre-tool-use hook for tracking when validation is needed.
//!
//! This hook sets a marker when modifying tools (`Edit`, `Write`, `NotebookEdit`) are used,
//! indicating that validation must run before stopping.

use crate::hooks::{HookInput, PreToolUseOutput};
use crate::session;
use crate::storage::SqliteStore;
use crate::traits::StateStore;
use std::path::Path;

/// Tools that modify files and require validation before stopping.
const MODIFYING_TOOLS: &[&str] = &["Edit", "Write", "NotebookEdit"];

/// Run the validation tracking hook.
///
/// This hook sets a "needs validation" marker when modifying tools are used.
/// The stop hook will then require validation to pass before allowing exit.
///
/// # Arguments
///
/// * `input` - The hook input from Claude Code
/// * `base_dir` - The base directory for marker files
pub fn run_validation_hook(input: &HookInput, base_dir: &Path) -> PreToolUseOutput {
    // Create the default store and delegate to the testable version
    match SqliteStore::new(base_dir) {
        Ok(store) => run_validation_hook_with_store(input, &store),
        Err(e) => {
            eprintln!("Warning: Failed to create store for validation marker: {e}");
            PreToolUseOutput::allow(None)
        }
    }
}

/// Run the validation tracking hook with a provided store.
///
/// This is the testable version that accepts a `StateStore` trait.
pub fn run_validation_hook_with_store(
    input: &HookInput,
    store: &dyn StateStore,
) -> PreToolUseOutput {
    let tool_name = input.tool_name.as_deref().unwrap_or("");

    // Check if this is a modifying tool
    if MODIFYING_TOOLS.contains(&tool_name) {
        // Set the marker indicating validation is needed
        if let Err(e) = session::set_needs_validation_with_store(store) {
            eprintln!("Warning: Failed to set needs_validation marker: {e}");
        }
    }

    // Always allow the tool - we're just tracking, not blocking
    PreToolUseOutput::allow(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ToolInput;
    use crate::testing::{FailingSetMarkerStore, MockStateStore};
    use tempfile::TempDir;

    fn make_input(tool_name: &str) -> HookInput {
        HookInput {
            transcript_path: None,
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(ToolInput::default()),
        }
    }

    #[test]
    fn test_edit_sets_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        assert!(!session::needs_validation(base));

        let input = make_input("Edit");
        let output = run_validation_hook(&input, base);

        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(session::needs_validation(base));
    }

    #[test]
    fn test_write_sets_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        assert!(!session::needs_validation(base));

        let input = make_input("Write");
        let output = run_validation_hook(&input, base);

        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(session::needs_validation(base));
    }

    #[test]
    fn test_notebook_edit_sets_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        assert!(!session::needs_validation(base));

        let input = make_input("NotebookEdit");
        let output = run_validation_hook(&input, base);

        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(session::needs_validation(base));
    }

    #[test]
    fn test_read_does_not_set_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = make_input("Read");
        let output = run_validation_hook(&input, base);

        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(!session::needs_validation(base));
    }

    #[test]
    fn test_bash_does_not_set_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = make_input("Bash");
        let output = run_validation_hook(&input, base);

        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(!session::needs_validation(base));
    }

    #[test]
    fn test_glob_does_not_set_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = make_input("Glob");
        let output = run_validation_hook(&input, base);

        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(!session::needs_validation(base));
    }

    #[test]
    fn test_marker_set_failure_still_allows_with_store() {
        // Use FailingSetMarkerStore to test the error path
        let inner = MockStateStore::new();
        let store = FailingSetMarkerStore::new(inner, "simulated set failure");

        let input = make_input("Edit");
        let output = run_validation_hook_with_store(&input, &store);

        // Should still allow even if marker fails to set
        assert!(output.hook_specific_output.permission_decision == "allow");
    }

    #[test]
    fn test_non_modifying_tool_with_failing_store() {
        // Even with a failing store, non-modifying tools should work
        let inner = MockStateStore::new();
        let store = FailingSetMarkerStore::new(inner, "simulated set failure");

        let input = make_input("Read");
        let output = run_validation_hook_with_store(&input, &store);

        // Should allow (and no attempt to set marker)
        assert!(output.hook_specific_output.permission_decision == "allow");
    }

    #[test]
    #[cfg(unix)]
    fn test_store_creation_failure_still_allows() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create .claude-reliability dir and make it read-only to cause store creation failure
        let reliability_dir = base.join(".claude-reliability");
        fs::create_dir_all(&reliability_dir).unwrap();
        fs::set_permissions(&reliability_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let input = make_input("Edit");
        let output = run_validation_hook(&input, base);

        // Should still allow even if store creation fails
        assert!(output.hook_specific_output.permission_decision == "allow");

        // Clean up: restore permissions so tempdir can be deleted
        fs::set_permissions(&reliability_dir, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
