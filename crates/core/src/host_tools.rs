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

/// What one engine launch is offered (agent-settings 08 §5.2), fixed for that launch: changing
/// the setting restarts the engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostToolOptions {
    /// `request_review` is listed (设置 → 模型 → Agent 触发的评审).
    pub request_review: bool,
    /// The verdict comes back as a later message instead of the tool result (DSH, whose MCP
    /// calls time out after 60 s).
    pub async_review: bool,
}

/// The TODO / review tools an Agent can call (04 §6). Arguments and results are JSON because the
/// tool boundary is a JSON contract; implementations validate them strictly.
pub trait HostToolHandler: Send + Sync {
    fn specs(&self, options: HostToolOptions) -> Vec<HostToolSpec>;
    /// A tool-level failure (bad id, invariant) is `Ok` with `is_error: true` so the model sees it;
    /// `Err` is reserved for infrastructure failures. May block for a long time (`request_review`
    /// waits for its verdict); callers run it off their reader threads.
    fn call(
        &self,
        scope: &PlanScope,
        options: HostToolOptions,
        name: &str,
        arguments: Value,
    ) -> Result<HostToolResult>;
    /// The engine withdrew a running call of this chat: a waiting `request_review` ends its review.
    fn cancel(&self, scope: &PlanScope) {
        let _ = scope;
    }
}

/// How one chat's engine reaches the host tools: pi and DSH launch `executable mcp-host`, which
/// connects to `pipe` and authenticates with `token` (the token alone selects the chat's scope and
/// options); OMP calls `handler` in-process through its `host_tool_call` frames.
#[derive(Clone)]
pub struct HostToolAccess {
    /// This app's own executable (absolute).
    pub executable: std::path::PathBuf,
    pub pipe: String,
    pub token: String,
    pub handler: std::sync::Arc<dyn HostToolHandler>,
    pub options: HostToolOptions,
}

impl std::fmt::Debug for HostToolAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostToolAccess")
            .field("pipe", &self.pipe)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}
