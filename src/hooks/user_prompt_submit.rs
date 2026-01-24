//! `UserPromptSubmit` hook for resetting session state.
//!
//! This hook runs when the user submits a new prompt. It resets the
//! reflection marker so that the reflection check runs again
//! on the next stop attempt after making changes.

use crate::error::Result;
use crate::session;
use std::path::Path;

/// Input provided to `UserPromptSubmit` hooks by Claude Code.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPromptSubmitInput {
    /// Whether this prompt is a compaction summary.
    #[serde(default)]
    pub is_compact_summary: bool,
    /// Path to the previous transcript (for post-compaction recovery).
    #[serde(default)]
    pub transcript_path: Option<String>,
}

/// Output from the `UserPromptSubmit` hook.
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPromptSubmitOutput {
    /// Optional message to inject into the conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
}

/// Run the user prompt submit hook.
///
/// This hook resets session state when the user sends a new message,
/// including clearing the reflection marker and validation marker.
/// It also detects post-compaction scenarios and prompts task recovery.
///
/// # Arguments
///
/// * `input` - The hook input (may contain compaction info).
/// * `base_dir` - Optional base directory (defaults to current directory).
///
/// # Errors
///
/// Returns an error if file operations fail.
pub fn run_user_prompt_submit_hook(
    input: &UserPromptSubmitInput,
    base_dir: Option<&Path>,
) -> Result<UserPromptSubmitOutput> {
    let base = base_dir.unwrap_or_else(|| Path::new("."));

    // Clear the needs validation marker - user has seen changes
    session::clear_needs_validation(base)?;

    // Clear the reflection marker so the next stop with modifying tools will prompt again
    session::clear_reflect_marker(base)?;

    // Check if this is a post-compaction scenario
    if input.is_compact_summary {
        return Ok(UserPromptSubmitOutput { system_message: Some(post_compaction_message(input)) });
    }

    Ok(UserPromptSubmitOutput::default())
}

/// Generate the post-compaction message to prompt work item recovery.
fn post_compaction_message(input: &UserPromptSubmitInput) -> String {
    let mut msg = String::from(
        "# Post-Compaction Work Item Check\n\n\
         Context was just compacted. Please verify that no user-requested work items were lost:\n\n\
         1. Review the compaction summary above for any user requests or mentioned issues\n\
         2. Check the work item database using `list_tasks` to see current tracked items\n\
         3. If you notice any user requests that aren't tracked, create work items for them immediately\n\n\
         This ensures nothing the user mentioned gets forgotten due to context compaction.",
    );

    if let Some(path) = &input.transcript_path {
        use std::fmt::Write;
        let _ = write!(
            msg,
            "\n\nPrevious transcript available at: {path}\n\
             You can read this file to review user messages if needed."
        );
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn default_input() -> UserPromptSubmitInput {
        UserPromptSubmitInput::default()
    }

    #[test]
    fn test_user_prompt_submit_clears_reflect_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the reflection marker
        session::set_reflect_marker(base).unwrap();
        assert!(session::has_reflect_marker(base));

        // Run the hook
        run_user_prompt_submit_hook(&default_input(), Some(base)).unwrap();

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
        run_user_prompt_submit_hook(&default_input(), Some(base)).unwrap();
    }

    #[test]
    fn test_user_prompt_submit_default_dir() {
        // Test with default directory (current directory)
        // Just ensure it doesn't error
        let _ = run_user_prompt_submit_hook(&default_input(), None);
    }

    #[test]
    fn test_user_prompt_submit_clears_needs_validation_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the needs validation marker
        session::set_needs_validation(base).unwrap();
        assert!(session::needs_validation(base));

        // Run the hook
        run_user_prompt_submit_hook(&default_input(), Some(base)).unwrap();

        // Marker should be cleared
        assert!(!session::needs_validation(base));
    }

    #[test]
    fn test_user_prompt_submit_post_compaction() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = UserPromptSubmitInput { is_compact_summary: true, transcript_path: None };

        let output = run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("Post-Compaction Work Item Check"));
        assert!(msg.contains("list_tasks"));
    }

    #[test]
    fn test_user_prompt_submit_post_compaction_with_transcript() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = UserPromptSubmitInput {
            is_compact_summary: true,
            transcript_path: Some("/path/to/transcript.jsonl".to_string()),
        };

        let output = run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("/path/to/transcript.jsonl"));
    }

    #[test]
    fn test_user_prompt_submit_no_compaction() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = UserPromptSubmitInput { is_compact_summary: false, transcript_path: None };

        let output = run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        assert!(output.system_message.is_none());
    }
}
