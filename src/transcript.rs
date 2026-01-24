//! Transcript parsing for Claude Code JSONL transcripts.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A content block in an assistant message.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// A text block.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },
    /// A tool use block.
    #[serde(rename = "tool_use")]
    ToolUse {
        /// The tool name.
        name: String,
    },
    /// Any other block type.
    #[serde(other)]
    Other,
}

/// Message content - can be a string (user messages) or array of blocks (assistant messages).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple string content (user messages).
    Text(String),
    /// Array of content blocks (assistant messages).
    Blocks(Vec<ContentBlock>),
}

impl Default for MessageContent {
    fn default() -> Self {
        Self::Blocks(Vec::new())
    }
}

/// A message in the transcript.
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    /// The message content.
    #[serde(default)]
    pub content: MessageContent,
}

/// A transcript entry.
#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptEntry {
    /// The type of entry (user, assistant, system, etc.).
    #[serde(rename = "type")]
    pub entry_type: String,
    /// The timestamp of the entry.
    #[serde(default)]
    pub timestamp: Option<String>,
    /// The message content (for assistant messages).
    #[serde(default)]
    pub message: Option<Message>,
    /// Whether this entry is an API error message.
    #[serde(rename = "isApiErrorMessage", default)]
    pub is_api_error_message: bool,
}

/// Tools that are considered "read-only" and don't count as modifications.
const READ_ONLY_TOOLS: &[&str] = &["Read", "Glob", "Grep", "WebFetch", "WebSearch", "LS"];

/// Parsed transcript information.
#[derive(Debug, Clone, Default)]
pub struct TranscriptInfo {
    /// The last assistant output text.
    pub last_assistant_output: Option<String>,
    /// The timestamp of the last user message.
    pub last_user_message_time: Option<DateTime<Utc>>,
    /// Whether the transcript contains a recent API error.
    pub has_api_error: bool,
    /// Count of consecutive API errors at the end of the transcript.
    pub consecutive_api_errors: u32,
    /// Whether the transcript contains any modifying (non-Read) tool uses.
    pub has_modifying_tool_use: bool,
    /// Whether the transcript contains any modifying tool uses since the last user message.
    pub has_modifying_tool_use_since_user: bool,
    /// The first user message in the transcript.
    pub first_user_message: Option<String>,
    /// The last user message in the transcript.
    pub last_user_message: Option<String>,
}

/// Parse a transcript file and extract relevant information.
///
/// # Arguments
///
/// * `path` - Path to the JSONL transcript file.
///
/// # Returns
///
/// Parsed transcript information, or an error if parsing fails.
///
/// # Errors
///
/// Returns an error if the file doesn't exist or cannot be read.
pub fn parse_transcript(path: &Path) -> Result<TranscriptInfo> {
    if !path.exists() {
        return Err(Error::FileNotFound(path.to_path_buf()));
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut info = TranscriptInfo::default();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Try to parse as a transcript entry
        let entry: TranscriptEntry = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue, // Skip malformed lines
        };

        // Check for API error
        let is_api_error = entry.is_api_error_message || is_api_error_text(&entry);

        if is_api_error {
            info.has_api_error = true;
            info.consecutive_api_errors += 1;
        } else if entry.entry_type == "assistant" && !entry.is_api_error_message {
            // A valid (non-error) assistant message resets the consecutive counter
            info.consecutive_api_errors = 0;
        }

        match entry.entry_type.as_str() {
            "assistant" => {
                // Extract text from assistant message and check for tool uses
                if let Some(message) = &entry.message {
                    if let MessageContent::Blocks(blocks) = &message.content {
                        for block in blocks {
                            match block {
                                ContentBlock::Text { text } => {
                                    info.last_assistant_output = Some(text.clone());
                                }
                                ContentBlock::ToolUse { name } => {
                                    // Check if this is a modifying tool
                                    if !READ_ONLY_TOOLS.contains(&name.as_str()) {
                                        info.has_modifying_tool_use = true;
                                        info.has_modifying_tool_use_since_user = true;
                                    }
                                }
                                ContentBlock::Other => {}
                            }
                        }
                    }
                }
            }
            "user" => {
                // Check if this is a compaction event or system-reminder-only (not a real user message)
                let is_compaction = entry.message.as_ref().is_some_and(|m| {
                    if let MessageContent::Text(text) = &m.content {
                        is_compaction_message(text)
                    } else {
                        false
                    }
                });

                let is_system_reminder = entry.message.as_ref().is_some_and(|m| {
                    if let MessageContent::Text(text) = &m.content {
                        is_system_reminder_only(text)
                    } else {
                        false
                    }
                });

                // Only reset modifying tool use tracking for genuine user messages
                // Don't reset for compaction events or system reminders
                if !is_compaction && !is_system_reminder {
                    info.has_modifying_tool_use_since_user = false;
                }

                // Capture user message content
                if let Some(message) = &entry.message {
                    if let MessageContent::Text(text) = &message.content {
                        // Capture first user message (excluding compaction)
                        if info.first_user_message.is_none() && !is_compaction {
                            info.first_user_message = Some(text.clone());
                        }
                        // Always update last user message (excluding compaction)
                        if !is_compaction {
                            info.last_user_message = Some(text.clone());
                        }
                    }
                }
                // Parse timestamp (only for real user messages, not compaction events)
                if !is_compaction {
                    if let Some(ts_str) = &entry.timestamp {
                        if let Ok(ts) = parse_timestamp(ts_str) {
                            info.last_user_message_time = Some(ts);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(info)
}

/// Check if an entry contains API error text patterns.
fn is_api_error_text(entry: &TranscriptEntry) -> bool {
    if let Some(message) = &entry.message {
        let texts_to_check: Vec<&str> = match &message.content {
            MessageContent::Text(text) => vec![text.as_str()],
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| {
                    if let ContentBlock::Text { text } = b {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect(),
        };

        for text in texts_to_check {
            // Check for common API error patterns
            if text.contains("API Error:") && text.contains("400") {
                return true;
            }
            if text.contains("thinking")
                && text.contains("blocks")
                && text.contains("cannot be modified")
            {
                return true;
            }
            if text.contains("invalid_request_error") {
                return true;
            }
        }
    }
    false
}

/// Check if a message is a compaction event rather than a real user message.
///
/// Compaction events are system-generated messages that appear as user messages
/// but should not be counted for user activity tracking.
fn is_compaction_message(text: &str) -> bool {
    text.starts_with("This session is being continued from a previous conversation")
}

/// Check if a message is purely a system reminder with no actual user content.
///
/// System reminders are injected by the system and wrapped in `<system-reminder>` tags.
/// Messages that are ONLY system reminders (no other content) should not reset
/// the `has_modifying_tool_use_since_user` flag or be considered as user questions.
fn is_system_reminder_only(text: &str) -> bool {
    let trimmed = text.trim();
    // Check if it starts with system-reminder tag and has no content outside of tags
    if !trimmed.starts_with("<system-reminder>") {
        return false;
    }
    // Remove all system-reminder blocks and check if anything meaningful remains
    let without_reminders = remove_system_reminder_blocks(trimmed);
    without_reminders.trim().is_empty()
}

/// Remove all `<system-reminder>...</system-reminder>` blocks from text.
fn remove_system_reminder_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("<system-reminder>") {
        // Add text before the tag
        result.push_str(&remaining[..start]);

        // Find the closing tag
        let after_start = &remaining[start..];
        if let Some(end) = after_start.find("</system-reminder>") {
            remaining = &after_start[end + "</system-reminder>".len()..];
        } else {
            // No closing tag, treat rest as part of reminder (discard it)
            return result;
        }
    }
    // Add any remaining text after last reminder block
    result.push_str(remaining);
    result
}

/// Parse an ISO 8601 timestamp.
fn parse_timestamp(s: &str) -> Result<DateTime<Utc>> {
    // Handle Z suffix and +00:00 format
    let normalized = s.replace('Z', "+00:00");
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| Error::InvalidTranscript(format!("Invalid timestamp: {e}")))
}

/// Check if the user sent a message within the given number of minutes.
///
/// # Arguments
///
/// * `info` - Parsed transcript information.
/// * `recency_minutes` - The recency window in minutes.
///
/// # Returns
///
/// True if the user was active within the window.
pub fn is_user_recently_active(info: &TranscriptInfo, recency_minutes: u32) -> bool {
    let Some(last_time) = info.last_user_message_time else {
        return false;
    };

    let now = Utc::now();
    let cutoff = now - chrono::Duration::minutes(i64::from(recency_minutes));
    last_time >= cutoff
}

/// Check if a message is a simple question (single line ending with ?).
///
/// A simple question is one where:
/// - The message is a single line (no newlines except possibly trailing)
/// - The line ends with a question mark
///
/// # Arguments
///
/// * `message` - The message text to check.
///
/// # Returns
///
/// True if the message is a simple question.
pub fn is_simple_question(message: &str) -> bool {
    let trimmed = message.trim();
    // Must be non-empty, single line, and end with ?
    !trimmed.is_empty() && !trimmed.contains('\n') && trimmed.ends_with('?')
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_temp_transcript(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_parse_transcript_with_assistant_message() {
        let content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z"}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello!"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert_eq!(info.last_assistant_output, Some("Hello!".to_string()));
    }

    #[test]
    fn test_parse_transcript_with_user_timestamp() {
        let content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z"}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.last_user_message_time.is_some());
    }

    #[test]
    fn test_parse_transcript_empty() {
        let file = create_temp_transcript("");
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.last_assistant_output.is_none());
        assert!(info.last_user_message_time.is_none());
    }

    #[test]
    fn test_parse_transcript_malformed_lines() {
        let content = r#"not json
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Valid"}]}}
also not json
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert_eq!(info.last_assistant_output, Some("Valid".to_string()));
    }

    #[test]
    fn test_parse_transcript_file_not_found() {
        let result = parse_transcript(Path::new("/nonexistent/file.jsonl"));
        assert!(result.is_err());
    }

    #[test]
    fn test_is_user_recently_active_true() {
        let info = TranscriptInfo {
            last_assistant_output: None,
            last_user_message_time: Some(Utc::now()),
            has_api_error: false,
            consecutive_api_errors: 0,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        assert!(is_user_recently_active(&info, 5));
    }

    #[test]
    fn test_is_user_recently_active_false_old() {
        let info = TranscriptInfo {
            last_assistant_output: None,
            last_user_message_time: Some(Utc::now() - chrono::Duration::minutes(10)),
            has_api_error: false,
            consecutive_api_errors: 0,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        assert!(!is_user_recently_active(&info, 5));
    }

    #[test]
    fn test_is_user_recently_active_false_none() {
        let info = TranscriptInfo::default();
        assert!(!is_user_recently_active(&info, 5));
    }

    #[test]
    fn test_parse_timestamp_z_suffix() {
        let ts = parse_timestamp("2024-01-01T12:00:00Z").unwrap();
        assert_eq!(ts.year(), 2024);
    }

    #[test]
    fn test_parse_timestamp_offset() {
        let ts = parse_timestamp("2024-01-01T12:00:00+00:00").unwrap();
        assert_eq!(ts.year(), 2024);
    }

    #[test]
    fn test_last_assistant_output_takes_last() {
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "First"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Second"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Third"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert_eq!(info.last_assistant_output, Some("Third".to_string()));
    }

    #[test]
    fn test_parse_transcript_with_empty_lines() {
        // Test that empty lines are skipped (line 89)
        let content = r#"
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello"}]}}


{"type": "user", "timestamp": "2024-01-01T12:00:00Z"}

"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert_eq!(info.last_assistant_output, Some("Hello".to_string()));
        assert!(info.last_user_message_time.is_some());
    }

    #[test]
    fn test_parse_transcript_unknown_entry_type() {
        // Test that unknown entry types are ignored (line 117)
        let content = r#"{"type": "system", "message": "ignored"}
{"type": "tool_use", "tool": "test"}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Valid"}]}}
{"type": "result", "output": "ignored"}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert_eq!(info.last_assistant_output, Some("Valid".to_string()));
    }

    #[test]
    fn test_parse_transcript_assistant_with_non_text_content() {
        // Test that non-text content blocks are skipped
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "123", "name": "test", "input": {}}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "After tool"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert_eq!(info.last_assistant_output, Some("After tool".to_string()));
    }

    #[test]
    fn test_parse_transcript_user_without_timestamp() {
        // Test user message without timestamp field
        let content = r#"{"type": "user"}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Response"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.last_user_message_time.is_none());
        assert_eq!(info.last_assistant_output, Some("Response".to_string()));
    }

    #[test]
    fn test_parse_transcript_user_with_invalid_timestamp() {
        // Test user message with unparseable timestamp
        let content = r#"{"type": "user", "timestamp": "not-a-timestamp"}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Response"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // Invalid timestamp is ignored, so last_user_message_time remains None
        assert!(info.last_user_message_time.is_none());
    }

    #[test]
    fn test_parse_transcript_assistant_with_no_message() {
        // Test assistant entry without message field
        let content = r#"{"type": "assistant"}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Valid"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert_eq!(info.last_assistant_output, Some("Valid".to_string()));
    }

    #[test]
    fn test_parse_transcript_api_error_flag() {
        // Test detection of API error via isApiErrorMessage flag
        let content = r#"{"type": "assistant", "isApiErrorMessage": true, "message": {"content": [{"type": "text", "text": "API Error: 400"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_api_error);
        assert_eq!(info.consecutive_api_errors, 1);
    }

    #[test]
    fn test_parse_transcript_api_error_text_pattern() {
        // Test detection of API error via text patterns
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "API Error: 400 {\"error\": {\"type\": \"invalid_request_error\"}}"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_api_error);
    }

    #[test]
    fn test_parse_transcript_thinking_blocks_error() {
        // Test detection of thinking blocks error
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "thinking or redacted_thinking blocks cannot be modified"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_api_error);
    }

    #[test]
    fn test_parse_transcript_consecutive_api_errors() {
        // Test counting consecutive API errors
        let content = r#"{"type": "assistant", "isApiErrorMessage": true, "message": {"content": [{"type": "text", "text": "Error 1"}]}}
{"type": "assistant", "isApiErrorMessage": true, "message": {"content": [{"type": "text", "text": "Error 2"}]}}
{"type": "assistant", "isApiErrorMessage": true, "message": {"content": [{"type": "text", "text": "Error 3"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_api_error);
        assert_eq!(info.consecutive_api_errors, 3);
    }

    #[test]
    fn test_parse_transcript_api_error_reset() {
        // Test that consecutive errors reset after valid message
        let content = r#"{"type": "assistant", "isApiErrorMessage": true, "message": {"content": [{"type": "text", "text": "Error 1"}]}}
{"type": "assistant", "isApiErrorMessage": true, "message": {"content": [{"type": "text", "text": "Error 2"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Valid response"}]}}
{"type": "assistant", "isApiErrorMessage": true, "message": {"content": [{"type": "text", "text": "Error 3"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_api_error);
        // Should be 1 because the valid message reset the counter
        assert_eq!(info.consecutive_api_errors, 1);
    }

    #[test]
    fn test_parse_transcript_no_api_error() {
        // Test transcript without API errors
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Normal response"}]}}
{"type": "user", "timestamp": "2024-01-01T12:00:00Z"}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Another normal response"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(!info.has_api_error);
        assert_eq!(info.consecutive_api_errors, 0);
    }

    #[test]
    fn test_is_api_error_text_400() {
        let entry = TranscriptEntry {
            entry_type: "assistant".to_string(),
            timestamp: None,
            message: Some(Message {
                content: MessageContent::Blocks(vec![ContentBlock::Text {
                    text: "API Error: 400 something went wrong".to_string(),
                }]),
            }),
            is_api_error_message: false,
        };
        assert!(is_api_error_text(&entry));
    }

    #[test]
    fn test_is_api_error_text_thinking_blocks() {
        let entry = TranscriptEntry {
            entry_type: "assistant".to_string(),
            timestamp: None,
            message: Some(Message {
                content: MessageContent::Blocks(vec![ContentBlock::Text {
                    text: "thinking blocks in the latest assistant message cannot be modified"
                        .to_string(),
                }]),
            }),
            is_api_error_message: false,
        };
        assert!(is_api_error_text(&entry));
    }

    #[test]
    fn test_is_api_error_text_invalid_request() {
        let entry = TranscriptEntry {
            entry_type: "assistant".to_string(),
            timestamp: None,
            message: Some(Message {
                content: MessageContent::Blocks(vec![ContentBlock::Text {
                    text: "error type: invalid_request_error".to_string(),
                }]),
            }),
            is_api_error_message: false,
        };
        assert!(is_api_error_text(&entry));
    }

    #[test]
    fn test_is_api_error_text_normal_text() {
        let entry = TranscriptEntry {
            entry_type: "assistant".to_string(),
            timestamp: None,
            message: Some(Message {
                content: MessageContent::Blocks(vec![ContentBlock::Text {
                    text: "This is a normal response about error handling".to_string(),
                }]),
            }),
            is_api_error_message: false,
        };
        assert!(!is_api_error_text(&entry));
    }

    #[test]
    fn test_is_api_error_text_no_message() {
        let entry = TranscriptEntry {
            entry_type: "assistant".to_string(),
            timestamp: None,
            message: None,
            is_api_error_message: false,
        };
        assert!(!is_api_error_text(&entry));
    }

    #[test]
    fn test_is_api_error_text_non_text_content() {
        let entry = TranscriptEntry {
            entry_type: "assistant".to_string(),
            timestamp: None,
            message: Some(Message {
                content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                    name: "test".to_string(),
                }]),
            }),
            is_api_error_message: false,
        };
        assert!(!is_api_error_text(&entry));
    }

    #[test]
    fn test_parse_transcript_detects_modifying_tool_use() {
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "123"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_modifying_tool_use);
    }

    #[test]
    fn test_parse_transcript_read_only_tool_not_modifying() {
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "123"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(!info.has_modifying_tool_use);
    }

    #[test]
    fn test_parse_transcript_glob_not_modifying() {
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Glob", "id": "123"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(!info.has_modifying_tool_use);
    }

    #[test]
    fn test_parse_transcript_write_is_modifying() {
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Write", "id": "123"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_modifying_tool_use);
    }

    #[test]
    fn test_parse_transcript_bash_is_modifying() {
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Bash", "id": "123"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_modifying_tool_use);
    }

    #[test]
    fn test_parse_transcript_mixed_tools_detects_modifying() {
        // Even with read-only tools, if there's a modifying tool, it should be detected
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Glob", "id": "2"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "3"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Grep", "id": "4"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_modifying_tool_use);
    }

    #[test]
    fn test_parse_transcript_only_read_tools_not_modifying() {
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Glob", "id": "2"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Grep", "id": "3"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "WebFetch", "id": "4"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "WebSearch", "id": "5"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "LS", "id": "6"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(!info.has_modifying_tool_use);
    }

    #[test]
    fn test_modifying_tool_use_since_user_is_set() {
        // User message followed by modifying tool use
        let content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Do something"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_modifying_tool_use);
        assert!(info.has_modifying_tool_use_since_user);
    }

    #[test]
    fn test_modifying_tool_use_since_user_resets_on_user_message() {
        // Modifying tool, then user message, then read-only tool
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Check status"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "2"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // Overall flag should still be true (Edit was used somewhere)
        assert!(info.has_modifying_tool_use);
        // But since_user should be false (only Read after the user message)
        assert!(!info.has_modifying_tool_use_since_user);
    }

    #[test]
    fn test_modifying_tool_use_since_user_with_multiple_user_messages() {
        // Multiple user messages with tools in between
        let content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "First request"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "user", "timestamp": "2024-01-01T12:01:00Z", "message": {"content": "Second request"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "2"}]}}
{"type": "user", "timestamp": "2024-01-01T12:02:00Z", "message": {"content": "Third request"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Write", "id": "3"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // Overall flag should be true
        assert!(info.has_modifying_tool_use);
        // Since_user should be true (Write after the last user message)
        assert!(info.has_modifying_tool_use_since_user);
    }

    #[test]
    fn test_modifying_tool_use_since_user_no_tools_after_user() {
        // User message with no subsequent tools
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Thanks"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "You're welcome!"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert!(info.has_modifying_tool_use);
        assert!(!info.has_modifying_tool_use_since_user);
    }

    #[test]
    fn test_parse_transcript_unknown_content_block_type() {
        // Test that unknown content block types (like "thinking") are handled gracefully
        // This exercises the ContentBlock::Other arm
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "thinking", "thinking": "hmm"}, {"type": "text", "text": "Final answer"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // Should successfully parse, capturing only the text block
        assert_eq!(info.last_assistant_output, Some("Final answer".to_string()));
    }

    #[test]
    fn test_parse_transcript_tool_result_block_ignored() {
        // Test that tool_result blocks (not text or tool_use) are handled as Other
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "tool_result", "tool_use_id": "123", "content": "result"}, {"type": "text", "text": "Done"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // Should successfully parse, capturing only the text block
        assert_eq!(info.last_assistant_output, Some("Done".to_string()));
    }

    #[test]
    fn test_is_simple_question_basic() {
        assert!(is_simple_question("What does this function do?"));
        assert!(is_simple_question("How do I use this API?"));
        assert!(is_simple_question("Is this code correct?"));
    }

    #[test]
    fn test_is_simple_question_with_whitespace() {
        assert!(is_simple_question("  What is this?  "));
        assert!(is_simple_question("\nWhat is this?\n"));
    }

    #[test]
    fn test_is_simple_question_not_question() {
        assert!(!is_simple_question("Fix the bug"));
        assert!(!is_simple_question("Please refactor this code"));
        assert!(!is_simple_question("Run the tests!"));
    }

    #[test]
    fn test_is_simple_question_multiline() {
        // Multiline messages are not simple questions
        assert!(!is_simple_question("What is this?\nAnd also explain it."));
        assert!(!is_simple_question("Can you help?\nI need to refactor this code."));
    }

    #[test]
    fn test_is_simple_question_empty() {
        assert!(!is_simple_question(""));
        assert!(!is_simple_question("   "));
        // Just a question mark IS a valid simple question
        assert!(is_simple_question("?"));
    }

    #[test]
    fn test_parse_transcript_captures_first_user_message() {
        let content = r#"{"type": "user", "message": {"role": "user", "content": "What does this do?"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "It does X"}]}}
{"type": "user", "message": {"role": "user", "content": "Can you explain more?"}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // Should capture the FIRST user message, not the second
        assert_eq!(info.first_user_message, Some("What does this do?".to_string()));
    }

    #[test]
    fn test_parse_transcript_no_user_message() {
        // Transcript with only assistant messages
        let content = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        assert_eq!(info.first_user_message, None);
    }

    #[test]
    fn test_message_content_default() {
        // Test that MessageContent defaults to empty Blocks
        // This exercises the Default impl used by serde when content is missing
        let default = MessageContent::default();
        assert!(matches!(default, MessageContent::Blocks(blocks) if blocks.is_empty()));
    }

    #[test]
    fn test_parse_transcript_message_without_content() {
        // Test parsing a message where the content field is missing
        // This exercises the Default impl through serde deserialization
        let content = r#"{"type": "assistant", "message": {"role": "assistant"}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // Should parse successfully, with no output (empty content)
        assert_eq!(info.last_assistant_output, None);
    }

    #[test]
    fn test_is_compaction_message_true() {
        assert!(is_compaction_message(
            "This session is being continued from a previous conversation that ran out of context."
        ));
        assert!(is_compaction_message(
            "This session is being continued from a previous conversation. The summary below..."
        ));
    }

    #[test]
    fn test_is_compaction_message_false() {
        assert!(!is_compaction_message("Hello, how are you?"));
        assert!(!is_compaction_message("Please continue working"));
        assert!(!is_compaction_message("What is the session status?"));
    }

    #[test]
    fn test_compaction_message_not_counted_for_user_activity() {
        // Compaction event followed by real user message
        let content = r#"{"type": "user", "timestamp": "2024-01-01T10:00:00Z", "message": {"content": "This session is being continued from a previous conversation that ran out of context."}}
{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Hello!"}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // Should capture the REAL user message timestamp (12:00), not compaction (10:00)
        let ts = info.last_user_message_time.unwrap();
        assert_eq!(ts.hour(), 12);
        // First/last user message should also exclude compaction
        assert_eq!(info.first_user_message, Some("Hello!".to_string()));
        assert_eq!(info.last_user_message, Some("Hello!".to_string()));
    }

    #[test]
    fn test_compaction_only_no_real_user_messages() {
        // Only compaction event, no real user messages
        let content = r#"{"type": "user", "timestamp": "2024-01-01T10:00:00Z", "message": {"content": "This session is being continued from a previous conversation that ran out of context."}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "I understand."}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // No real user messages, so these should be None
        assert!(info.last_user_message_time.is_none());
        assert!(info.first_user_message.is_none());
        assert!(info.last_user_message.is_none());
    }

    #[test]
    fn test_compaction_between_real_messages() {
        // Real message, then compaction, then another real message
        let content = r#"{"type": "user", "timestamp": "2024-01-01T10:00:00Z", "message": {"content": "First message"}}
{"type": "user", "timestamp": "2024-01-01T11:00:00Z", "message": {"content": "This session is being continued from a previous conversation that ran out of context."}}
{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"content": "Second real message"}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();
        // First user message should be "First message"
        assert_eq!(info.first_user_message, Some("First message".to_string()));
        // Last user message should be "Second real message" (not compaction)
        assert_eq!(info.last_user_message, Some("Second real message".to_string()));
        // Timestamp should be from "Second real message" (12:00)
        let ts = info.last_user_message_time.unwrap();
        assert_eq!(ts.hour(), 12);
    }

    #[test]
    fn test_is_system_reminder_only_true() {
        assert!(is_system_reminder_only("<system-reminder>Some reminder text</system-reminder>"));
        assert!(is_system_reminder_only(
            "<system-reminder>First</system-reminder>\n<system-reminder>Second</system-reminder>"
        ));
        assert!(is_system_reminder_only("  <system-reminder>With whitespace</system-reminder>  "));
    }

    #[test]
    fn test_is_system_reminder_only_false() {
        // Regular user messages
        assert!(!is_system_reminder_only("Hello, how are you?"));
        assert!(!is_system_reminder_only("What is the status?"));

        // Message with reminder AND user content
        assert!(!is_system_reminder_only(
            "<system-reminder>Reminder</system-reminder>\nActual user question?"
        ));
        assert!(!is_system_reminder_only(
            "User content <system-reminder>Reminder</system-reminder>"
        ));

        // Doesn't start with system-reminder
        assert!(!is_system_reminder_only("Regular text <system-reminder>R</system-reminder>"));
    }

    #[test]
    fn test_remove_system_reminder_blocks() {
        assert_eq!(remove_system_reminder_blocks("<system-reminder>R</system-reminder>"), "");
        assert_eq!(
            remove_system_reminder_blocks("before<system-reminder>R</system-reminder>after"),
            "beforeafter"
        );
        assert_eq!(
            remove_system_reminder_blocks(
                "<system-reminder>A</system-reminder>middle<system-reminder>B</system-reminder>"
            ),
            "middle"
        );
        // No reminder tags
        assert_eq!(remove_system_reminder_blocks("plain text"), "plain text");
        // Malformed (unclosed) tag - treat rest as reminder
        assert_eq!(
            remove_system_reminder_blocks("before<system-reminder>unclosed content"),
            "before"
        );
    }

    #[test]
    fn test_system_reminder_does_not_reset_modifying_flag() {
        // Work session with modifications, then system reminder
        let content = r#"{"type": "user", "message": {"role": "user", "content": "Create a file"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Write", "id": "1"}]}}
{"type": "user", "message": {"role": "user", "content": "<system-reminder>Task reminder</system-reminder>"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Done"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();

        // The system reminder should NOT have reset the modifying flag
        assert!(
            info.has_modifying_tool_use_since_user,
            "System reminder should not reset has_modifying_tool_use_since_user"
        );
    }

    #[test]
    fn test_real_user_message_resets_modifying_flag() {
        // Work session with modifications, then real user question
        let content = r#"{"type": "user", "message": {"role": "user", "content": "Create a file"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Write", "id": "1"}]}}
{"type": "user", "message": {"role": "user", "content": "What did you create?"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "A file"}]}}
"#;
        let file = create_temp_transcript(content);
        let info = parse_transcript(file.path()).unwrap();

        // The real user message SHOULD have reset the modifying flag
        assert!(
            !info.has_modifying_tool_use_since_user,
            "Real user message should reset has_modifying_tool_use_since_user"
        );
    }
}
