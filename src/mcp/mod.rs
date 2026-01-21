//! MCP (Model Context Protocol) server implementations.
//!
//! This module provides MCP servers for exposing functionality to Claude Code.

#[cfg(feature = "mcp")]
pub mod tasks_server;

#[cfg(feature = "mcp")]
pub use tasks_server::TasksServer;
