//! Official OMP CLI over `--mode rpc-ui`: protocol v2 (chunked frames), `prompt_result` /
//! `session_settled` completion, `ask` questionnaires and `always-ask` tool approval.

use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use thinkrail_core::{
    AgentRuntime, AgentSession, BackendKind, Capabilities, Error, HostToolHandler,
    InteractionAnswer, NativeSessionKey, PlanScope, Result, SessionSink, SessionSnapshot,
    StartSession, ThinkingLevel,
};
use tracing::info;

use crate::{
    framing::Chunks,
    host,
    process::{self, Process},
    projection::Projection,
    rpc::{self, Dialogs, Stream},
    session::{self, COMMAND, HANDSHAKE, Live, engine_error, new_id, session_error},
};

mod host_tools;
mod launch;
mod withdraw;

use launch::launch_spec;

/// OMP physical line limit; logical frames up to [`crate::framing::MAX_FRAME`] arrive as v2 chunks.
const LINE_LIMIT: usize = 1024 * 1024;
const PAGE: u64 = 256;
const MAX_HISTORY: usize = 2000;
const READY: &str = "ready";
const NO_MODEL: (&str, &str) = (
    "No default model selected",
    "OMP 没有可用的默认模型：请在 OMP 配置目录中配置模型（例如 models.yml）后重试",
);
const CAPABILITIES: Capabilities = Capabilities {
    resume: true,
    history_replay: true,
    token_deltas: true,
    questions: true,
    tool_permissions: true,
    model_switch: true,
    compact: false,
    rename: false,
};

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

    fn start(
        &self,
        request: StartSession,
        sink: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>> {
        session::validate(&request, BackendKind::Omp)?;
        let command = process::command(&launch_spec(&request)?)?;
        let (process, stdout) = Process::spawn(command, LINE_LIMIT)?;
        let projection = Projection::new(session::identity(&request, CAPABILITIES), sink);
        let live = Live::new(process, projection, BackendKind::Omp);
        let tools = request.host_tools.as_ref().map(|access| HostTools {
            handler: access.handler.clone(),
            scope: PlanScope {
                workspace: request.workspace.clone(),
                chat: request.chat.clone(),
            },
            definitions: host::omp_definitions(access),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
        });
        let omp = Arc::new(OmpSession {
            live: live.clone(),
            dialogs: Dialogs::default(),
            tools,
            interrupted: AtomicBool::new(false),
        });
        let ready = live
            .expect(READY)
            .map_err(|error| live.fail_start(error, None))?;
        let reader = omp.clone();
        let (mut chunks, mut stream) = (Chunks::default(), Stream::default());
        session::spawn_reader(live.clone(), stdout, LINE_LIMIT, move |frame| match chunks
            .accept(frame)?
        {
            Some(frame) => reader.on_frame(&mut stream, frame),
            None => Ok(()),
        })?;
        let deadline = Instant::now() + HANDSHAKE;
        omp.handshake(&ready, deadline)
            .map_err(|error| live.fail_start(error, Some(NO_MODEL)))?;
        info!(chat = %request.chat, backend = "omp", "agent session started");
        Ok(omp)
    }
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
    fn handshake(
        &self,
        ready: &std::sync::mpsc::Receiver<Result<Value>>,
        deadline: Instant,
    ) -> Result<()> {
        let frame = self.live.wait(READY, ready, session::left(deadline))?;
        let v2 = frame["supportedProtocolVersions"]
            .as_array()
            .is_some_and(|v| v.iter().any(|v| v == 2));
        if !v2 {
            return Err(Error::protocol("这个 OMP 版本不支持 RPC 协议 v2，请升级"));
        }
        self.call(
            "negotiate_protocol",
            json!({"protocolVersion": 2}),
            session::left(deadline),
        )?;
        self.call(
            "set_event_filter",
            json!({"events": null, "messageUpdates": "delta"}),
            session::left(deadline),
        )?;
        self.call(
            "set_ask_dialog",
            json!({"enabled": true}),
            session::left(deadline),
        )?;
        if let Some(tools) = &self.tools {
            self.call(
                "set_host_tools",
                json!({"tools": tools.definitions}),
                session::left(deadline),
            )?;
        }
        let state = self.call("get_state", json!({}), session::left(deadline))?;
        let key = state["sessionFile"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Error::protocol("OMP 没有返回会话文件；请从启动参数中删除 --no-session")
            })?;
        let history = self.history(deadline)?;
        let commands = rpc::commands::or_empty(
            self.call("get_available_commands", json!({}), session::left(deadline))
                .map(|data| rpc::commands::omp_commands(&data["commands"])),
        );
        let mut p = self.live.state.lock();
        p.set_key(NativeSessionKey::from(key));
        p.set_model(rpc::model(&state));
        p.set_session_name(rpc::commands::session_name(&state));
        p.set_commands(commands);
        rpc::replay(&mut p, &history, 0);
        if history
            .last()
            .is_some_and(|m| m["role"] == "assistant" && m["stopReason"] == "aborted")
        {
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

    fn call(&self, kind: &str, mut fields: Value, timeout: Duration) -> Result<Value> {
        let id = new_id();
        fields["type"] = json!(kind);
        fields["id"] = json!(id);
        self.live.request(id, &fields, timeout)
    }

    fn on_frame(&self, stream: &mut Stream, frame: Value) -> Result<()> {
        let kind = frame["type"]
            .as_str()
            .ok_or_else(|| Error::protocol("OMP 帧缺少 type"))?;
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
                    "available_commands_update" => {
                        p.set_commands(rpc::commands::omp_commands(&frame["commands"]));
                    }
                    "session_info_update" if frame.get("title").is_some() => {
                        p.set_session_name(frame["title"].as_str().map(str::to_owned));
                    }
                    "command_output" => {
                        if let Some(text) = frame["text"].as_str().filter(|s| !s.is_empty()) {
                            rpc::commands::note(&mut p, text.to_owned());
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
        }
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

    fn usage_snapshot(&self) -> Result<Option<thinkrail_core::SessionUsage>> {
        self.live.ensure_idle()?;
        let state = self.call("get_state", json!({}), COMMAND)?;
        let stats = self.call("get_session_stats", json!({}), COMMAND)?;
        let usage = rpc::usage::parse(&state, &stats);
        self.live.ensure_idle()?;
        Ok(Some(usage))
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

    fn set_model(
        &self,
        provider: &str,
        model_id: &str,
        effort: Option<ThinkingLevel>,
    ) -> Result<()> {
        rpc::switch_model(
            &self.live,
            |kind, fields| self.call(kind, fields, COMMAND),
            provider,
            model_id,
            effort,
        )
    }
}

#[cfg(test)]
mod tests;
