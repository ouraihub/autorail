//! What one engine launch projects: providers, default model, MCP servers and the secret
//! environment that carries their values (agent-settings 04 §2.5).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    BackendKind, ChatId, Endpoint, Header, ProjectId, Protocol, ProviderId, ProviderKeyId, Result,
    Secret, ThinkingLevel,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteRequest {
    pub chat: ChatId,
    pub project: ProjectId,
    pub backend: BackendKind,
    pub new_session: bool,
}

/// Everything one engine launch projects. Empty when there are no providers and no MCP servers;
/// the engine then uses its own configuration. Every sensitive value lives only in `env`; the other
/// fields reference variable names, so writing them into generated files leaks nothing.
#[derive(Clone, Default)]
pub struct EngineRouting {
    pub providers: Vec<ProjectedProvider>,
    /// Only given for a new session.
    pub default_model: Option<DefaultModel>,
    pub mcp: Vec<ProjectedMcpServer>,
    /// (variable name, value), appended to the engine child's environment.
    pub env: Vec<(String, Secret)>,
    pub signature: String,
}

impl EngineRouting {
    /// True when nothing is projected: the adapter must behave exactly as without routing.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
            && self.mcp.is_empty()
            && self.env.is_empty()
            && self.default_model.is_none()
    }
}

impl fmt::Debug for EngineRouting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineRouting")
            .field("providers", &self.providers)
            .field("default_model", &self.default_model)
            .field("mcp", &self.mcp)
            .field("env", &self.env)
            .field("signature", &self.signature)
            .finish()
    }
}

/// (name, host-injected variable name). `name` is a header name or a variable inside the MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvBinding {
    pub name: String,
    pub var: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedProvider {
    /// `thinkrail-<first 8 of provider id>-<oc|or|am>`, the provider name inside the engine.
    pub route: String,
    pub display_name: String,
    pub protocol: Protocol,
    /// Normalized per engine rules; points at the gateway in gateway mode.
    pub base_url: String,
    /// `THINKRAIL_PROVIDER_<short id>_KEY`.
    pub key_var: String,
    /// Variables `THINKRAIL_HDR_<short id>_<n>`; empty in gateway mode (the gateway adds them).
    pub headers: Vec<EnvBinding>,
    pub models: Vec<ProjectedModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedModel {
    pub id: String,
    pub name: String,
    pub vision: bool,
    pub reasoning: bool,
    pub efforts: Vec<ThinkingLevel>,
    pub context_window: u32,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultModel {
    pub route: String,
    pub model_id: String,
    pub effort: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedMcpServer {
    /// `thinkrail-<name>`.
    pub name: String,
    pub transport: ProjectedMcpTransport,
}

/// Values were resolved by the host and placed in `EngineRouting.env` under
/// `THINKRAIL_MCP_<first 8 of server id>_<E|H><n>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProjectedMcpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<EnvBinding>,
    },
    Http {
        url: String,
        headers: Vec<EnvBinding>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    pub provider: ProviderId,
    pub provider_name: String,
    pub route: String,
    pub model: ProjectedModel,
}

pub trait ModelRouter: Send + Sync {
    /// Builds one launch's routing. Direct mode advances key rotation; gateway mode registers a
    /// fresh session token for the chat.
    fn prepare(&self, request: &RouteRequest) -> Result<EngineRouting>;
    /// The signature `prepare` would produce; read-only (no rotation, no token).
    fn signature(&self, request: &RouteRequest) -> Result<String>;
    /// On engine exit or chat close: revokes the chat's gateway token.
    fn release(&self, chat: &ChatId);
    /// Selectable models: enabled providers × enabled, visible models × protocols the engine supports.
    fn catalog(&self, backend: BackendKind) -> Result<Vec<ModelChoice>>;
}

#[derive(Clone)]
pub struct ProbeTarget {
    pub endpoint: Endpoint,
    pub headers: Vec<Header>,
    pub key: Option<Secret>,
}

impl fmt::Debug for ProbeTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<&str> = self.headers.iter().map(|h| h.name.as_str()).collect();
        f.debug_struct("ProbeTarget")
            .field("endpoint", &self.endpoint)
            .field("headers", &headers)
            .field("key", &self.key)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredModel {
    pub id: String,
    pub name: Option<String>,
    pub context_window: Option<u32>,
}

/// One key's "检测" result on one line. `error` is a redacted Chinese description: status code
/// + first 200 chars of upstream message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheck {
    pub key: ProviderKeyId,
    /// Number of models the line listed with this key; `None` when the request failed.
    pub models: Option<usize>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

pub trait ProviderProbe: Send + Sync {
    /// "同步模型": the upstream model list.
    fn list_models(&self, target: &ProbeTarget) -> Result<Vec<DiscoveredModel>>;
    /// Model check: one minimal request with the model, 15 s timeout, no retry; elapsed ms.
    fn check(&self, target: &ProbeTarget, model_id: &str) -> Result<u64>;
    /// "检测": lists the models of the provider's `protocol` line once per enabled key, explicit
    /// key, no rotation. An empty result means the provider has no enabled key.
    fn check_keys(&self, provider: &ProviderId, protocol: Protocol) -> Result<Vec<KeyCheck>>;
}

#[cfg(test)]
mod tests;
