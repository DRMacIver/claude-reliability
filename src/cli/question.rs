//! Question CLI subcommands.
//!
//! Provides commands for managing questions that can block work items
//! until answered by the user.

use clap::Subcommand;

/// Question management commands.
///
/// Questions capture things that need user input or clarification.
/// When a question is linked to a work item, that item is blocked
/// until the question is answered.
///
/// ## When to Create Questions
///
/// Create questions for genuine blockers that require user input:
/// - Missing information needed to proceed
/// - Conflicting requirements that need clarification
/// - Decisions only the user can make
///
/// Do NOT create questions for:
/// - "Too much work" - just work through tasks by priority
/// - Things you can figure out yourself
///
/// ## Blocking Work Items
///
/// Link a question to a work item to block it:
/// ```bash
/// claude-reliability question create --text "Which auth provider?"
/// claude-reliability question link <work-id> --question-id <q-id>
/// ```
///
/// The work item will be blocked until the question is answered.
///
/// See skill: requirements
#[derive(Subcommand, Debug, Clone)]
pub enum QuestionCommand {
    /// Create a question requiring user input.
    ///
    /// The question will be evaluated by a sub-agent to determine if
    /// it can be auto-answered. If not, it's created and can be linked
    /// to work items to block them.
    Create {
        /// The question text (required)
        #[arg(short, long)]
        text: String,
    },

    /// Get a question by ID.
    ///
    /// Shows the question text and answer if answered.
    Get {
        /// Question ID
        id: String,
    },

    /// Provide an answer to a question.
    ///
    /// Answering a question unblocks any work items linked to it.
    Answer {
        /// Question ID
        id: String,

        /// The answer to provide
        #[arg(short, long)]
        answer: String,
    },

    /// Delete a question.
    ///
    /// Also removes all links to work items.
    Delete {
        /// Question ID
        id: String,
    },

    /// List all questions.
    List {
        /// Only show unanswered questions
        #[arg(short, long)]
        unanswered_only: bool,

        /// Maximum number of questions to return
        #[arg(short, long)]
        limit: Option<usize>,
    },

    /// Search questions by text.
    Search {
        /// Search query
        query: String,

        /// Maximum number of results
        #[arg(short, long)]
        limit: Option<usize>,
    },

    /// Link a work item to a question (blocks until answered).
    ///
    /// The work item will be blocked until this question is answered.
    Link {
        /// Work item ID to block
        work_id: String,

        /// Question ID
        #[arg(long)]
        question_id: String,
    },

    /// Remove a question link from a work item.
    Unlink {
        /// Work item ID
        work_id: String,

        /// Question ID
        #[arg(long)]
        question_id: String,
    },

    /// Get questions blocking a specific work item.
    ///
    /// Shows all unanswered questions that are blocking the given
    /// work item from being worked on.
    Blocking {
        /// Work item ID
        work_id: String,
    },
}
