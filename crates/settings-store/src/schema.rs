//! Schema version and table definitions (agent-settings 04 §3.2, 07 §4).

/// Version written to `schema_meta`. Only this version opens; any other is refused as `Corrupt`
/// (no upgrade steps: a development database is deleted and recreated).
pub(crate) const SCHEMA_VERSION: i64 = 4;

/// Every table of [`SCHEMA_VERSION`], created in order on an empty database.
pub(crate) const TABLES: &[&str] = &[
    "CREATE TABLE schema_meta (version INTEGER NOT NULL)",
    "CREATE TABLE providers (
        id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, default_protocol TEXT NOT NULL,
        headers TEXT NOT NULL,
        access TEXT NOT NULL, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
        revision INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
        preset_id TEXT UNIQUE)",
    "CREATE TABLE provider_endpoints (
        provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
        protocol TEXT NOT NULL, base_url TEXT NOT NULL, PRIMARY KEY (provider_id, protocol))",
    "CREATE TABLE provider_models (
        provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
        model_id TEXT NOT NULL, name TEXT NOT NULL, protocols TEXT NOT NULL,
        context_window INTEGER, max_tokens INTEGER, vision INTEGER NOT NULL, reasoning INTEGER NOT NULL,
        tools INTEGER NOT NULL, efforts TEXT NOT NULL, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
        model_type TEXT NOT NULL DEFAULT 'chat',
        PRIMARY KEY (provider_id, model_id))",
    "CREATE TABLE provider_keys (
        id TEXT PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
        key TEXT NOT NULL, label TEXT, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
        UNIQUE (provider_id, key))",
    "CREATE TABLE model_prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "CREATE TABLE hidden_models (
        provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE, model_id TEXT NOT NULL,
        PRIMARY KEY (provider_id, model_id))",
    "CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY, name TEXT NOT NULL,
        project_id TEXT NOT NULL,
        transport TEXT NOT NULL,
        enabled INTEGER NOT NULL, description TEXT, revision INTEGER NOT NULL, sort INTEGER NOT NULL,
        UNIQUE (project_id, name))",
    "CREATE TABLE mcp_overrides (
        project_id TEXT NOT NULL, server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
        enabled INTEGER NOT NULL, PRIMARY KEY (project_id, server_id))",
    "CREATE TABLE chat_bindings (
        chat_id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, backend TEXT NOT NULL,
        native_session_key TEXT NOT NULL, created_at INTEGER NOT NULL)",
];
