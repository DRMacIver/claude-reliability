//! `UserPromptSubmit` hook for resetting session state.
//!
//! This hook runs when the user submits a new prompt. It resets the
//! reflection marker so that the reflection check runs again
//! on the next stop attempt after making changes.
//! It also records user messages for verification during the reflection prompt.

use crate::error::Result;
use crate::session;
use crate::tasks;
use std::path::Path;

/// Input provided to `UserPromptSubmit` hooks by Claude Code.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPromptSubmitInput {
    /// Whether this prompt is a compaction summary.
    #[serde(default)]
    pub is_compact_summary: bool,
    /// Path to the conversation transcript.
    #[serde(default, alias = "transcript_path")]
    pub transcript_path: Option<String>,
    /// The user's prompt text.
    #[serde(default)]
    pub prompt: Option<String>,
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
    let single_work_item_id = crate::single_work_item::get_single_work_item_id();
    run_user_prompt_submit_hook_inner(input, base_dir, single_work_item_id.as_deref())
}

/// Check if this is the opening prompt (first user message in the session).
///
/// Returns `true` if no transcript is available, the transcript can't be parsed,
/// or no assistant output has been produced yet.
fn is_opening_prompt(input: &UserPromptSubmitInput) -> bool {
    let Some(transcript_path) = &input.transcript_path else {
        return true; // No transcript → first prompt
    };
    let path = Path::new(transcript_path);
    let Ok(info) = crate::transcript::parse_transcript(path) else {
        return true; // Can't parse → assume first prompt
    };
    // If there's any assistant output, the agent has already responded
    info.last_assistant_output.is_none()
}

/// Check if the prompt is a task notification from a background agent.
///
/// Task notifications have the form `<task-notification> <task-id>...` and should
/// not be recorded as user messages since they are system-generated noise.
fn is_task_notification(prompt: &str) -> bool {
    prompt.trim_start().starts_with("<task-notification>")
}

/// Inner implementation that accepts single work item ID explicitly for testability.
fn run_user_prompt_submit_hook_inner(
    input: &UserPromptSubmitInput,
    base_dir: Option<&Path>,
    single_work_item_id: Option<&str>,
) -> Result<UserPromptSubmitOutput> {
    let base = base_dir.unwrap_or_else(|| Path::new("."));

    // Clear the needs validation marker - user has seen changes
    session::clear_needs_validation(base)?;

    // Clear the reflection marker so the next stop with modifying tools will prompt again
    session::clear_reflect_marker(base)?;

    let session_id = input.transcript_path.as_deref().unwrap_or("unknown");

    // Check if this is a post-compaction scenario
    if input.is_compact_summary {
        // Mark existing messages as pre-compaction (they're still relevant)
        tasks::mark_pre_compaction_messages(base, session_id);
        let compaction_msg = post_compaction_message(input);
        return Ok(UserPromptSubmitOutput { system_message: Some(compaction_msg) });
    }

    // Single work item mode: validate the assigned item and announce it
    if let Some(swi_id) = single_work_item_id {
        return match crate::single_work_item::validate_work_item(base, swi_id) {
            Ok((id, title)) => {
                let swi_msg = format!(
                    "Single work item mode active. Assigned item: [{id}] {title}\n\
                     Only this item needs to be completed to exit."
                );
                Ok(UserPromptSubmitOutput { system_message: Some(swi_msg) })
            }
            Err(msg) => {
                let error_msg = format!("ERROR: {msg}");
                Ok(UserPromptSubmitOutput { system_message: Some(error_msg) })
            }
        };
    }

    // Record user message for verification during reflection prompt
    // Skip task notifications — they are system-generated noise from background agents
    if let Some(prompt) = &input.prompt {
        if !is_task_notification(prompt) {
            let context = if is_opening_prompt(input) { "opening prompt" } else { "follow-up" };

            // Clear previous messages on opening prompt (new session)
            if is_opening_prompt(input) {
                tasks::clear_session_user_messages(base, session_id);
            }

            tasks::record_user_message(
                base,
                prompt,
                context,
                input.transcript_path.as_deref(),
                session_id,
            );
        }
    }

    Ok(UserPromptSubmitOutput { system_message: None })
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
    use crate::tasks::{Priority, SqliteTaskStore, TaskFilter, TaskStore};
    use tempfile::TempDir;

    fn default_input() -> UserPromptSubmitInput {
        UserPromptSubmitInput::default()
    }

    fn input_with_prompt(prompt: &str) -> UserPromptSubmitInput {
        UserPromptSubmitInput { prompt: Some(prompt.to_string()), ..Default::default() }
    }

    #[test]
    fn test_user_prompt_submit_clears_reflect_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        session::set_reflect_marker(base).unwrap();
        assert!(session::has_reflect_marker(base));

        run_user_prompt_submit_hook(&default_input(), Some(base)).unwrap();

        assert!(!session::has_reflect_marker(base));
    }

    #[test]
    fn test_user_prompt_submit_no_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        assert!(!session::has_reflect_marker(base));
        run_user_prompt_submit_hook(&default_input(), Some(base)).unwrap();
    }

    #[test]
    fn test_user_prompt_submit_default_dir() {
        let _ = run_user_prompt_submit_hook(&default_input(), None);
    }

    #[test]
    fn test_user_prompt_submit_clears_needs_validation_marker() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        session::set_needs_validation(base).unwrap();
        assert!(session::needs_validation(base));

        run_user_prompt_submit_hook(&default_input(), Some(base)).unwrap();

        assert!(!session::needs_validation(base));
    }

    #[test]
    fn test_user_prompt_submit_post_compaction() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input =
            UserPromptSubmitInput { is_compact_summary: true, transcript_path: None, prompt: None };

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
            prompt: None,
        };

        let output = run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("/path/to/transcript.jsonl"));
    }

    #[test]
    fn test_user_prompt_submit_no_system_message_without_prompt() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = UserPromptSubmitInput {
            is_compact_summary: false,
            transcript_path: None,
            prompt: None,
        };

        let output = run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        // No prompt and not compaction = no system message needed
        assert!(output.system_message.is_none());
    }

    #[test]
    fn test_user_prompt_submit_no_system_message_after_first_prompt() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create a transcript file that has an assistant response (not opening prompt)
        let transcript_path = dir.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Hello"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hi there!"}]}}
"#,
        )
        .unwrap();

        let input = UserPromptSubmitInput {
            is_compact_summary: false,
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            prompt: Some("Follow up question".to_string()),
        };

        let output = run_user_prompt_submit_hook_inner(&input, Some(base), None).unwrap();

        assert!(
            output.system_message.is_none(),
            "system_message should be None, got: {:?}",
            output.system_message
        );
    }

    #[test]
    fn test_user_prompt_records_message() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = input_with_prompt("Fix the login bug");
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        // Verify a user message was recorded
        let messages = tasks::get_session_user_messages(base, "unknown");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message, "Fix the login bug");
        assert_eq!(messages[0].context, "opening prompt");
    }

    #[test]
    fn test_user_prompt_records_follow_up() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create a transcript that has an assistant response (not opening prompt)
        let transcript_path = dir.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Hello"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hi there!"}]}}
"#,
        )
        .unwrap();

        let input = UserPromptSubmitInput {
            prompt: Some("Follow up question".to_string()),
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };
        run_user_prompt_submit_hook_inner(&input, Some(base), None).unwrap();

        let session_id = transcript_path.to_string_lossy().to_string();
        let messages = tasks::get_session_user_messages(base, &session_id);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message, "Follow up question");
        assert_eq!(messages[0].context, "follow-up");
    }

    #[test]
    fn test_user_prompt_no_message_without_prompt() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = default_input();
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        let messages = tasks::get_session_user_messages(base, "unknown");
        assert!(messages.is_empty());
    }

    #[test]
    fn test_user_prompt_no_message_for_compaction() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = UserPromptSubmitInput {
            is_compact_summary: true,
            prompt: Some("This is a compaction summary".to_string()),
            ..Default::default()
        };
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        let messages = tasks::get_session_user_messages(base, "unknown");
        assert!(messages.is_empty());
    }

    #[test]
    fn test_compaction_marks_pre_compaction() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let session_id = "/tmp/test-session.jsonl";

        // First, record a message
        tasks::record_user_message(
            base,
            "First message",
            "opening prompt",
            Some(session_id),
            session_id,
        );

        // Then send a compaction event
        let input = UserPromptSubmitInput {
            is_compact_summary: true,
            transcript_path: Some(session_id.to_string()),
            prompt: None,
        };
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        // Verify the message is marked as pre-compaction
        let messages = tasks::get_session_user_messages(base, session_id);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].pre_compaction);
    }

    #[test]
    fn test_opening_prompt_clears_previous_messages() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Record a message from a previous session with the same ID
        tasks::record_user_message(base, "Old message", "opening prompt", None, "unknown");

        // Opening prompt should clear previous messages
        let input = input_with_prompt("New session message");
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        let messages = tasks::get_session_user_messages(base, "unknown");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message, "New session message");
    }

    #[test]
    fn test_deserialize_input_with_prompt() {
        let json = r#"{"prompt": "Hello world", "transcript_path": "/tmp/test.jsonl"}"#;
        let input: UserPromptSubmitInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.prompt.as_deref(), Some("Hello world"));
        assert_eq!(input.transcript_path.as_deref(), Some("/tmp/test.jsonl"));
        assert!(!input.is_compact_summary);
    }

    #[test]
    fn test_deserialize_input_camel_case() {
        let json = r#"{"isCompactSummary": true, "transcriptPath": "/tmp/test.jsonl"}"#;
        let input: UserPromptSubmitInput = serde_json::from_str(json).unwrap();
        assert!(input.is_compact_summary);
        assert_eq!(input.transcript_path.as_deref(), Some("/tmp/test.jsonl"));
    }

    // -- Single work item mode tests --

    #[test]
    fn test_single_work_item_valid_announces_mode() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("My assigned task", "Do the thing", Priority::Medium).unwrap();

        let input = input_with_prompt("Hello");
        let output = run_user_prompt_submit_hook_inner(&input, Some(base), Some(&task.id)).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("Single work item mode active"), "msg: {msg}");
        assert!(msg.contains(&task.id), "msg: {msg}");
        assert!(msg.contains("My assigned task"), "msg: {msg}");
    }

    #[test]
    fn test_single_work_item_invalid_id_returns_error() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let _store = SqliteTaskStore::for_project(base).unwrap();

        let input = input_with_prompt("Hello");
        let output =
            run_user_prompt_submit_hook_inner(&input, Some(base), Some("nonexistent-id")).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("ERROR"), "msg: {msg}");
        assert!(msg.contains("not found"), "msg: {msg}");
    }

    #[test]
    fn test_single_work_item_skips_user_message_recording() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Assigned item", "Work on this", Priority::Medium).unwrap();

        let input = input_with_prompt("Please fix the bug");
        run_user_prompt_submit_hook_inner(&input, Some(base), Some(&task.id)).unwrap();

        // Only the pre-existing task should exist (no user message recorded in SWI mode)
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, task.id);
    }

    #[test]
    fn test_single_work_item_no_binary_msg() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("My assigned task", "Do the thing", Priority::Medium).unwrap();

        // Create a transcript with an assistant response (not opening prompt)
        let transcript_path = dir.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Hello"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hi!"}]}}
"#,
        )
        .unwrap();

        let input = UserPromptSubmitInput {
            prompt: Some("Continue work".to_string()),
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let output = run_user_prompt_submit_hook_inner(&input, Some(base), Some(&task.id)).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("Single work item mode active"), "msg: {msg}");
    }

    #[test]
    fn test_is_task_notification_true() {
        assert!(is_task_notification(
            "<task-notification> <task-id>b8e2422</task-id> <output-file>/tmp/claude</output-file>"
        ));
        // Leading whitespace should still match
        assert!(is_task_notification("  <task-notification> <task-id>abc</task-id>"));
    }

    #[test]
    fn test_is_task_notification_false() {
        assert!(!is_task_notification("Fix the login bug"));
        assert!(!is_task_notification(""));
        assert!(!is_task_notification("some text <task-notification>"));
    }

    #[test]
    fn test_task_notification_not_recorded() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = input_with_prompt(
            "<task-notification> <task-id>b8e2422</task-id> <output-file>/tmp/out</output-file>",
        );
        run_user_prompt_submit_hook_inner(&input, Some(base), None).unwrap();

        let messages = tasks::get_session_user_messages(base, "unknown");
        assert!(messages.is_empty(), "task notification should not be recorded");
    }

    #[test]
    fn test_unparseable_transcript_treated_as_opening_prompt() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Point to a non-existent transcript file (triggers parse error)
        let transcript_path = base.join("nonexistent-transcript.jsonl");

        let input = UserPromptSubmitInput {
            prompt: Some("Hello".to_string()),
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };

        let output = run_user_prompt_submit_hook_inner(&input, Some(base), None).unwrap();
        // Should succeed (opening prompt path - clears existing messages)
        assert!(output.system_message.is_none());

        // Verify the message was recorded as opening prompt
        // (proves is_opening_prompt returned true via the parse error path)
        let session_id = transcript_path.to_string_lossy();
        let messages = tasks::get_session_user_messages(base, &session_id);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].context, "opening prompt");
    }
}
