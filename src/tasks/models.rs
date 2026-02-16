//! Task model types for the tasks management system.

use serde::{Deserialize, Serialize};

/// Task priority levels (0 = most important).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Priority {
    /// Critical priority - blocking issues.
    Critical = 0,
    /// High priority.
    High = 1,
    /// Medium priority (default).
    #[default]
    Medium = 2,
    /// Low priority.
    Low = 3,
    /// Backlog - future work.
    Backlog = 4,
}

impl Priority {
    /// Create a priority from a numeric value.
    ///
    /// # Errors
    ///
    /// Returns an error if the value is greater than 4.
    pub const fn from_u8(value: u8) -> Result<Self, InvalidPriority> {
        match value {
            0 => Ok(Self::Critical),
            1 => Ok(Self::High),
            2 => Ok(Self::Medium),
            3 => Ok(Self::Low),
            4 => Ok(Self::Backlog),
            _ => Err(InvalidPriority(value)),
        }
    }

    /// Get the numeric value of the priority.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Error when an invalid priority value is provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPriority(pub u8);

impl std::fmt::Display for InvalidPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid priority: {} (must be 0-4)", self.0)
    }
}

impl std::error::Error for InvalidPriority {}

/// Task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Task is open and available for work.
    #[default]
    Open,
    /// Task has been completed successfully.
    Complete,
    /// Task was abandoned and will not be completed.
    Abandoned,
    /// Task is stuck and needs help.
    Stuck,
    /// Task is blocked by dependencies.
    Blocked,
}

impl Status {
    /// Parse a status from a string.
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not a valid status.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self, InvalidStatus> {
        match s.to_lowercase().as_str() {
            "open" => Ok(Self::Open),
            "complete" => Ok(Self::Complete),
            "abandoned" => Ok(Self::Abandoned),
            "stuck" => Ok(Self::Stuck),
            "blocked" => Ok(Self::Blocked),
            _ => Err(InvalidStatus(s.to_string())),
        }
    }

    /// Get the string representation of the status.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Complete => "complete",
            Self::Abandoned => "abandoned",
            Self::Stuck => "stuck",
            Self::Blocked => "blocked",
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Error when an invalid status string is provided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidStatus(pub String);

impl std::fmt::Display for InvalidStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid status: '{}' (must be one of: open, complete, abandoned, stuck, blocked)",
            self.0
        )
    }
}

impl std::error::Error for InvalidStatus {}

/// A task in the task management system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier (slug from title + 4 random hex chars).
    pub id: String,
    /// Short title describing the task.
    pub title: String,
    /// Detailed description of the task.
    pub description: String,
    /// Priority level (0-4, lower is more important).
    pub priority: Priority,
    /// Current status.
    pub status: Status,
    /// Whether this task is currently being worked on.
    pub in_progress: bool,
    /// Whether this task was explicitly requested by the user.
    /// Requested tasks block the agent from stopping until complete or blocked on a question.
    pub requested: bool,
    /// ISO 8601 timestamp when the task was created.
    pub created_at: String,
    /// ISO 8601 timestamp when the task was last updated.
    pub updated_at: String,
}

impl Task {
    /// Check if the task is complete or abandoned (terminal states).
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        matches!(self.status, Status::Complete | Status::Abandoned)
    }
}

/// A note attached to a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// Unique identifier for the note.
    pub id: i64,
    /// ID of the task this note belongs to.
    pub task_id: String,
    /// Note content.
    pub content: String,
    /// ISO 8601 timestamp when the note was created.
    pub created_at: String,
}

/// A how-to guide for performing a particular task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HowTo {
    /// Unique identifier (slug from title + 4 random hex chars).
    pub id: String,
    /// Short title describing what this how-to explains.
    pub title: String,
    /// Detailed instructions for how to perform the task.
    pub instructions: String,
    /// ISO 8601 timestamp when the how-to was created.
    pub created_at: String,
    /// ISO 8601 timestamp when the how-to was last updated.
    pub updated_at: String,
}

/// A question requiring user input that may block tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Unique identifier (slug from text + 4 random hex chars).
    pub id: String,
    /// The question text.
    pub text: String,
    /// The answer, if provided. None means the question is unanswered.
    pub answer: Option<String>,
    /// ISO 8601 timestamp when the question was created.
    pub created_at: String,
    /// ISO 8601 timestamp when the question was answered, if applicable.
    pub answered_at: Option<String>,
}

impl Question {
    /// Check if the question has been answered.
    #[must_use]
    pub const fn is_answered(&self) -> bool {
        self.answer.is_some()
    }
}

/// A recorded user message for session tracking.
///
/// User messages are recorded during a session and included in the
/// reflection prompt to ensure all requests are addressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMessage {
    /// Unique identifier.
    pub id: i64,
    /// The user's message text.
    pub message: String,
    /// Context about the message (e.g. "opening prompt", "follow-up").
    pub context: String,
    /// Path to the conversation transcript, if available.
    pub transcript_path: Option<String>,
    /// Session identifier (typically the transcript path).
    pub session_id: String,
    /// Whether this message was recorded before a context compaction.
    pub pre_compaction: bool,
    /// ISO 8601 timestamp when the message was recorded.
    pub created_at: String,
}

/// An entry in the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unique identifier for the entry.
    pub id: i64,
    /// ISO 8601 timestamp when the operation occurred.
    pub timestamp: String,
    /// Type of operation (e.g., "create", "update", "delete").
    pub operation: String,
    /// ID of the affected task (if applicable).
    pub task_id: Option<String>,
    /// Previous value (JSON serialized, if applicable).
    pub old_value: Option<String>,
    /// New value (JSON serialized, if applicable).
    pub new_value: Option<String>,
    /// Additional details about the operation.
    pub details: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_from_u8() {
        assert_eq!(Priority::from_u8(0).unwrap(), Priority::Critical);
        assert_eq!(Priority::from_u8(1).unwrap(), Priority::High);
        assert_eq!(Priority::from_u8(2).unwrap(), Priority::Medium);
        assert_eq!(Priority::from_u8(3).unwrap(), Priority::Low);
        assert_eq!(Priority::from_u8(4).unwrap(), Priority::Backlog);
        assert!(Priority::from_u8(5).is_err());
        assert!(Priority::from_u8(255).is_err());
    }

    #[test]
    fn test_priority_as_u8() {
        assert_eq!(Priority::Critical.as_u8(), 0);
        assert_eq!(Priority::High.as_u8(), 1);
        assert_eq!(Priority::Medium.as_u8(), 2);
        assert_eq!(Priority::Low.as_u8(), 3);
        assert_eq!(Priority::Backlog.as_u8(), 4);
    }

    #[test]
    fn test_priority_default() {
        assert_eq!(Priority::default(), Priority::Medium);
    }

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Critical < Priority::High);
        assert!(Priority::High < Priority::Medium);
        assert!(Priority::Medium < Priority::Low);
        assert!(Priority::Low < Priority::Backlog);
    }

    #[test]
    fn test_invalid_priority_display() {
        let err = InvalidPriority(5);
        assert_eq!(err.to_string(), "invalid priority: 5 (must be 0-4)");
    }

    #[test]
    fn test_status_from_str() {
        assert_eq!(Status::from_str("open").unwrap(), Status::Open);
        assert_eq!(Status::from_str("OPEN").unwrap(), Status::Open);
        assert_eq!(Status::from_str("Open").unwrap(), Status::Open);
        assert_eq!(Status::from_str("complete").unwrap(), Status::Complete);
        assert_eq!(Status::from_str("abandoned").unwrap(), Status::Abandoned);
        assert_eq!(Status::from_str("stuck").unwrap(), Status::Stuck);
        assert_eq!(Status::from_str("blocked").unwrap(), Status::Blocked);
        assert!(Status::from_str("invalid").is_err());
    }

    #[test]
    fn test_status_as_str() {
        assert_eq!(Status::Open.as_str(), "open");
        assert_eq!(Status::Complete.as_str(), "complete");
        assert_eq!(Status::Abandoned.as_str(), "abandoned");
        assert_eq!(Status::Stuck.as_str(), "stuck");
        assert_eq!(Status::Blocked.as_str(), "blocked");
    }

    #[test]
    fn test_status_default() {
        assert_eq!(Status::default(), Status::Open);
    }

    #[test]
    fn test_status_display() {
        assert_eq!(Status::Open.to_string(), "open");
        assert_eq!(Status::Complete.to_string(), "complete");
    }

    #[test]
    fn test_invalid_status_display() {
        let err = InvalidStatus("foo".to_string());
        assert!(err.to_string().contains("foo"));
        assert!(err.to_string().contains("open"));
    }

    #[test]
    fn test_task_is_closed() {
        let mut task = Task {
            id: "test-1234".to_string(),
            title: "Test".to_string(),
            description: String::new(),
            priority: Priority::Medium,
            status: Status::Open,
            in_progress: false,
            requested: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        assert!(!task.is_closed());

        task.status = Status::Complete;
        assert!(task.is_closed());

        task.status = Status::Abandoned;
        assert!(task.is_closed());

        task.status = Status::Stuck;
        assert!(!task.is_closed());

        task.status = Status::Blocked;
        assert!(!task.is_closed());
    }

    #[test]
    fn test_task_serialization() {
        let task = Task {
            id: "test-1234".to_string(),
            title: "Test Task".to_string(),
            description: "A test description".to_string(),
            priority: Priority::High,
            status: Status::Open,
            in_progress: false,
            requested: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&task).unwrap();
        let parsed: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, task);
    }

    #[test]
    fn test_note_serialization() {
        let note = Note {
            id: 1,
            task_id: "test-1234".to_string(),
            content: "A note".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&note).unwrap();
        let parsed: Note = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, note);
    }

    #[test]
    fn test_audit_entry_serialization() {
        let entry = AuditEntry {
            id: 1,
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            operation: "create".to_string(),
            task_id: Some("test-1234".to_string()),
            old_value: None,
            new_value: Some(r#"{"id":"test-1234"}"#.to_string()),
            details: Some("Created task".to_string()),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: AuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn test_howto_serialization() {
        let howto = HowTo {
            id: "how-to-deploy-1234".to_string(),
            title: "How to deploy".to_string(),
            instructions: "Run the deploy script".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&howto).unwrap();
        let parsed: HowTo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, howto);
    }

    #[test]
    fn test_question_is_answered() {
        let mut question = Question {
            id: "question-1234".to_string(),
            text: "What is the API key?".to_string(),
            answer: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            answered_at: None,
        };

        assert!(!question.is_answered());

        question.answer = Some("secret123".to_string());
        question.answered_at = Some("2024-01-02T00:00:00Z".to_string());
        assert!(question.is_answered());
    }

    #[test]
    fn test_question_serialization() {
        let question = Question {
            id: "question-1234".to_string(),
            text: "What is the deployment target?".to_string(),
            answer: Some("production".to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            answered_at: Some("2024-01-02T00:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&question).unwrap();
        let parsed: Question = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, question);
    }
}
