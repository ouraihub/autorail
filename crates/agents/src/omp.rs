//! Official OMP CLI over `--mode rpc-ui`: protocol v2 (chunked frames), `prompt_result` /
//! `session_settled` completion, `ask` questionnaires and `always-ask` tool approval.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use thinkrail_core::{
    AgentRuntime, AgentSession, BackendKind, Capabilities, Error, ErrorCode as _, HostToolHandler, InteractionAnswer,
    NativeSessionKey, PlanScope, Result, SessionSink, SessionSnapshot, StartSession, ThinkingLevel,
};
use tracing::{debug, info, warn};

use crate::{
    framing::Chunks,
    host,
    omp_mcp,
    process::{self, LaunchSpec, Process},
    projection::Projection,
    projection_engine,
    rpc::{self, Dialogs, Stream},
    session::{self, COMMAND, HANDSHAKE, Live, engine_error, new_id, session_error},
    workflow,
};

/// OMP physical line limit; logical frames up to [`crate::framing::MAX_FRAME`] arrive as v2 chunks.
const LINE_LIMIT: usize = 1024 * 1024;
const PAGE: u64 = 256;
const MAX_HISTORY: usize = 2000;
const READY: &str = "ready";
const NO_MODEL: (&str, &str) =
    ("No default model selected", "OMP 没有可用的默认模型：请在 OMP 配置目录中配置模型（例如 models.yml）后重试");
const CAPABILITIES: Capabilities =
    Capabilities { resume: true, history_replay: true, token_deltas: true, questions: true, tool_permissions: true, model_switch: true };

/// Launches OMP `--mode rpc-ui --approval-mode always-ask` per chat.
#[derive(Debug, Default)]
pub struct OmpRuntime;

impl OmpRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl AgentRuntime for OmpRuntime {
    fn backend(&self) -> BackendKind {
        BackendKind::Omp
    }

    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    fn start(&self, request: StartSession, sink: Arc<dyn SessionSink>) -> Result<Arc<dyn AgentSession>> {
        session::validate(&request, BackendKind::Omp)?;
        let command = process::command(&launch_spec(&request)?)?;
        let (process, stdout) = Process::spawn(command, LINE_LIMIT)?;
        let projection = Projection::new(session::identity(&request, CAPABILITIES), sink);
        let live = Live::new(process, projection, BackendKind::Omp);
        let tools = request.host_tools.as_ref().map(|access| HostTools {
            handler: access.handler.clone(),
            scope: PlanScope { workspace: request.workspace.clone(), chat: request.chat.clone() },
            definitions: host::omp_definitions(access),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
        });
        let omp = Arc::new(OmpSession {
            live: live.clone(),
            dialogs: Dialogs::default(),
            tools,
            interrupted: AtomicBool::new(false),
        });
        let ready = live.expect(READY).map_err(|error| live.fail_start(error, None))?;
        let reader = omp.clone();
        let (mut chunks, mut stream) = (Chunks::default(), Stream::default());
        session::spawn_reader(live.clone(), stdout, LINE_LIMIT, move |frame| {
            match chunks.accept(frame)? {
                Some(frame) => reader.on_frame(&mut stream, frame),
                None => Ok(()),
            }
        })?;
        let deadline = Instant::now() + HANDSHAKE;
        omp.handshake(&ready, deadline).map_err(|error| live.fail_start(error, Some(NO_MODEL)))?;
        info!(chat = %request.chat, backend = "omp", "agent session started");
        Ok(omp)
    }
}

/// Host arguments and environment of one OMP launch: the extension (`-e`: workflow rules and
/// providers), the overlay (`--config`: bundled skills, MCP, default model) and the `thinkrail-`
/// entries of the config root's (default `~/.omp/agent`) `mcp.json`. Host tools travel over RPC
/// (`set_host_tools`), so they add neither arguments nor variables.
fn launch_spec(request: &StartSession) -> Result<LaunchSpec<'_>> {
    let mut host_args: Vec<String> =
        ["--mode", "rpc-ui", "--approval-mode", "always-ask"].into_iter().map(Into::into).collect();
    omp_mcp::sync(&process::config_dir(request.config_root.as_deref(), ".omp/agent")?, &request.routing.mcp)?;
    let skills = workflow::stage_skills(&request.support_dir)?;
    let extension = projection_engine::omp_extension(&request.routing);
    let path = host::write_support(&request.support_dir, projection_engine::OMP_EXTENSION_FILE, &extension)?;
    host_args.extend(["-e".to_owned(), path.display().to_string()]);
    let overlay = projection_engine::omp_overlay(&request.routing, &skills);
    let path = host::write_support(&request.support_dir, projection_engine::OMP_OVERLAY_FILE, &overlay)?;
    host_args.extend(["--config".to_owned(), path.display().to_string()]);
    if let Some(key) = &request.resume {
        host_args.extend(["--session".to_owned(), key.as_str().to_owned()]);
    }
    Ok(LaunchSpec {
        launch: &request.launch,
        config_root: request.config_root.as_deref(),
        host_args,
        cwd: request.cwd.clone(),
        config_var: "PI_CODING_AGENT_DIR",
        host_env: process::routing_env(&request.routing).collect(),
    })
}

/// The host tools of one OMP chat: called in-process from `host_tool_call` frames.
struct HostTools {
    handler: Arc<dyn HostToolHandler>,
    scope: PlanScope,
    definitions: Value,
    /// Calls OMP withdrew; their late results are dropped.
    cancelled: Arc<Mutex<HashSet<String>>>,
}

struct OmpSession {
    live: Arc<Live>,
    dialogs: Dialogs,
    tools: Option<HostTools>,
    /// The last turn may have been stopped; the next prompt checks for a request to withdraw.
    interrupted: AtomicBool,
}

impl OmpSession {
    fn handshake(&self, ready: &std::sync::mpsc::Receiver<Result<Value>>, deadline: Instant) -> Result<()> {
        let frame = self.live.wait(READY, ready, session::left(deadline))?;
        let v2 = frame["supportedProtocolVersions"].as_array().is_some_and(|v| v.iter().any(|v| v == 2));
        if !v2 {
            return Err(Error::protocol("这个 OMP 版本不支持 RPC 协议 v2，请升级"));
        }
        self.call("negotiate_protocol", json!({"protocolVersion": 2}), session::left(deadline))?;
        self.call("set_event_filter", json!({"events": null, "messageUpdates": "delta"}), session::left(deadline))?;
        self.call("set_ask_dialog", json!({"enabled": true}), session::left(deadline))?;
        if let Some(tools) = &self.tools {
            self.call("set_host_tools", json!({"tools": tools.definitions}), session::left(deadline))?;
        }
        let state = self.call("get_state", json!({}), session::left(deadline))?;
        let key = state["sessionFile"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::protocol("OMP 没有返回会话文件；请从启动参数中删除 --no-session"))?;
        let history = self.history(deadline)?;
        let mut p = self.live.state.lock();
        p.set_key(NativeSessionKey::from(key));
        p.set_model(rpc::model(&state));
        rpc::replay(&mut p, &history, 0);
        if history.last().is_some_and(|m| m["role"] == "assistant" && m["stopReason"] == "aborted") {
            self.interrupted.store(true, Ordering::Release);
        }
        let settled = state["isSettled"] != false && state["isStreaming"] != true;
        p.set_running(!settled);
        if settled {
            p.fail_unfinished_tools();
        }
        p.publish();
        Ok(())
    }

    fn history(&self, deadline: Instant) -> Result<Vec<Value>> {
        let mut messages = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = json!({"limit": PAGE});
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            let page = self.call("get_messages_page", params, session::left(deadline))?;
            messages.extend(page["messages"].as_array().into_iter().flatten().cloned());
            if messages.len() > MAX_HISTORY {
                let overflow = messages.len() - MAX_HISTORY;
                messages.drain(..overflow);
            }
            match page["nextCursor"].as_str() {
                Some(next) if !next.is_empty() => cursor = Some(next.to_owned()),
                _ => return Ok(messages),
            }
        }
    }

    /// OMP drops an aborted reply with no text or tool call from the model context but keeps its
    /// request, so the model would still answer a request the user stopped. Moving the leaf back
    /// before that request withdraws it; the host transcript keeps showing it.
    fn withdraw_stopped_request(&self) {
        if !self.interrupted.swap(false, Ordering::AcqRel) {
            return;
        }
        let withdrawn = self.call("get_entries", json!({}), COMMAND).and_then(|data| {
            let Some(request) = stopped_request(&data["entries"], data["leafId"].as_str()) else { return Ok(false) };
            self.call("navigate_tree", json!({"entryId": request, "summarize": false}), COMMAND).map(|_| true)
        });
        match withdrawn {
            Ok(true) => info!("stopped request withdrawn from the OMP context"),
            Ok(false) => {}
            Err(error) => warn!(code = error.code(), "cannot withdraw the stopped request"),
        }
    }

    fn call(&self, kind: &str, mut fields: Value, timeout: Duration) -> Result<Value> {
        let id = new_id();
        fields["type"] = json!(kind);
        fields["id"] = json!(id);
        self.live.request(id, &fields, timeout)
    }

    fn on_frame(&self, stream: &mut Stream, frame: Value) -> Result<()> {
        let kind = frame["type"].as_str().ok_or_else(|| Error::protocol("OMP 帧缺少 type"))?;
        match kind {
            READY => {
                self.live.resolve(READY, Ok(frame));
                Ok(())
            }
            "response" => {
                self.on_response(frame);
                Ok(())
            }
            "extension_ui_request" => self.dialogs.on_request(&self.live, &frame),
            "host_tool_call" => self.on_tool_call(&frame),
            "host_tool_cancel" => {
                if let (Some(tools), Some(target)) = (&self.tools, frame["targetId"].as_str()) {
                    tools.cancelled.lock().insert(target.to_owned());
                }
                Ok(())
            }
            "rpc_frame_error" | "extension_error" => {
                let error = engine_error(frame["error"].as_str().unwrap_or("OMP 协议事件失败"));
                self.live.state.lock().error(session_error(&error));
                Ok(())
            }
            "prompt_result" => {
                let mut p = self.live.state.lock();
                if frame["status"] == "error" {
                    let message = frame["error"]["message"].as_str().unwrap_or("本轮失败");
                    p.error(session_error(&engine_error(message)));
                }
                if frame["sessionSettled"] == true || frame["agentInvoked"] == false {
                    p.fail_unfinished_tools();
                    p.set_running(false);
                }
                Ok(())
            }
            _ => {
                let mut p = self.live.state.lock();
                match kind {
                    "agent_start" => p.set_running(true),
                    "thinking_level_changed" | "config_update" => rpc::model_event(&mut p, &frame),
                    "session_settled" => {
                        p.fail_unfinished_tools();
                        p.set_running(false);
                    }
                    "message_start" => stream.message_start(&mut p, &frame),
                    "message_update" => stream.message_update(&mut p, &frame),
                    "message_end" => stream.message_end(&mut p, &frame),
                    "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                        rpc::tool_event(&mut p, &frame);
                    }
                    _ => {}
                }
                Ok(())
            }
        }
    }

    /// Runs a host tool off the reader thread and answers with `host_tool_result`. Tool-level
    /// failures are results with `isError`; a withdrawn call gets no answer.
    fn on_tool_call(&self, frame: &Value) -> Result<()> {
        let id = frame["id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| Error::protocol("host_tool_call 缺少 id"))?;
        let name = frame["toolName"].as_str().unwrap_or_default().to_owned();
        let id = id.to_owned();
        let Some(tools) = &self.tools else {
            let result = tool_result(&id, "Host tools are not enabled for this chat.", true);
            return self.live.process.send(&result);
        };
        let (handler, scope, cancelled) = (tools.handler.clone(), tools.scope.clone(), tools.cancelled.clone());
        let arguments = frame.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let process = self.live.process.clone();
        let spawned = thread::Builder::new().name("omp-host-tool".into()).spawn(move || {
            let reply = match handler.call(&scope, &name, arguments) {
                Ok(result) => tool_result(&id, &result.text, result.is_error),
                Err(error) => {
                    warn!(code = error.code(), tool = %name, "host tool failed");
                    tool_result(&id, &error.to_string(), true)
                }
            };
            if cancelled.lock().remove(&id) {
                debug!(tool = %name, "host tool result dropped: call was cancelled");
                return;
            }
            if let Err(error) = process.send(&reply) {
                debug!(%error, "host tool result not delivered");
            }
        });
        spawned.map(drop).map_err(|error| Error::protocol(format!("无法运行宿主工具：{error}")))
    }

    fn on_response(&self, frame: Value) {
        let Some(id) = frame["id"].as_str() else {
            if frame["success"] == false {
                let error = engine_error(frame["error"].as_str().unwrap_or("命令失败"));
                self.live.state.lock().error(session_error(&error));
            }
            return;
        };
        let result = if frame["success"] == true {
            Ok(frame.get("data").cloned().unwrap_or(Value::Null))
        } else {
            Err(engine_error(frame["error"].as_str().unwrap_or("命令失败")))
        };
        if let Some(Err(error)) = self.live.resolve(id, result) {
            self.live.state.lock().error(session_error(&error));
        }
    }
}

impl AgentSession for OmpSession {
    fn snapshot(&self) -> SessionSnapshot {
        self.live.state.lock().snapshot()
    }

    fn prompt(&self, text: &str) -> Result<()> {
        self.live.ensure_idle()?;
        self.withdraw_stopped_request();
        self.live.state.lock().set_running(true);
        if let Err(error) = self.call("prompt", json!({"message": text}), COMMAND) {
            self.live.state.lock().set_running(false);
            return Err(error);
        }
        Ok(())
    }

    fn stop(&self) -> Result<()> {
        self.live.ensure_open()?;
        self.interrupted.store(true, Ordering::Release);
        self.call("abort", json!({}), COMMAND).map(drop)
    }

    fn respond(&self, request_id: &str, answer: InteractionAnswer) -> Result<()> {
        self.dialogs.respond(&self.live, request_id, &answer)
    }

    fn dispose(&self) -> Result<()> {
        if self.live.begin_dispose() {
            self.dialogs.cancel_all(&self.live);
            self.live.shutdown();
        }
        Ok(())
    }

    fn set_model(&self, provider: &str, model_id: &str, effort: Option<ThinkingLevel>) -> Result<()> {
        rpc::switch_model(&self.live, |kind, fields| self.call(kind, fields, COMMAND), provider, model_id, effort)
    }
}

fn tool_result(id: &str, text: &str, is_error: bool) -> Value {
    let mut frame = json!({"type": "host_tool_result", "id": id, "result": {"content": [{"type": "text", "text": text}]}});
    if is_error {
        frame["isError"] = json!(true);
    }
    frame
}

/// The earliest user entry of the trailing run of requests whose only replies are aborted without
/// text or tool calls, walking the active path back from `leaf`. Those requests reach the model
/// unanswered.
fn stopped_request(entries: &Value, leaf: Option<&str>) -> Option<String> {
    let by_id: HashMap<&str, &Value> =
        entries.as_array()?.iter().filter_map(|e| e["id"].as_str().map(|id| (id, e))).collect();
    let mut cursor = leaf;
    let (mut stopped, mut earliest) = (false, None);
    while let Some(entry) = cursor.and_then(|id| by_id.get(id)) {
        if entry["type"] == "message" {
            let message = &entry["message"];
            match message["role"].as_str() {
                Some("assistant") if message["stopReason"] == "aborted" && !answers(&message["content"]) => {
                    stopped = true;
                }
                Some("user") if stopped => {
                    earliest = entry["id"].as_str().map(str::to_owned);
                    stopped = false;
                }
                _ => break,
            }
        }
        cursor = entry["parentId"].as_str();
    }
    earliest
}

fn answers(content: &Value) -> bool {
    content.as_array().is_some_and(|blocks| {
        blocks.iter().any(|b| b["type"] == "toolCall" || (b["type"] == "text" && b["text"].as_str().is_some_and(|t| !t.trim().is_empty())))
    })
}

#[cfg(test)]
mod tests;
