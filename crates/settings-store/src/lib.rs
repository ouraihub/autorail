//! The settings database `thinkrail.db` (agent-settings 04 §3, 07): providers with their keys,
//! MCP servers, model preferences (incl. the default Agent) and chat bindings. Implements
//! `ProviderService`, `McpService`, `ModelPrefService` and `ChatBindingService` on one Turso file.

mod bindings;
mod db;
mod mask;
mod mcp;
mod prefs;
mod presets;
mod providers;
mod schema;
mod store;
mod validate;

pub use store::SqliteSettingsStore;

#[cfg(test)]
mod tests;
