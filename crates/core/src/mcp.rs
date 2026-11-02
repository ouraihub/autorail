//! User-configured MCP servers, global or per project (agent-settings 04 §2.3).

use serde::{Deserialize, Serialize};

use crate::{McpServerId, ProjectId, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum McpScope {
    Global,
    Project { project: ProjectId },
}

/// `value` is plaintext or a `$VAR` / `${VAR}` reference to the host's launch environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<EnvValue>,
    },
    Http {
        url: String,
        headers: Vec<EnvValue>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInput {
    pub name: String,
    pub scope: McpScope,
    pub transport: McpTransport,
    pub enabled: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: McpServerId,
    pub revision: u64,
    pub input: McpServerInput,
}

/// `GlobalOverridden`: replaced by a same-named project entry; its `enabled` is always false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum McpOrigin {
    Global,
    Project,
    GlobalOverridden,
}

/// One entry as it applies inside a project; `origin` drives the "from global" label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveMcpServer {
    pub server: McpServer,
    pub origin: McpOrigin,
    pub enabled: bool,
}

pub trait McpService: Send + Sync {
    fn global(&self) -> Result<Vec<McpServer>>;
    /// Global + project entries with project overrides and same-name replacement applied.
    fn effective(&self, project: &ProjectId) -> Result<Vec<EffectiveMcpServer>>;
    fn create(&self, input: McpServerInput) -> Result<McpServer>;
    fn update(
        &self,
        id: &McpServerId,
        expect_revision: u64,
        input: McpServerInput,
    ) -> Result<McpServer>;
    fn delete(&self, id: &McpServerId, expect_revision: u64) -> Result<()>;
    /// A project's enable/disable of a global entry; None clears the override.
    fn set_override(
        &self,
        project: &ProjectId,
        server: &McpServerId,
        enabled: Option<bool>,
    ) -> Result<()>;
}
