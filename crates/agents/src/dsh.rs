//! DSH over ACP (JSON-RPC 2.0 on stdio): `initialize`, `session/new|resume|prompt|close`,
//! `session/cancel`, `session/update` notifications and `session/request_permission`.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use thinkrail_core::{
    AgentRuntime, AgentSession, BackendKind, Capabilities, ContentBlock, Error, ErrorCode as _,
    ErrorKind, InteractionAnswer, Message, NativeSessionKey, Result, Role, SessionRole,
    SessionSink, SessionSnapshot, StartSession, ThinkingLevel,
};
use tracing::{debug, info};

use crate::{
    framing::MAX_FRAME,
    process::{self, Process},
    projection::Projection,
    session::{self, COMMAND, HANDSHAKE, Live, engine_error, new_id, session_error},
};

mod launch;
mod permissions;
mod skills;
mod update;

use launch::{launch_spec, mcp_hint, session_params};
use permissions::permission_outcome;
use update::{apply_update, effort_option, model, model_option};

const CLOSE: Duration = Duration::from_secs(10);
const CAPABILITIES: Capabilities = Capabilities {
    resume: false,
    history_replay: false,
    token_deltas: false,
    questions: false,
    tool_permissions: false,
    model_switch: true,
    compact: false,
    rename: false,
};
const METHOD_NOT_FOUND: i64 = -32601;

/// Launches DSH `--profile acp` (DSH's own `acp` profile) per chat. Launch arguments are the CLI
/// entry (e.g. `lib/bin.js`); the launcher flags must precede any app argument, so the host
/// inserts them right after.
#[derive(Debug, Default)]
pub struct DshRuntime;

impl DshRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl AgentRuntime for DshRuntime {
    fn backend(&self) -> BackendKind {
        BackendKind::Dsh
    }

    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    fn start(
        &self,
        request: StartSession,
        sink: Arc<dyn SessionSink>,
    ) -> Result<Arc<dyn AgentSession>> {
        session::validate(&request, BackendKind::Dsh)?;
        let trace = Arc::new(crate::trace::SessionTrace::new(&request));
        let _entered = trace.enter("agent.session.start");
        let (spec, staged) = launch_spec(&request)?;
        let commands = skills::commands(&request.cwd, &staged, &spec.cwd);
        let command = process::command(&spec)?;
        let (process, stdout) = Process::spawn(command, MAX_FRAME, trace)?;
        let projection = Projection::new(session::identity(&request, CAPABILITIES), sink);
        let live = Live::new(process, projection, BackendKind::Dsh);
        let dsh = Arc::new(DshSession {
            live: live.clone(),
            key: Mutex::new(String::new()),
            permissions: Mutex::new(HashMap::new()),
            reviewer: matches!(request.role, SessionRole::Reviewer { .. }),
        });
        let reader = dsh.clone();
        session::spawn_reader(live.clone(), stdout, MAX_FRAME, move |frame| {
            reader.on_frame(frame)
        })?;
        let deadline = Instant::now() + HANDSHAKE;
        dsh.handshake(&request, deadline)
            .map_err(|error| live.fail_start(error, None))?;
        live.state.lock().set_commands(commands);
        info!(chat = %request.chat, backend = "dsh", "agent session started");
        Ok(dsh)
    }
}

struct DshSession {
    live: Arc<Live>,
    key: Mutex<String>,
    /// Open permission requests: interaction id → JSON-RPC request id to reply to.
    permissions: Mutex<HashMap<String, Value>>,
    reviewer: bool,
}

impl DshSession {
    fn handshake(&self, request: &StartSession, deadline: Instant) -> Result<()> {
        let init = self.call(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {},
                   "clientInfo": {"name": "thinkrail-native", "version": env!("CARGO_PKG_VERSION")}}),
            session::left(deadline),
        )?;
        if init["protocolVersion"] != 1 {
            return Err(Error::protocol(format!(
                "DSH 返回了不支持的 ACP 协议版本 {}",
                init["protocolVersion"]
            )));
        }
        let sessions = &init["agentCapabilities"]["sessionCapabilities"];
        let resume = sessions.get("resume").is_some();
        let cwd = process::plain_path(&request.cwd);
        let params = session_params(request, &cwd)?;
        let (key, reply) = match &request.resume {
            Some(_) if !resume => return Err(Error::invalid("这个 DSH 版本不支持恢复会话")),
            Some(key) => {
                let reply = self.call("session/resume", params, session::left(deadline));
                (
                    key.as_str().to_owned(),
                    reply.map_err(|e| mcp_hint(e, request))?,
                )
            }
            None => {
                let reply = self
                    .call("session/new", params, session::left(deadline))
                    .map_err(|e| mcp_hint(e, request))?;
                let key = reply["sessionId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::protocol("DSH 没有返回 sessionId"))?
                    .to_owned();
                (key, reply)
            }
        };
        key.clone_into(&mut self.key.lock());
        let mut reply = reply;
        let mut options = reply
            .get_mut("configOptions")
            .map(Value::take)
            .unwrap_or_default();
        // `routing.default_model` is only given for a new session (DSH starts at the `acp` entry's
        // model); its level is applied right after `session/new`.
        if let Some(level) = request
            .routing
            .default_model
            .as_ref()
            .and_then(|m| m.effort)
        {
            options = self.set_option(
                effort_option(&key, &options, level)?,
                session::left(deadline),
            )?;
        }
        let mut p = self.live.state.lock();
        p.set_key(NativeSessionKey::from(key));
        p.set_capabilities(Capabilities {
            resume,
            ..CAPABILITIES
        });
        p.set_model(model(&options));
        p.publish();
        Ok(())
    }

    fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = new_id();
        self.live.request(
            id.clone(),
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
            timeout,
        )
    }

    fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.live
            .process
            .send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// `session/set_config_option`; DSH answers with the complete resulting `configOptions`.
    fn set_option(&self, params: Value, timeout: Duration) -> Result<Value> {
        let mut reply = self.call("session/set_config_option", params, timeout)?;
        Ok(reply
            .get_mut("configOptions")
            .map(Value::take)
            .unwrap_or_default())
    }

    fn on_frame(&self, frame: Value) -> Result<()> {
        if frame["jsonrpc"] != "2.0" {
            return Err(Error::protocol("DSH 帧不是 JSON-RPC 2.0"));
        }
        let Some(method) = frame["method"].as_str() else {
            let key = match &frame["id"] {
                Value::String(id) => id.clone(),
                Value::Number(id) => id.to_string(),
                _ => return Err(Error::protocol("DSH 响应缺少 id")),
            };
            let result = match frame.get("error") {
                Some(error) => Err(engine_error(&format!(
                    "{} ({})",
                    error["message"].as_str().unwrap_or("请求失败"),
                    error["code"]
                ))),
                None => Ok(frame["result"].clone()),
            };
            if let Some(Err(error)) = self.live.resolve(&key, result) {
                self.live.state.lock().error(session_error(&error));
            }
            return Ok(());
        };
        let params = &frame["params"];
        let request_id = frame.get("id").cloned();
        match method {
            "session/update" => {
                if params["sessionId"] == *self.key.lock() {
                    self.on_update(&params["update"]);
                }
            }
            "session/request_permission" => match request_id {
                Some(id) => self.on_permission(id, params)?,
                None => return Err(Error::protocol("权限请求缺少 id")),
            },
            _ => {
                if let Some(id) = request_id {
                    let error = json!({"code": METHOD_NOT_FOUND, "message": format!("method not found: {method}")});
                    self.live
                        .process
                        .send(&json!({"jsonrpc": "2.0", "id": id, "error": error}))?;
                }
                debug!(method, "unhandled ACP method");
            }
        }
        Ok(())
    }

    fn on_update(&self, update: &Value) {
        apply_update(&mut self.live.state.lock(), update);
    }
}

impl AgentSession for DshSession {
    fn snapshot(&self) -> SessionSnapshot {
        self.live.state.lock().snapshot()
    }

    /// Returns once `session/prompt` is written; its reply ends the turn.
    fn prompt(&self, text: &str) -> Result<()> {
        self.live.ensure_idle()?;
        let id = new_id();
        let live = Arc::downgrade(&self.live);
        let note_id = format!("user-{id}-error");
        self.live.on_reply(
            id.clone(),
            Box::new(move |result| {
                let Some(live) = live.upgrade() else { return };
                let mut p = live.state.lock();
                if let Err(error) = result
                    && error.kind() != Some(ErrorKind::SessionClosed)
                {
                    turn_failed(&mut p, note_id, &error);
                }
                p.fail_unfinished_tools();
                p.set_running(false);
            }),
        )?;
        {
            let mut p = self.live.state.lock();
            let content = vec![ContentBlock::Text {
                text: text.to_owned(),
            }];
            p.upsert(Message {
                id: format!("user-{id}"),
                role: Role::User,
                content,
            });
            p.set_running(true);
        }
        let key = self.key.lock().clone();
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": "session/prompt",
            "params": {"sessionId": key, "prompt": [{"type": "text", "text": text}]}});
        if let Err(error) = self.live.process.send(&frame) {
            self.live.state.lock().finish_turn_error(error.code());
            self.live.forget(&id);
            self.live.state.lock().set_running(false);
            return Err(error);
        }
        Ok(())
    }

    fn stop(&self) -> Result<()> {
        self.live.ensure_open()?;
        let _entered = self.live.process.trace.enter("agent.session.stop");
        info!("agent.session.stop_requested");
        self.live.state.lock().cancel_turn("user_cancelled");
        self.cancel_permissions();
        self.notify("session/cancel", json!({"sessionId": *self.key.lock()}))
    }

    fn respond(&self, request_id: &str, answer: InteractionAnswer) -> Result<()> {
        self.live.ensure_open()?;
        let expired = || Error::new(ErrorKind::InteractionExpired, "该问题已被回答或已失效");
        let mut p = self.live.state.lock();
        let interaction = p.take_interaction(request_id).ok_or_else(expired)?;
        let outcome = permission_outcome(&interaction, &answer);
        let rpc_id = self.permissions.lock().get(request_id).cloned();
        let sent = match (outcome, rpc_id) {
            (Ok(outcome), Some(rpc_id)) => self.reply_permission(rpc_id, outcome),
            (Err(error), _) => Err(error),
            (Ok(_), None) => Err(expired()),
        };
        if let Err(error) = sent {
            p.restore_interaction(interaction);
            return Err(error);
        }
        self.permissions.lock().remove(request_id);
        p.publish();
        Ok(())
    }

    /// Cancels open questions and the running turn, closes the ACP session (10 s), then stdin.
    fn dispose(&self) -> Result<()> {
        let _entered = self.live.process.trace.enter("agent.session.dispose");
        if !self.live.begin_dispose() {
            return Ok(());
        }
        self.cancel_permissions();
        let key = self.key.lock().clone();
        if self.live.state.lock().is_running()
            && let Err(error) = self.notify("session/cancel", json!({"sessionId": key}))
        {
            debug!(code = error.code(), "cannot cancel prompt during dispose");
        }
        let id = new_id();
        let close = json!({"jsonrpc": "2.0", "id": id, "method": "session/close", "params": {"sessionId": key}});
        if let Err(error) = self.live.request(id, &close, CLOSE) {
            debug!(code = error.code(), "session/close did not complete");
        }
        self.live.shutdown();
        Ok(())
    }

    /// Switches the model, then the effort the new model offers; the snapshot follows each reply.
    fn set_model(
        &self,
        provider: &str,
        model_id: &str,
        effort: Option<ThinkingLevel>,
    ) -> Result<()> {
        self.live.ensure_idle()?;
        let key = self.key.lock().clone();
        let options = self.set_option(model_option(&key, provider, model_id), COMMAND)?;
        self.live.state.lock().set_model(model(&options));
        if let Some(level) = effort {
            let options = self.set_option(effort_option(&key, &options, level)?, COMMAND)?;
            self.live.state.lock().set_model(model(&options));
        }
        Ok(())
    }
}

/// A failed turn shows as a system note in the transcript, like a pi/OMP model error.
fn turn_failed(p: &mut Projection, note_id: String, error: &Error) {
    let note = Message {
        id: note_id,
        role: Role::System,
        content: vec![ContentBlock::Text {
            text: error.to_string(),
        }],
    };
    p.upsert(note);
    p.error(session_error(error));
}

fn str_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests;
