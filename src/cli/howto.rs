//! How-to guide CLI subcommands.
//!
//! Provides commands for managing how-to guides: reusable procedures
//! that can be linked to work items.

use clap::Subcommand;

/// How-to guide management commands.
///
/// How-to guides capture reusable procedures that can be linked to work
/// items. When you retrieve a work item, linked how-tos appear with their
/// full instructions automatically.
///
/// ## When to Create a How-To
///
/// - When you discover a procedure that could be reused
///   (e.g., "How to run tests", "How to deploy")
/// - When a work item requires specific steps that should be documented
/// - When the user asks to "document how to do X"
///
/// ## Linking to Work Items
///
/// Use `work link-howto` to associate a how-to with a work item:
/// ```bash
/// claude-reliability work link-howto <work-id> --howto-id <howto-id>
/// ```
///
/// Multiple work items can share the same how-to guide.
///
/// See skill: documentation
#[derive(Subcommand, Debug, Clone)]
pub enum HowToCommand {
    /// Create a new how-to guide.
    ///
    /// Creates a guide with a title and instructions. Instructions can
    /// be detailed markdown explaining the procedure step by step.
    Create {
        /// Title for the how-to (required)
        #[arg(short, long)]
        title: String,

        /// Instructions for how to perform the work
        #[arg(short, long, default_value = "")]
        instructions: String,
    },

    /// Get a how-to guide by ID.
    ///
    /// Shows the full how-to including its title and instructions.
    Get {
        /// How-to ID
        id: String,
    },

    /// Update a how-to guide.
    ///
    /// Only specified fields are updated; others remain unchanged.
    Update {
        /// How-to ID
        id: String,

        /// New title
        #[arg(short, long)]
        title: Option<String>,

        /// New instructions
        #[arg(short, long)]
        instructions: Option<String>,
    },

    /// Delete a how-to guide.
    ///
    /// Also removes all links to work items.
    Delete {
        /// How-to ID
        id: String,
    },

    /// List all how-to guides.
    List,

    /// Search how-to guides by text.
    ///
    /// Searches titles and instructions for the query string.
    Search {
        /// Search query
        query: String,

        /// Maximum number of results
        #[arg(short, long)]
        limit: Option<usize>,
    },
}
