//! MCP server binary for task management.
//!
//! This binary runs an MCP server that exposes task management
//! functionality through stdio transport.

use claude_reliability::mcp::TasksServer;
use claude_reliability::paths;
use rmcp::ServiceExt;
use std::env;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Determine database path
    let db_path = if let Ok(path) = env::var("TASKS_DB_PATH") {
        PathBuf::from(path)
    } else {
        // Default to current directory's project-specific path in .claude-reliability/
        let cwd = env::current_dir()?;
        paths::project_db_path(&cwd)
    };

    // Create the server
    let server = TasksServer::new(&db_path)?;

    // Run with stdio transport
    let service = server.serve(rmcp::transport::stdio()).await?;

    // Wait for the service to complete
    service.waiting().await?;

    Ok(())
}
