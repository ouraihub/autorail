//! Launch signature: SHA-256 over the projected configuration, never over key values or tokens
//! (agent-settings 04 §4.4).

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thinkrail_core::{DefaultModel, ProjectedModel, Protocol, ProviderKeyId};

/// One projected route as it enters the signature.
#[derive(Debug)]
pub(crate) struct SignedRoute<'a> {
    pub route: &'a str,
    pub protocol: Protocol,
    /// Upstream address in direct mode; `None` in gateway mode (signed as `gateway`).
    pub base_url: Option<&'a str>,
    /// (name, value); values enter only as their SHA-256.
    pub headers: Vec<(&'a str, &'a str)>,
    pub models: &'a [ProjectedModel],
}

/// One MCP server as it enters the signature; values enter only as their SHA-256.
#[derive(Debug)]
pub(crate) struct SignedMcp<'a> {
    pub name: &'a str,
    pub kind: &'static str,
    /// Command or URL.
    pub target: &'a str,
    pub args: &'a [String],
    pub values: Vec<(&'a str, &'a str)>,
}

/// Provider revisions capture credential edits without reading keys; rotation changes neither
/// revisions nor the ordered enabled key ids.
#[derive(Debug)]
pub(crate) struct Signed<'a> {
    pub routes: Vec<SignedRoute<'a>>,
    pub keys: Vec<(&'a str, u64, Vec<&'a ProviderKeyId>)>,
    pub default_model: Option<&'a DefaultModel>,
    pub mcp: Vec<SignedMcp<'a>>,
}

pub(crate) fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hashed(pairs: &[(&str, &str)]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|(name, value)| json!([name, sha256_hex(value)]))
            .collect(),
    )
}

/// Lowercase hex SHA-256 of the canonical JSON (object keys are sorted by `serde_json`).
pub(crate) fn compute(signed: &Signed<'_>) -> String {
    let routes: Vec<Value> = signed
        .routes
        .iter()
        .map(|route| {
            json!({
                "route": route.route,
                "protocol": route.protocol,
                "baseUrl": route.base_url.unwrap_or("gateway"),
                "headers": hashed(&route.headers),
                "models": route.models,
            })
        })
        .collect();
    let keys: Vec<Value> = signed
        .keys
        .iter()
        .map(|(provider, revision, ids)| json!([provider, revision, ids]))
        .collect();
    let mcp: Vec<Value> = signed
        .mcp
        .iter()
        .map(|server| {
            json!({
                "name": server.name,
                "kind": server.kind,
                "target": server.target,
                "args": server.args,
                "values": hashed(&server.values),
            })
        })
        .collect();
    let canonical = json!({
        "routes": routes,
        "keys": keys,
        "defaultModel": signed.default_model,
        "mcp": mcp,
    });
    sha256_hex(&canonical.to_string())
}
