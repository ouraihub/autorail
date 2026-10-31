use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{PlanScope, Result};

/// One model-callable host tool, described for MCP `tools/list` and OMP `set_host_tools`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments object.
    pub input_schema: Value,
}

/// Tool output in the shape both MCP and OMP host tools accept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostToolResult {
    pub text: String,
    pub is_error: bool,
    pub structured: Option<Value>,
}

/// The TODO / review tools an Agent can call (04 §6). Arguments and results are JSON because the
/// tool boundary is a JSON contract; implementations validate them strictly.
pub trait HostToolHandler: Send + Sync {
    fn specs(&self) -> Vec<HostToolSpec>;
    /// A tool-level failure (bad id, invariant) is `Ok` with `is_error: true` so the model sees it;
    /// `Err` is reserved for infrastructure failures.
    fn call(&self, scope: &PlanScope, name: &str, arguments: Value) -> Result<HostToolResult>;
}

/// How one chat's engine reaches the host tools: pi and DSH launch `executable mcp-host`, which
/// connects to `pipe` and authenticates with `token` (the token alone selects the chat's scope);
/// OMP calls `handler` in-process through its `host_tool_call` frames.
#[derive(Clone)]
pub struct HostToolAccess {
    /// This app's own executable (absolute).
    pub executable: std::path::PathBuf,
    pub pipe: String,
    pub token: String,
    /// Directory for the generated engine glue (pi extension, DSH patch).
    pub support_dir: std::path::PathBuf,
    pub handler: std::sync::Arc<dyn HostToolHandler>,
}

impl std::fmt::Debug for HostToolAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostToolAccess").field("pipe", &self.pipe).finish_non_exhaustive()
    }
}
