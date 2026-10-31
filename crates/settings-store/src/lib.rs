//! The settings database `thinkrail.db` (agent-settings 04 §3): providers with their keys,
//! MCP servers and model preferences. Implements `ProviderService`, `McpService` and
//! `ModelPrefService` on one Turso file.

mod db;
mod mask;
mod mcp;
mod prefs;
mod providers;
mod schema;
mod store;
mod validate;

pub use store::SqliteSettingsStore;

#[cfg(test)]
mod tests;
