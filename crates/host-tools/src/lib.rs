//! TODO / review tools for Agents and the stdio MCP server that exposes them.
//!
//! `tools` is the tool core (`HostTools`), `bridge` the host's authenticated named pipe
//! (`ToolBridge`), `mcp` the `mcp-host` subcommand that pi and DSH launch.

mod bridge;
mod mcp;
mod tools;
#[cfg(test)]
mod tests;

pub use bridge::ToolBridge;
pub use mcp::run_mcp_stdio;
pub use tools::HostTools;
