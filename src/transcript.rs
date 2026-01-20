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

/// A message in the transcript.
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    /// The message content (for assistant messages).
    #[serde(default)]
    pub content: Vec<ContentBlock>,
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
                // Extract text from assistant message
                if let Some(message) = &entry.message {
                    for block in &message.content {
                        if let ContentBlock::Text { text } = block {
                            info.last_assistant_output = Some(text.clone());
                        }
                    }
                }
            }
            "user" => {
                // Parse timestamp
                if let Some(ts_str) = &entry.timestamp {
                    if let Ok(ts) = parse_timestamp(ts_str) {
                        info.last_user_message_time = Some(ts);
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
        for block in &message.content {
            if let ContentBlock::Text { text } = block {
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
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
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
                content: vec![ContentBlock::Text {
                    text: "API Error: 400 something went wrong".to_string(),
                }],
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
                content: vec![ContentBlock::Text {
                    text: "thinking blocks in the latest assistant message cannot be modified"
                        .to_string(),
                }],
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
                content: vec![ContentBlock::Text {
                    text: "error type: invalid_request_error".to_string(),
                }],
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
                content: vec![ContentBlock::Text {
                    text: "This is a normal response about error handling".to_string(),
                }],
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
                content: vec![ContentBlock::ToolUse { name: "test".to_string() }],
            }),
            is_api_error_message: false,
        };
        assert!(!is_api_error_text(&entry));
    }
}
