//! Pre-tool-use hook for enforcing JKW setup.
//!
//! This hook ensures that when just-keep-working mode is invoked, the session
//! file is created before any other file operations occur.
//!
//! ## Detection
//!
//! JKW invocation is detected when the Skill tool is called with:
//! - `skill: "just-keep-working"`
//! - `skill: "claude-reliability:just-keep-working"`
//!
//! ## Enforcement
//!
//! When JKW is invoked but the session file doesn't exist, this hook:
//! 1. Sets a marker indicating JKW setup is required
//! 2. Blocks Write/Edit operations to files other than the session file
//! 3. Provides a message telling the agent to create the session file first
//!
//! The marker is cleared once the session file exists.

use crate::hooks::{HookInput, PreToolUseOutput};
use crate::session::{
    clear_jkw_setup_required, is_jkw_setup_required, jkw_session_file_exists,
    set_jkw_setup_required, SESSION_NOTES_PATH,
};
use std::path::Path;

/// Check if the skill name indicates JKW invocation.
fn is_jkw_skill(skill: &str) -> bool {
    skill == "just-keep-working" || skill.ends_with(":just-keep-working")
}

/// Check if a file path is the JKW session file (or in .claude directory).
fn is_jkw_session_path(file_path: &str) -> bool {
    let path = Path::new(file_path);

    // Allow any path that contains .claude/ or ends with the session file name
    file_path.contains(".claude/") || path.ends_with(SESSION_NOTES_PATH)
}

/// Run the JKW setup enforcement hook.
///
/// This hook:
/// 1. Detects Skill tool calls for JKW and sets the setup marker
/// 2. Blocks Write/Edit when the marker is set and session file doesn't exist
/// 3. Clears the marker once the session file exists
///
/// # Arguments
///
/// * `input` - The hook input from Claude Code
/// * `base_dir` - The base directory to check for markers and session files
pub fn run_jkw_setup_hook(input: &HookInput, base_dir: &Path) -> PreToolUseOutput {
    let tool_name = input.tool_name.as_deref().unwrap_or("");
    let tool_input = input.tool_input.as_ref();

    // Check if this is a Skill tool call for JKW
    if tool_name == "Skill" {
        if let Some(skill) = tool_input.and_then(|t| t.skill.as_deref()) {
            if is_jkw_skill(skill) {
                // JKW is being invoked - check if session file exists
                if !jkw_session_file_exists(base_dir) {
                    // Set the marker to enforce setup
                    if let Err(e) = set_jkw_setup_required(base_dir) {
                        eprintln!("Warning: Failed to set JKW setup marker: {e}");
                    }
                }
            }
        }
        // Always allow the Skill tool itself
        return PreToolUseOutput::allow(None);
    }

    // Check if JKW setup is required
    if is_jkw_setup_required(base_dir) {
        // Check if session file now exists
        if jkw_session_file_exists(base_dir) {
            // Session file created - clear the marker
            if let Err(e) = clear_jkw_setup_required(base_dir) {
                eprintln!("Warning: Failed to clear JKW setup marker: {e}");
            }
            return PreToolUseOutput::allow(None);
        }

        // Session file still doesn't exist - block Write/Edit to non-session files
        if tool_name == "Write" || tool_name == "Edit" {
            if let Some(file_path) = tool_input.and_then(|t| t.file_path.as_deref()) {
                if !is_jkw_session_path(file_path) {
                    return PreToolUseOutput::block(Some(format!(
                        "BLOCKED: Just-keep-working mode has been invoked but the session file \
                         does not exist yet.\n\n\
                         Before making any code changes, you MUST:\n\
                         1. Create the JKW session file at: {SESSION_NOTES_PATH}\n\
                         2. Include: session goal, success criteria, constraints, and quality requirements\n\n\
                         This ensures proper tracking of the autonomous work session."
                    )));
                }
            }
        }
    }

    PreToolUseOutput::allow(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ToolInput;
    use std::fs;
    use tempfile::TempDir;

    fn make_input(tool_name: &str, skill: Option<&str>, file_path: Option<&str>) -> HookInput {
        HookInput {
            transcript_path: None,
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(ToolInput {
                command: None,
                skill: skill.map(String::from),
                file_path: file_path.map(String::from),
            }),
        }
    }

    #[test]
    fn test_is_jkw_skill() {
        assert!(is_jkw_skill("just-keep-working"));
        assert!(is_jkw_skill("claude-reliability:just-keep-working"));
        assert!(is_jkw_skill("foo:just-keep-working"));
        assert!(!is_jkw_skill("other-skill"));
        assert!(!is_jkw_skill("just-keep-working-extra"));
    }

    #[test]
    fn test_is_jkw_session_path() {
        assert!(is_jkw_session_path(".claude/jkw-session.local.md"));
        assert!(is_jkw_session_path("/home/user/project/.claude/jkw-session.local.md"));
        assert!(is_jkw_session_path(".claude/other-file.md"));
        assert!(!is_jkw_session_path("src/main.rs"));
        assert!(!is_jkw_session_path("/home/user/project/src/lib.rs"));
    }

    #[test]
    fn test_skill_tool_sets_marker_when_no_session_file() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // No session file exists
        assert!(!is_jkw_setup_required(base));

        // Invoke JKW skill
        let input = make_input("Skill", Some("just-keep-working"), None);
        let output = run_jkw_setup_hook(&input, base);

        // Should allow the skill call but set the marker
        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(is_jkw_setup_required(base));
    }

    #[test]
    fn test_skill_tool_no_marker_when_session_exists() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the session file
        let session_path = base.join(SESSION_NOTES_PATH);
        fs::create_dir_all(session_path.parent().unwrap()).unwrap();
        fs::write(&session_path, "# Session notes").unwrap();

        // Invoke JKW skill
        let input = make_input("Skill", Some("just-keep-working"), None);
        let output = run_jkw_setup_hook(&input, base);

        // Should allow and NOT set the marker
        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(!is_jkw_setup_required(base));
    }

    #[test]
    fn test_write_blocked_when_setup_required() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set the marker (simulating JKW was invoked)
        set_jkw_setup_required(base).unwrap();

        // Try to write to a non-session file
        let input = make_input("Write", None, Some("src/main.rs"));
        let output = run_jkw_setup_hook(&input, base);

        // Should be blocked
        assert!(output.hook_specific_output.permission_decision == "block");
        assert!(output.hook_specific_output.additional_context.is_some());
        let context = output.hook_specific_output.additional_context.unwrap();
        assert!(context.contains("BLOCKED"));
        assert!(context.contains("session file"));
    }

    #[test]
    fn test_edit_blocked_when_setup_required() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set the marker
        set_jkw_setup_required(base).unwrap();

        // Try to edit a non-session file
        let input = make_input("Edit", None, Some("src/lib.rs"));
        let output = run_jkw_setup_hook(&input, base);

        // Should be blocked
        assert!(output.hook_specific_output.permission_decision == "block");
    }

    #[test]
    fn test_write_allowed_to_session_file() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set the marker
        set_jkw_setup_required(base).unwrap();

        // Try to write to the session file
        let input = make_input("Write", None, Some(".claude/jkw-session.local.md"));
        let output = run_jkw_setup_hook(&input, base);

        // Should be allowed (this is the session file)
        assert!(output.hook_specific_output.permission_decision == "allow");
    }

    #[test]
    fn test_write_allowed_to_claude_dir() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set the marker
        set_jkw_setup_required(base).unwrap();

        // Try to write to any .claude file
        let input = make_input("Write", None, Some(".claude/other-notes.md"));
        let output = run_jkw_setup_hook(&input, base);

        // Should be allowed (in .claude directory)
        assert!(output.hook_specific_output.permission_decision == "allow");
    }

    #[test]
    fn test_marker_cleared_when_session_file_created() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set the marker
        set_jkw_setup_required(base).unwrap();
        assert!(is_jkw_setup_required(base));

        // Create the session file
        let session_path = base.join(SESSION_NOTES_PATH);
        fs::create_dir_all(session_path.parent().unwrap()).unwrap();
        fs::write(&session_path, "# Session notes").unwrap();

        // Any tool call should clear the marker now
        let input = make_input("Write", None, Some("src/main.rs"));
        let output = run_jkw_setup_hook(&input, base);

        // Should be allowed and marker should be cleared
        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(!is_jkw_setup_required(base));
    }

    #[test]
    fn test_other_tools_not_blocked() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set the marker
        set_jkw_setup_required(base).unwrap();

        // Other tools should not be blocked
        let input = make_input("Read", None, Some("src/main.rs"));
        let output = run_jkw_setup_hook(&input, base);
        assert!(output.hook_specific_output.permission_decision == "allow");

        let input = make_input("Bash", None, None);
        let output = run_jkw_setup_hook(&input, base);
        assert!(output.hook_specific_output.permission_decision == "allow");

        let input = make_input("Glob", None, None);
        let output = run_jkw_setup_hook(&input, base);
        assert!(output.hook_specific_output.permission_decision == "allow");
    }

    #[test]
    fn test_no_blocking_without_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // No marker set
        assert!(!is_jkw_setup_required(base));

        // Write should be allowed
        let input = make_input("Write", None, Some("src/main.rs"));
        let output = run_jkw_setup_hook(&input, base);
        assert!(output.hook_specific_output.permission_decision == "allow");
    }

    #[test]
    fn test_other_skill_does_not_set_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Invoke a different skill
        let input = make_input("Skill", Some("other-skill"), None);
        let output = run_jkw_setup_hook(&input, base);

        // Should allow but NOT set the marker
        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(!is_jkw_setup_required(base));
    }

    #[test]
    fn test_namespaced_jkw_skill() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Invoke with namespace
        let input = make_input("Skill", Some("claude-reliability:just-keep-working"), None);
        let output = run_jkw_setup_hook(&input, base);

        // Should set the marker
        assert!(output.hook_specific_output.permission_decision == "allow");
        assert!(is_jkw_setup_required(base));
    }

    #[test]
    fn test_set_marker_fails_gracefully() {
        // Use a path that can't be written to - /dev/null can't have children
        let base = Path::new("/dev/null");

        // This should not panic even though the marker can't be set
        let input = make_input("Skill", Some("just-keep-working"), None);
        let output = run_jkw_setup_hook(&input, base);

        // Should still allow the skill tool call
        assert!(output.hook_specific_output.permission_decision == "allow");
    }

    #[test]
    fn test_clear_marker_fails_gracefully() {
        // Use a directory where we can set the marker but then make it non-removable
        // This is tricky on Unix. Instead, we'll use a non-existent base where
        // is_jkw_setup_required returns false but clear would fail if attempted.
        // Actually, since is_jkw_setup_required checks existence first, the
        // clear path is only taken when the marker exists.

        // For this test, we need to manually create a scenario where
        // clear_jkw_setup_required fails. We can do this by making the
        // marker file read-only and the directory read-only.

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the marker
        set_jkw_setup_required(base).unwrap();

        // Make the .claude directory read-only (so we can't delete files)
        let claude_dir = base.join(".claude");
        let original_perms = fs::metadata(&claude_dir).unwrap().permissions();
        let mut read_only = original_perms.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&claude_dir, read_only).unwrap();

        // Create the session file so the hook tries to clear the marker
        // But wait - we can't create files in a read-only directory.
        // Let's restore permissions first.
        fs::set_permissions(&claude_dir, original_perms.clone()).unwrap();
        let session_path = base.join(SESSION_NOTES_PATH);
        fs::write(&session_path, "# Session").unwrap();

        // Now make it read-only again
        fs::set_permissions(&claude_dir, {
            let mut p = original_perms.clone();
            p.set_readonly(true);
            p
        })
        .unwrap();

        // Try a tool call - this should trigger clear_jkw_setup_required which will fail
        let input = make_input("Write", None, Some("src/main.rs"));
        let output = run_jkw_setup_hook(&input, base);

        // Restore permissions for cleanup
        fs::set_permissions(&claude_dir, original_perms).unwrap();

        // The hook should still allow (because session file exists) even though
        // clear failed
        assert!(output.hook_specific_output.permission_decision == "allow");
    }
}
