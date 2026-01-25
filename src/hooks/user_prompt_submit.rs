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
    let base = base_dir.unwrap_or_else(|| Path::new("."));

    // Clear the needs validation marker - user has seen changes
    session::clear_needs_validation(base)?;

    // Clear the reflection marker so the next stop with modifying tools will prompt again
    session::clear_reflect_marker(base)?;

    // Check if this is a post-compaction scenario
    if input.is_compact_summary {
        return Ok(UserPromptSubmitOutput { system_message: Some(post_compaction_message(input)) });
    }

    // Create a work item for the user's message to verify nothing is missed
    if let Some(prompt) = &input.prompt {
        create_user_message_work_item(prompt, input.transcript_path.as_deref(), base);
    }

    Ok(UserPromptSubmitOutput::default())
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

    let task = store.create_task(&title, &description, Priority::Backlog)?;

    // Mark as requested so the agent must verify it's addressed
    store.request_tasks(&[&task.id])?;

    Ok(())
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
    fn test_user_prompt_submit_no_compaction() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        let input = UserPromptSubmitInput {
            is_compact_summary: false,
            transcript_path: None,
            prompt: None,
        };

        let output = run_user_prompt_submit_hook(&input, Some(base)).unwrap();

        assert!(output.system_message.is_none());
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
}
