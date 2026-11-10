//! OMP host tool calls: `host_tool_call` frames run in-process off the reader thread.

use std::thread;

use serde_json::{Value, json};
use thinkrail_core::{Error, ErrorCode as _, Result};
use tracing::debug;

use super::OmpSession;

impl OmpSession {
    /// Runs a host tool off the reader thread and answers with `host_tool_result`. Tool-level
    /// failures are results with `isError`; a withdrawn call gets no answer.
    pub(super) fn on_tool_call(&self, frame: &Value) -> Result<()> {
        let id = frame["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::protocol("host_tool_call 缺少 id"))?;
        let name = frame["toolName"].as_str().unwrap_or_default().to_owned();
        let id = id.to_owned();
        let Some(tools) = &self.tools else {
            let result = tool_result(&id, "Host tools are not enabled for this chat.", true);
            return self.live.process.send(&result);
        };
        let (handler, scope, options, cancelled) = (
            tools.handler.clone(),
            tools.scope.clone(),
            tools.options,
            tools.cancelled.clone(),
        );
        let arguments = frame.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let process = self.live.process.clone();
        let trace = process.trace.clone();
        let spawned = thread::Builder::new()
            .name("omp-host-tool".into())
            .spawn(move || {
                let _session = trace.enter("agent.host_tool.call");
                let operation = trace.operation("agent.host_tool.call", "service");
                let _entered = trace.enter_operation(&operation);
                let result = handler.call(&scope, options, &name, arguments);
                if cancelled.lock().remove(&id) {
                    operation.cancel("engine_withdrew");
                    debug!("agent.host_tool.result_withdrawn");
                    return;
                }
                let (reply, failure) = match result {
                    Ok(result) => (
                        tool_result(&id, &result.text, result.is_error),
                        result.is_error.then_some("HOST_TOOL_ERROR"),
                    ),
                    Err(error) => (
                        tool_result(&id, &error.to_string(), true),
                        Some(error.code()),
                    ),
                };
                if let Err(error) = process.send(&reply) {
                    operation.fail(error.code());
                } else if let Some(code) = failure {
                    operation.fail(code);
                } else {
                    operation.finish("SUCCESS");
                }
            });
        spawned
            .map(drop)
            .map_err(|error| Error::protocol(format!("无法运行宿主工具：{error}")))
    }
}

fn tool_result(id: &str, text: &str, is_error: bool) -> Value {
    let mut frame = json!({"type": "host_tool_result", "id": id, "result": {"content": [{"type": "text", "text": text}]}});
    if is_error {
        frame["isError"] = json!(true);
    }
    frame
}
