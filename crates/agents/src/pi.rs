//! Official pi CLI over `--mode rpc` (JSONL commands, events and `extension_ui_*` dialogs).

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use thinkrail_core::{
    AgentRuntime, AgentSession, BackendKind, Capabilities, Error, InteractionAnswer, NativeSessionKey, Result,
    SessionSink, SessionSnapshot, StartSession, ThinkingLevel,
};
use tracing::info;

use crate::{
    framing::MAX_FRAME,
    host,
    process::{self, LaunchSpec, Process},
    projection::Projection,
    projection_engine,
    rpc::{self, Dialogs, Stream},
    session::{self, COMMAND, HANDSHAKE, Live, engine_error, new_id, session_error},
    workflow,
};

const CAPABILITIES: Capabilities =
    Capabilities { resume: true, history_replay: true, token_deltas: true, questions: true, tool_permissions: false, model_switch: true };

/// Launches pi `--mode rpc` per chat.
#[derive(Debug, Default)]
pub struct PiRuntime;

impl PiRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl AgentRuntime for PiRuntime {
    fn backend(&self) -> BackendKind {
        BackendKind::Pi
    }

    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    fn start(&self, request: StartSession, sink: Arc<dyn SessionSink>) -> Result<Arc<dyn AgentSession>> {
        session::validate(&request, BackendKind::Pi)?;
        let command = process::command(&launch_spec(&request)?)?;
        let (process, stdout) = Process::spawn(command, MAX_FRAME)?;
        let projection = Projection::new(session::identity(&request, CAPABILITIES), sink);
        let live = Live::new(process, projection, BackendKind::Pi);
        let pi = Arc::new(PiSession { live: live.clone(), dialogs: Dialogs::default() });
        let reader = pi.clone();
        let mut stream = Stream::default();
        session::spawn_reader(live.clone(), stdout, MAX_FRAME, move |frame| reader.on_frame(&mut stream, frame))?;
        let deadline = Instant::now() + HANDSHAKE;
        pi.handshake(deadline).map_err(|error| live.fail_start(error, None))?;
        info!(chat = %request.chat, backend = "pi", "agent session started");
        Ok(pi)
    }
}

/// Host arguments and environment of one pi launch: the generated extension (workflow rules,
/// host tools, providers, MCP servers), the bundled skills (`--skill`) and, for a new session,
/// the default model.
fn launch_spec(request: &StartSession) -> Result<LaunchSpec<'_>> {
    let mut host_args = vec!["--mode".to_owned(), "rpc".to_owned()];
    let mut host_env = Vec::new();
    let extension = projection_engine::pi_extension(request.host_tools.is_some(), &request.routing);
    let path = host::write_support(&request.support_dir, projection_engine::PI_EXTENSION_FILE, &extension)?;
    host_args.extend(["-e".to_owned(), path.display().to_string()]);
    let skills = workflow::stage_skills(&request.support_dir)?;
    host_args.extend(["--skill".to_owned(), skills.display().to_string()]);
    if let Some(access) = &request.host_tools {
        host_env = host::env(access);
    }
    match &request.resume {
        Some(key) => host_args.extend(["--session".to_owned(), key.as_str().to_owned()]),
        None => host_args.extend(projection_engine::pi_model_args(&request.routing)),
    }
    host_env.extend(process::routing_env(&request.routing));
    Ok(LaunchSpec {
        launch: &request.launch,
        config_root: request.config_root.as_deref(),
        host_args,
        cwd: request.cwd.clone(),
        config_var: "PI_CODING_AGENT_DIR",
        host_env,
    })
}

struct PiSession {
    live: Arc<Live>,
    dialogs: Dialogs,
}

impl PiSession {
    fn handshake(&self, deadline: Instant) -> Result<()> {
        let state = self.call("get_state", json!({}), session::left(deadline))?;
        let key = state["sessionFile"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::protocol("pi 没有返回会话文件；请从启动参数中删除 --no-session"))?;
        let history = self.call("get_messages", json!({}), session::left(deadline))?;
        let mut p = self.live.state.lock();
        p.set_key(NativeSessionKey::from(key));
        p.set_model(rpc::model(&state));
        if let Some(messages) = history["messages"].as_array() {
            rpc::replay(&mut p, messages, 0);
        }
        p.set_running(state["isStreaming"] == true);
        if state["isStreaming"] != true {
            p.fail_unfinished_tools();
        }
        p.publish();
        Ok(())
    }

    /// Sends a command and waits for its `response`.
    fn call(&self, kind: &str, mut fields: Value, timeout: Duration) -> Result<Value> {
        let id = new_id();
        fields["type"] = json!(kind);
        fields["id"] = json!(id);
        self.live.request(id, &fields, timeout)
    }

    fn on_frame(&self, stream: &mut Stream, frame: Value) -> Result<()> {
        let kind = frame["type"].as_str().ok_or_else(|| Error::protocol("pi 帧缺少 type"))?;
        match kind {
            "response" => self.on_response(frame),
            "extension_ui_request" => self.on_ui_request(&frame),
            "extension_error" => {
                let error = engine_error(frame["error"].as_str().unwrap_or("扩展执行失败"));
                self.live.state.lock().error(session_error(&error));
                Ok(())
            }
            _ => {
                let mut p = self.live.state.lock();
                match kind {
                    "agent_start" => p.set_running(true),
                    "agent_settled" => {
                        p.fail_unfinished_tools();
                        p.set_running(false);
                    }
                    "message_start" => stream.message_start(&mut p, &frame),
                    "message_update" => stream.message_update(&mut p, &frame),
                    "message_end" => stream.message_end(&mut p, &frame),
                    "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                        rpc::tool_event(&mut p, &frame);
                    }
                    "auto_retry_end" if frame["success"] == false => {
                        let error = engine_error(frame["finalError"].as_str().unwrap_or("自动重试失败"));
                        p.error(session_error(&error));
                    }
                    "thinking_level_changed" => rpc::model_event(&mut p, &frame),
                    "compaction_end" if frame["errorMessage"].is_string() => {
                        let error = engine_error(frame["errorMessage"].as_str().unwrap_or_default());
                        p.error(session_error(&error));
                    }
                    _ => {}
                }
                Ok(())
            }
        }
    }

    fn on_response(&self, frame: Value) -> Result<()> {
        let Some(id) = frame["id"].as_str() else {
            if frame["success"] == false {
                let error = engine_error(frame["error"].as_str().unwrap_or("命令失败"));
                self.live.state.lock().error(session_error(&error));
            }
            return Ok(());
        };
        let result = if frame["success"] == true {
            Ok(frame.get("data").cloned().unwrap_or(Value::Null))
        } else {
            Err(engine_error(frame["error"].as_str().unwrap_or("命令失败")))
        };
        if let Some(Err(error)) = self.live.resolve(id, result) {
            self.live.state.lock().error(session_error(&error));
        }
        Ok(())
    }

    fn on_ui_request(&self, frame: &Value) -> Result<()> {
        self.dialogs.on_request(&self.live, frame)
    }
}

impl AgentSession for PiSession {
    fn snapshot(&self) -> SessionSnapshot {
        self.live.state.lock().snapshot()
    }

    fn prompt(&self, text: &str) -> Result<()> {
        self.live.ensure_idle()?;
        self.live.state.lock().set_running(true);
        match self.call("prompt", json!({"message": text}), COMMAND) {
            // `handled`: an extension command completed locally; a run it starts reports agent_start.
            Ok(data) if data["disposition"] == "handled" => {
                self.live.state.lock().set_running(false);
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(error) => {
                self.live.state.lock().set_running(false);
                Err(error)
            }
        }
    }

    fn stop(&self) -> Result<()> {
        self.live.ensure_open()?;
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

#[cfg(test)]
mod tests;
