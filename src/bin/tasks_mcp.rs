//! MCP server binary for task management.
//!
//! This binary runs an MCP server that exposes task management
//! functionality through stdio transport.

use claude_reliability::beads_sync;
use claude_reliability::command::RealCommandRunner;
use claude_reliability::mcp::TasksServer;
use claude_reliability::{mcp_health, mcp_logging};
use rmcp::ServiceExt;
use std::sync::Arc;
use tokio::sync::Notify;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project_dir = std::env::current_dir()?;

    // Initialize logging first (writes to .claude-reliability/mcp.log)
    if let Err(e) = mcp_logging::init(&project_dir) {
        eprintln!("Warning: MCP logging init failed: {e}");
    }
    mcp_logging::install_panic_hook();

    // Sync beads issues to tasks (optional, logs warning if fails)
    let runner = RealCommandRunner::new();
    if let Err(e) = beads_sync::sync_beads_to_tasks(&runner, &project_dir) {
        mcp_logging::log_warning(&format!("beads sync failed: {e}"));
    }

    // Start heartbeat task for health monitoring
    let shutdown = Arc::new(Notify::new());
    mcp_health::start_heartbeat_task(project_dir.clone(), Arc::clone(&shutdown));
    mcp_logging::log_event("Heartbeat task started");

    // Create and run the MCP server
    let server = TasksServer::for_project(&project_dir)?;
    mcp_logging::log_event("MCP server created, starting stdio transport");
    let service = server.serve(rmcp::transport::stdio()).await?;
    mcp_logging::log_event("MCP server running");
    service.waiting().await?;

    // Clean up on shutdown
    mcp_logging::log_shutdown(None);
    shutdown.notify_one();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    Ok(())
}
