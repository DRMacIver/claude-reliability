//! Work item CLI subcommands.
//!
//! Provides commands for managing work items: create, update, list, search,
//! track progress, and manage dependencies.

use clap::Subcommand;

/// Work item management commands.
///
/// Work items are the core unit of task tracking. Each item has:
/// - A title and optional description
/// - A priority (0=critical to 4=backlog)
/// - A status (open, complete, abandoned, stuck, blocked)
/// - Optional dependencies on other work items
/// - Optional linked how-to guides and questions
///
/// ## Quick Start
///
/// ```bash
/// # Create a work item
/// claude-reliability work create --title "Fix login bug" --priority 1
///
/// # Find what to work on next
/// claude-reliability work next
///
/// # Start working on an item
/// claude-reliability work on <id>
///
/// # Mark it complete
/// claude-reliability work update <id> --status complete
/// ```
///
/// ## Priority Levels
///
/// - 0 (critical): Stop everything, fix now
/// - 1 (high): Should be done soon
/// - 2 (medium): Normal work (default)
/// - 3 (low): Nice to have
/// - 4 (backlog): Future work
///
/// See skill: task-management
#[derive(Subcommand, Debug, Clone)]
pub enum WorkCommand {
    /// Create a new work item.
    ///
    /// Creates a work item with the given title. By default, items are
    /// created with priority 2 (medium) and status "open".
    Create {
        /// Title for the work item (required)
        #[arg(short, long)]
        title: String,

        /// Description with more details
        #[arg(short, long, default_value = "")]
        description: String,

        /// Priority: 0=critical, 1=high, 2=medium, 3=low, 4=backlog
        #[arg(short, long, default_value = "2")]
        priority: u8,
    },

    /// Get a work item by ID with full details.
    ///
    /// Shows the work item including its notes, dependencies, and any
    /// linked how-to guides (with full instructions).
    Get {
        /// Work item ID
        id: String,
    },

    /// Update a work item's fields.
    ///
    /// Only specified fields are updated; others remain unchanged.
    Update {
        /// Work item ID
        id: String,

        /// New title
        #[arg(short, long)]
        title: Option<String>,

        /// New description
        #[arg(short, long)]
        description: Option<String>,

        /// New priority: 0=critical, 1=high, 2=medium, 3=low, 4=backlog
        #[arg(short, long)]
        priority: Option<u8>,

        /// New status: open, complete, abandoned, stuck, blocked
        #[arg(short, long)]
        status: Option<String>,
    },

    /// Delete a work item.
    Delete {
        /// Work item ID
        id: String,
    },

    /// List work items with optional filters.
    ///
    /// Without filters, lists all open work items. Use --ready-only to
    /// see only items that aren't blocked by dependencies.
    List {
        /// Filter by status: open, complete, abandoned, stuck, blocked
        #[arg(short, long)]
        status: Option<String>,

        /// Filter by exact priority
        #[arg(short, long)]
        priority: Option<u8>,

        /// Filter by maximum priority (inclusive)
        #[arg(long)]
        max_priority: Option<u8>,

        /// Only show items ready to work on (not blocked)
        #[arg(short, long)]
        ready_only: bool,

        /// Maximum number of items to return
        #[arg(short, long)]
        limit: Option<usize>,

        /// Number of items to skip
        #[arg(long)]
        offset: Option<usize>,
    },

    /// Search work items by text.
    ///
    /// Searches titles, descriptions, and notes for the query string.
    Search {
        /// Search query
        query: String,

        /// Maximum number of results
        #[arg(short, long)]
        limit: Option<usize>,
    },

    /// Pick the next work item to work on.
    ///
    /// Automatically selects a random item from the highest-priority
    /// unblocked items. Use this instead of manually scanning the list.
    ///
    /// After getting a suggestion, use `work on <id>` to mark it as
    /// in-progress before starting work.
    Next,

    /// Mark a work item as in-progress.
    ///
    /// Use this before making code changes to track what you're working
    /// on. Only one item should be in-progress at a time.
    On {
        /// Work item ID
        id: String,
    },

    /// Mark work items as requested by the user.
    ///
    /// Requested items must be completed before the agent can stop.
    /// Use this to ensure specific items get done in the current session.
    Request {
        /// Work item IDs to mark as requested
        ids: Vec<String>,
    },

    /// Mark all open work items as requested.
    ///
    /// Enables "request mode" - all current and future items are marked
    /// as requested until the agent successfully stops.
    #[command(name = "request-all")]
    RequestAll,

    /// Get incomplete requested work items.
    ///
    /// Shows items the user has requested that must be completed
    /// (or blocked on a question) before the agent can stop.
    Incomplete,

    /// Get work items blocked by unanswered questions.
    ///
    /// Shows items that are only blocked by questions (not by other
    /// dependencies). Answering these questions will unblock the items.
    Blocked,

    /// Add a dependency between work items.
    ///
    /// The first item will depend on the second - it cannot be worked
    /// on until the dependency is complete.
    #[command(name = "add-dep")]
    AddDep {
        /// Work item ID that will have the dependency
        id: String,

        /// Work item ID that must be completed first
        #[arg(long)]
        depends_on: String,
    },

    /// Remove a dependency between work items.
    #[command(name = "remove-dep")]
    RemoveDep {
        /// Work item ID that has the dependency
        id: String,

        /// Work item ID to remove as dependency
        #[arg(long)]
        depends_on: String,
    },

    /// Add a note to a work item.
    ///
    /// Notes capture progress, findings, or context about the work.
    /// They're displayed when you retrieve the work item.
    #[command(name = "add-note")]
    AddNote {
        /// Work item ID
        id: String,

        /// Note content
        #[arg(short, long)]
        content: String,
    },

    /// Get notes for a work item.
    Notes {
        /// Work item ID
        id: String,

        /// Maximum number of notes to return
        #[arg(short, long)]
        limit: Option<usize>,
    },

    /// Link a work item to a how-to guide.
    ///
    /// The how-to's instructions will appear when you retrieve the
    /// work item, providing guidance on how to complete it.
    #[command(name = "link-howto")]
    LinkHowTo {
        /// Work item ID
        id: String,

        /// How-to guide ID
        #[arg(long)]
        howto_id: String,
    },

    /// Remove a how-to guide link from a work item.
    #[command(name = "unlink-howto")]
    UnlinkHowTo {
        /// Work item ID
        id: String,

        /// How-to guide ID
        #[arg(long)]
        howto_id: String,
    },
}
