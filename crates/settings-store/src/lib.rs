//! The settings database `thinkrail.db` (agent-settings 04 §3, 07; native-multi-agent 06):
//! providers with their keys, MCP servers, model preferences (incl. the default Agent), chat
//! bindings and host preferences. Implements `ProviderService`, `McpService`, `ModelPrefService`,
//! `ChatBindingService` and `AppPrefService` on one Turso file.

mod app_prefs;
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
