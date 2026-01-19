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
}

/// Parsed transcript information.
#[derive(Debug, Clone, Default)]
pub struct TranscriptInfo {
    /// The last assistant output text.
    pub last_assistant_output: Option<String>,
    /// The timestamp of the last user message.
    pub last_user_message_time: Option<DateTime<Utc>>,
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
        };
        assert!(is_user_recently_active(&info, 5));
    }

    #[test]
    fn test_is_user_recently_active_false_old() {
        let info = TranscriptInfo {
            last_assistant_output: None,
            last_user_message_time: Some(Utc::now() - chrono::Duration::minutes(10)),
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
}
