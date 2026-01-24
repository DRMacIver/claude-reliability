//! MCP server binary for task management.
//!
//! This binary runs an MCP server that exposes task management
//! functionality through stdio transport.

use claude_reliability::beads_sync;
use claude_reliability::command::RealCommandRunner;
use claude_reliability::mcp::TasksServer;
use rmcp::ServiceExt;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get project directory (current working directory)
    let project_dir = env::current_dir()?;

    // Sync beads issues to tasks (transparent - agent doesn't know about beads)
    let runner = RealCommandRunner::new();
    if let Err(e) = beads_sync::sync_beads_to_tasks(&runner, &project_dir) {
        // Log error but don't fail - beads sync is optional
        eprintln!("Warning: beads sync failed: {e}");
    }

    // Create the server for this project
    let server = TasksServer::for_project(&project_dir)?;

    // Run with stdio transport
    let service = server.serve(rmcp::transport::stdio()).await?;

    // Wait for the service to complete
    service.waiting().await?;

    Ok(())
}
