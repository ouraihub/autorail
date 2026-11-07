//! Official pi CLI over `--mode rpc` (JSONL commands, events and `extension_ui_*` dialogs).

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use thinkrail_core::{
    AgentRuntime, AgentSession, BackendKind, Capabilities, Error, InteractionAnswer,
    NativeSessionKey, Result, SessionRole, SessionSink, SessionSnapshot, StartSession,
    ThinkingLevel,
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

/// `compact` summarises the transcript with one model call; the generic command timeout is short.
const COMPACT: Duration = Duration::from_secs(10 * 60);

const CAPABILITIES: Capabilities = Capabilities {
    resume: true,
    history_replay: true,
    token_deltas: true,
    questions: true,
    tool_permissions: false,
    model_switch: true,
    compact: true,
    rename: true,
};

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

    fn start(
        &self,
        request: StartSession,
        sink: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>> {
        session::validate(&request, BackendKind::Pi)?;
        let command = process::command(&launch_spec(&request)?)?;
        let (process, stdout) = Process::spawn(command, MAX_FRAME)?;
        let projection = Projection::new(session::identity(&request, CAPABILITIES), sink);
        let live = Live::new(process, projection, BackendKind::Pi);
        let pi = Arc::new(PiSession {
            live: live.clone(),
            dialogs: Dialogs::default(),
            reviewer: matches!(request.role, SessionRole::Reviewer { .. }),
        });
        let reader = pi.clone();
        let mut stream = Stream::default();
        session::spawn_reader(live.clone(), stdout, MAX_FRAME, move |frame| {
            reader.on_frame(&mut stream, frame)
        })?;
        let deadline = Instant::now() + HANDSHAKE;
        pi.handshake(deadline)
            .map_err(|error| live.fail_start(error, None))?;
        info!(chat = %request.chat, backend = "pi", "agent session started");
        Ok(pi)
    }
}

/// Host arguments and environment of one pi launch: the generated extension (workflow rules,
/// host tools, providers, MCP servers), the bundled skills (`--skill`) and, for a new session,
/// the default model.
fn launch_spec(request: &StartSession) -> Result<LaunchSpec<'_>> {
    let reviewer = matches!(request.role, SessionRole::Reviewer { .. });
    let mut host_args = vec!["--mode".to_owned(), "rpc".to_owned()];
    let mut host_env = Vec::new();
    let extension = if reviewer {
        projection_engine::pi_reviewer_extension(&request.routing)
    } else {
        projection_engine::pi_extension(request.host_tools.is_some(), &request.routing)
    };
    let path = host::write_support(
        &request.support_dir,
        projection_engine::PI_EXTENSION_FILE,
        &extension,
    )?;
    host_args.extend(["-e".to_owned(), path.display().to_string()]);
    if reviewer {
        host_args.extend([
            "--no-session".into(),
            "--no-skills".into(),
            "--no-prompt-templates".into(),
            "--no-extensions".into(),
            "--tools".into(),
            "read,grep,find,ls,powershell".into(),
            "--system-prompt".into(),
            match &request.role {
                SessionRole::Reviewer { system_prompt } => system_prompt.clone(),
                SessionRole::Worker => unreachable!(),
            },
        ]);
    } else {
        let skills = workflow::stage_skills(&request.support_dir)?;
        host_args.extend(["--skill".to_owned(), skills.display().to_string()]);
        if let Some(access) = &request.host_tools {
            host_env = host::env(access);
        }
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
    reviewer: bool,
}

impl PiSession {
    fn handshake(&self, deadline: Instant) -> Result<()> {
        let state = self.call("get_state", json!({}), session::left(deadline))?;
        let key = state["sessionFile"].as_str().filter(|s| !s.is_empty());
        let history = self.call("get_messages", json!({}), session::left(deadline))?;
        let commands = rpc::commands::or_empty(
            self.call("get_commands", json!({}), session::left(deadline))
                .map(|data| rpc::commands::pi_commands(&data)),
        );
        let mut p = self.live.state.lock();
        if let Some(key) = key {
            p.set_key(NativeSessionKey::from(key));
        }
        p.set_model(rpc::model(&state));
        p.set_session_name(rpc::commands::session_name(&state));
        p.set_commands(commands);
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
        let kind = frame["type"]
            .as_str()
            .ok_or_else(|| Error::protocol("pi 帧缺少 type"))?;
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
                        let error =
                            engine_error(frame["finalError"].as_str().unwrap_or("自动重试失败"));
                        p.error(session_error(&error));
                    }
                    "thinking_level_changed" => rpc::model_event(&mut p, &frame),
                    "compaction_end" if frame["errorMessage"].is_string() => {
                        let error =
                            engine_error(frame["errorMessage"].as_str().unwrap_or_default());
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
        if self.reviewer
            && let Some(id) = frame["id"].as_str()
        {
            return self.live.process.send(&crate::rpc::cancel(id));
        }
        self.dialogs.on_request(&self.live, frame)
    }
}

impl AgentSession for PiSession {
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

    fn compact(&self, instructions: Option<&str>) -> Result<()> {
        self.live.ensure_idle()?;
        self.live.state.lock().set_running(true);
        let mut fields = json!({});
        if let Some(text) = instructions.filter(|s| !s.trim().is_empty()) {
            fields["customInstructions"] = json!(text);
        }
        let result = self.call("compact", fields, COMPACT);
        let mut p = self.live.state.lock();
        p.set_running(false);
        let text = match result {
            Ok(data) => match (
                data["tokensBefore"].as_u64(),
                data["estimatedTokensAfter"].as_u64(),
            ) {
                (Some(before), Some(after)) => {
                    format!("上下文已压缩（约 {before} → {after} tokens）")
                }
                _ => "上下文已压缩".to_owned(),
            },
            // pi refused (e.g. "Nothing to compact"): the session is fine, so this is a turn
            // outcome in the transcript, as in the original, not an app error.
            Err(error) if error.kind() == Some(thinkrail_core::ErrorKind::External) => {
                format!("压缩失败。{}", error.detail())
            }
            Err(error) => return Err(error),
        };
        rpc::commands::note(&mut p, text);
        p.publish();
        Ok(())
    }

    fn rename(&self, name: &str) -> Result<()> {
        self.live.ensure_open()?;
        self.call("set_session_name", json!({"name": name}), COMMAND)?;
        self.live
            .state
            .lock()
            .set_session_name(Some(name.to_owned()));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
