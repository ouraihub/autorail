//! TODO / review / spec tools for Agents and the stdio MCP server that exposes them.
//!
//! `tools` is the tool core (`HostTools`), `spec` the `spec_*` spec-graph tools, `bridge` the host's
//! authenticated named pipe (`ToolBridge`), `mcp` the `mcp-host` subcommand that pi and DSH launch.

mod bridge;
mod mcp;
mod spec;
#[cfg(test)]
mod tests;
mod tools;

pub use bridge::ToolBridge;
pub use mcp::run_mcp_stdio;
pub use tools::HostTools;
