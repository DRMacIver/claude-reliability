//! `UserPromptSubmit` hook for resetting session state.
//!
//! This hook runs when the user submits a new prompt. It resets the
//! reflection marker so that the reflection check runs again
//! on the next stop attempt after making changes.
//! It also creates a work item for each user message to verify nothing is missed.

use crate::error::Result;
use crate::session;
use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
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

    let include_binary_msg = input.is_compact_summary || is_opening_prompt(input);

    // Check if this is a post-compaction scenario
    if input.is_compact_summary {
        let binary_msg = binary_location_message();
        let compaction_msg = post_compaction_message(input);
        return Ok(UserPromptSubmitOutput {
            system_message: Some(format!("{binary_msg}\n\n{compaction_msg}")),
        });
    }

    // Single work item mode: validate the assigned item and announce it
    if let Some(swi_id) = single_work_item_id {
        return match crate::single_work_item::validate_work_item(base, swi_id) {
            Ok((id, title)) => {
                let swi_msg = format!(
                    "Single work item mode active. Assigned item: [{id}] {title}\n\
                     Only this item needs to be completed to exit."
                );
                let system_message = if include_binary_msg {
                    let binary_msg = binary_location_message();
                    format!("{binary_msg}\n\n{swi_msg}")
                } else {
                    swi_msg
                };
                Ok(UserPromptSubmitOutput { system_message: Some(system_message) })
            }
            Err(msg) => {
                let error_msg = format!("ERROR: {msg}");
                let system_message = if include_binary_msg {
                    let binary_msg = binary_location_message();
                    format!("{binary_msg}\n\n{error_msg}")
                } else {
                    error_msg
                };
                Ok(UserPromptSubmitOutput { system_message: Some(system_message) })
            }
        };
    }

    // Create a work item for the user's message to verify nothing is missed
    if let Some(prompt) = &input.prompt {
        create_user_message_work_item(prompt, input.transcript_path.as_deref(), base);
    }

    if include_binary_msg {
        Ok(UserPromptSubmitOutput { system_message: Some(binary_location_message()) })
    } else {
        Ok(UserPromptSubmitOutput { system_message: None })
    }
}

/// Create a work item for a user message so nothing gets missed.
///
/// This creates a lowest-priority requested task containing the user's message
/// and transcript link. The agent must verify it's addressed before stopping.
///
/// Failures are logged to stderr but don't block the hook.
fn create_user_message_work_item(prompt: &str, transcript_path: Option<&str>, base_dir: &Path) {
    let store = match SqliteTaskStore::for_project(base_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Warning: Failed to open task store for user message work item: {e}");
            return;
        }
    };

    create_and_request_task(&store, prompt, transcript_path);
}

/// Create and request a user message task using the given store.
///
/// Logs errors to stderr without blocking.
fn create_and_request_task(store: &dyn TaskStore, prompt: &str, transcript_path: Option<&str>) {
    if let Err(e) = create_user_message_task(store, prompt, transcript_path) {
        eprintln!("Warning: Failed to create user message work item: {e}");
    }
}

/// Build the title and description for a user message work item, then create
/// and mark it as requested.
///
/// # Errors
///
/// Returns an error if task creation or marking as requested fails.
fn create_user_message_task(
    store: &dyn TaskStore,
    prompt: &str,
    transcript_path: Option<&str>,
) -> crate::error::Result<()> {
    // Truncate long prompts for the title (keep first ~80 chars at a char boundary)
    let title_text = if prompt.len() > 80 {
        let mut end = 80;
        while end > 0 && !prompt.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &prompt[..end])
    } else {
        prompt.to_string()
    };
    // Collapse whitespace in the title
    let title_text = title_text.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = format!("User message: {title_text}");

    let mut description = format!("User sent the following message:\n\n{prompt}");
    if let Some(path) = transcript_path {
        use std::fmt::Write;
        let _ = write!(description, "\n\nTranscript: {path}");
    }

    // Add reminder guidance
    description.push_str(
        "\n\n---\n\
         If you notice patterns that would benefit from a reminder, consider \
         suggesting the user add one to `.claude-reliability/reminders.yaml`.",
    );

    let task = store.create_task(&title, &description, Priority::Backlog)?;

    // Mark as requested so the agent must verify it's addressed
    store.request_tasks(&[&task.id])?;

    Ok(())
}

/// Generate the binary location message to inject into the system message.
pub fn binary_location_message() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let binary_path = cwd.join(".claude-reliability/bin/claude-reliability");
    crate::templates::render_with_vars(
        "messages/binary_location.tera",
        &[("binary_path", &binary_path.display().to_string())],
    )
    .expect("binary_location.tera template should always render")
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
    use crate::tasks::TaskFilter;
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

        let input =
            UserPromptSubmitInput { is_compact_summary: true, transcript_path: None, prompt: None };

        let output = run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("Post-Compaction Work Item Check"));
        assert!(msg.contains("list_tasks"));
        assert!(
            msg.contains(".claude-reliability/bin/claude-reliability"),
            "compaction msg should also include binary path: {msg}"
        );
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
    fn test_user_prompt_submit_no_compaction_has_binary_path() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = UserPromptSubmitInput {
            is_compact_summary: false,
            transcript_path: None,
            prompt: None,
        };

        let output = run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        assert!(output.system_message.is_some(), "system_message should always be set");
        let msg = output.system_message.unwrap();
        assert!(
            msg.contains(".claude-reliability/bin/claude-reliability"),
            "msg should contain binary path: {msg}"
        );
        assert!(msg.contains("pre-tool-use hook"), "msg should mention hook rewriting: {msg}");
        assert!(
            msg.contains("Do NOT construct paths"),
            "msg should warn against constructing paths: {msg}"
        );
    }

    #[test]
    fn test_user_prompt_submit_no_binary_msg_after_first_prompt() {
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

        // Should NOT include binary message since this is not the opening prompt
        assert!(
            output.system_message.is_none(),
            "system_message should be None after opening prompt, got: {:?}",
            output.system_message
        );
    }

    #[test]
    fn test_user_prompt_submit_binary_msg_on_opening_prompt_with_transcript() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create a transcript file with only a user message (no assistant response yet)
        let transcript_path = dir.path().join("transcript.jsonl");
        std::fs::write(
            &transcript_path,
            r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Hello"}}
"#,
        )
        .unwrap();

        let input = UserPromptSubmitInput {
            is_compact_summary: false,
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            prompt: None,
        };

        let output = run_user_prompt_submit_hook_inner(&input, Some(base), None).unwrap();

        assert!(output.system_message.is_some(), "system_message should be set on opening prompt");
        let msg = output.system_message.unwrap();
        assert!(
            msg.contains(".claude-reliability/bin/claude-reliability"),
            "opening prompt should contain binary path: {msg}"
        );
    }

    #[test]
    fn test_single_work_item_no_binary_msg_after_first_prompt() {
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
        assert!(
            !msg.contains(".claude-reliability/bin/claude-reliability"),
            "non-opening prompt should NOT include binary path: {msg}"
        );
    }

    #[test]
    fn test_single_work_item_invalid_id_no_binary_msg_after_first_prompt() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the database so the store can be opened
        let _store = SqliteTaskStore::for_project(base).unwrap();

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
            prompt: Some("Hello".to_string()),
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let output =
            run_user_prompt_submit_hook_inner(&input, Some(base), Some("nonexistent-id")).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("ERROR"), "msg: {msg}");
        assert!(
            !msg.contains(".claude-reliability/bin/claude-reliability"),
            "non-opening prompt error should NOT include binary path: {msg}"
        );
    }

    #[test]
    fn test_user_prompt_creates_work_item() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = input_with_prompt("Fix the login bug");
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        // Verify a task was created
        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].title.contains("Fix the login bug"));
        assert!(tasks[0].description.contains("Fix the login bug"));
        assert_eq!(tasks[0].priority, Priority::Backlog);
        assert!(tasks[0].requested);
    }

    #[test]
    fn test_user_prompt_work_item_includes_transcript_path() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = UserPromptSubmitInput {
            prompt: Some("Add tests".to_string()),
            transcript_path: Some("/home/user/.claude/transcripts/abc.jsonl".to_string()),
            ..Default::default()
        };
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].description.contains("/home/user/.claude/transcripts/abc.jsonl"));
    }

    #[test]
    fn test_user_prompt_work_item_truncates_long_prompt() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let long_prompt = "a".repeat(200);
        let input = input_with_prompt(&long_prompt);
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        // Title should be truncated
        assert!(tasks[0].title.len() < 200);
        assert!(tasks[0].title.ends_with("..."));
        // But full prompt should be in description
        assert!(tasks[0].description.contains(&long_prompt));
    }

    #[test]
    fn test_user_prompt_no_work_item_without_prompt() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // No prompt field = no work item
        let input = default_input();
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_user_prompt_no_work_item_for_compaction() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Compaction messages should not create work items
        let input = UserPromptSubmitInput {
            is_compact_summary: true,
            prompt: Some("This is a compaction summary".to_string()),
            ..Default::default()
        };
        run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_create_user_message_work_item_directly() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        create_user_message_work_item("Test prompt", Some("/path/to/transcript.jsonl"), base);

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].title.starts_with("User message:"));
        assert!(tasks[0].title.contains("Test prompt"));
        assert!(tasks[0].description.contains("Test prompt"));
        assert!(tasks[0].description.contains("/path/to/transcript.jsonl"));
        assert!(tasks[0].requested);
    }

    #[test]
    fn test_create_user_message_work_item_no_transcript() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        create_user_message_work_item("Test prompt", None, base);

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(!tasks[0].description.contains("Transcript:"));
    }

    #[test]
    fn test_create_user_message_work_item_includes_reminder_guidance() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        create_user_message_work_item("Test prompt", None, base);

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].description.contains("reminders.yaml"));
        assert!(tasks[0].description.contains("reminder"));
    }

    #[test]
    fn test_create_user_message_work_item_collapses_whitespace() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        create_user_message_work_item("Fix  the\n  login   bug", None, base);

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].title.contains("Fix the login bug"));
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

    #[test]
    fn test_multibyte_char_truncation() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Build a string where position 80 falls in the middle of a multi-byte char.
        // 79 ASCII chars + "é" (2 bytes: U+00E9) + more text = position 80 is mid-char.
        let mut prompt = "a".repeat(79);
        prompt.push('é'); // 2-byte char at positions 79-80
        prompt.push_str(&"b".repeat(50));

        create_user_message_work_item(&prompt, None, base);

        let store = SqliteTaskStore::for_project(base).unwrap();
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        // Title should be truncated before the multi-byte char boundary
        assert!(tasks[0].title.ends_with("..."));
        // Full prompt should be in description
        assert!(tasks[0].description.contains(&prompt));
    }

    #[test]
    fn test_store_open_failure_does_not_block() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create .claude-reliability as a file (not directory) so the store can't be created
        let store_dir = base.join(".claude-reliability");
        std::fs::write(&store_dir, "not a directory").unwrap();

        // Should not panic or error - just logs a warning
        create_user_message_work_item("test prompt", None, base);
    }

    #[test]
    fn test_create_and_request_task_with_failing_store() {
        use crate::testing::FailingTaskStore;

        let store = FailingTaskStore::new("Simulated database error");

        // Should not panic - logs warning to stderr
        create_and_request_task(&store, "test prompt", None);
    }

    #[test]
    fn test_create_user_message_task_create_failure() {
        use crate::testing::FailingTaskStore;

        let store = FailingTaskStore::new("Simulated database error");
        let result = create_user_message_task(&store, "test prompt", None);

        assert!(result.is_err());
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
        assert!(
            msg.contains(".claude-reliability/bin/claude-reliability"),
            "single work item msg should include binary path: {msg}"
        );
    }

    #[test]
    fn test_single_work_item_invalid_id_returns_error() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create the database so the store can be opened
        let _store = SqliteTaskStore::for_project(base).unwrap();

        let input = input_with_prompt("Hello");
        let output =
            run_user_prompt_submit_hook_inner(&input, Some(base), Some("nonexistent-id")).unwrap();

        assert!(output.system_message.is_some());
        let msg = output.system_message.unwrap();
        assert!(msg.contains("ERROR"), "msg: {msg}");
        assert!(msg.contains("not found"), "msg: {msg}");
        assert!(
            msg.contains(".claude-reliability/bin/claude-reliability"),
            "error msg should include binary path: {msg}"
        );
    }

    #[test]
    fn test_single_work_item_skips_work_item_creation() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Assigned item", "Work on this", Priority::Medium).unwrap();

        let input = input_with_prompt("Please fix the bug");
        run_user_prompt_submit_hook_inner(&input, Some(base), Some(&task.id)).unwrap();

        // Only the pre-existing task should exist (no "User message:" task created)
        let tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, task.id);
    }
}
