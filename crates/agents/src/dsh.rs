//! DSH over ACP (JSON-RPC 2.0 on stdio): `initialize`, `session/new|resume|prompt|close`,
//! `session/cancel`, `session/update` notifications and `session/request_permission`.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use thinkrail_core::{
    AgentRuntime, AgentSession, BackendKind, Capabilities, ContentBlock, Error, ErrorKind, Interaction,
    InteractionAnswer, InteractionKind, InteractionOption, Message, ModelInfo, NativeSessionKey, Result, Role,
    SessionSink, SessionSnapshot, StartSession, ToolStatus,
};
use tracing::{debug, info};

use crate::{
    framing::MAX_FRAME,
    host,
    process::{self, LaunchSpec, Process},
    projection::Projection,
    session::{self, HANDSHAKE, Live, engine_error, new_id, session_error},
};

const CLOSE: Duration = Duration::from_secs(10);
const RESERVED: [&str; 1] = ["--profile"];
const CAPABILITIES: Capabilities =
    Capabilities { resume: false, history_replay: false, token_deltas: false, questions: false, tool_permissions: false };
const METHOD_NOT_FOUND: i64 = -32601;

/// Launches DSH `--profile acp` per chat. Profile arguments are the CLI entry (e.g. `lib/bin.js`);
/// the launcher flags must precede any app argument, so the host inserts them right after.
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

    fn start(&self, request: StartSession, sink: Arc<dyn SessionSink>) -> Result<Arc<dyn AgentSession>> {
        session::validate(&request, BackendKind::Dsh)?;
        session::reject_args(&request.profile, &RESERVED)?;
        let home = dsh_home(&request)?;
        let mut host_args = vec!["--profile".to_owned(), "acp".to_owned()];
        if let Some(access) = &request.host_tools {
            // The host's TODO tools replace DSH's built-in one.
            host_args.extend(["--patch".to_owned(), host::dsh_patch(access)?.display().to_string()]);
        }
        let spec = LaunchSpec {
            profile: &request.profile,
            host_args,
            // DSH loads `.env` from its process cwd; keep the workspace out of that lookup.
            cwd: home,
            config_var: "DSH_HOME",
            host_env: Vec::new(),
        };
        let command = process::command(&spec)?;
        let (process, stdout) = Process::spawn(command, MAX_FRAME)?;
        let projection = Projection::new(session::identity(&request, CAPABILITIES), sink);
        let live = Live::new(process, projection, BackendKind::Dsh);
        let dsh = Arc::new(DshSession {
            live: live.clone(),
            key: Mutex::new(String::new()),
            permissions: Mutex::new(HashMap::new()),
        });
        let reader = dsh.clone();
        session::spawn_reader(live.clone(), stdout, MAX_FRAME, move |frame| reader.on_frame(frame))?;
        let deadline = Instant::now() + HANDSHAKE;
        dsh.handshake(&request, deadline).map_err(|error| live.fail_start(error, None))?;
        info!(chat = %request.chat, backend = "dsh", "agent session started");
        Ok(dsh)
    }
}

/// The config root the process runs in: the profile's, else DSH's default `~/.dsh`.
fn dsh_home(request: &StartSession) -> Result<PathBuf> {
    let home = match &request.profile.input.config_root {
        Some(root) => root.clone(),
        None => std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(|home| PathBuf::from(home).join(".dsh"))
            .ok_or_else(|| Error::invalid("找不到用户主目录，请为 DSH 档案设置配置目录"))?,
    };
    if !home.is_dir() {
        return Err(Error::new(ErrorKind::BackendUnavailable, format!("DSH 配置目录不存在：{}", home.display())));
    }
    Ok(home)
}

struct DshSession {
    live: Arc<Live>,
    key: Mutex<String>,
    /// Open permission requests: interaction id → JSON-RPC request id to reply to.
    permissions: Mutex<HashMap<String, Value>>,
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
            return Err(Error::protocol(format!("DSH 返回了不支持的 ACP 协议版本 {}", init["protocolVersion"])));
        }
        let sessions = &init["agentCapabilities"]["sessionCapabilities"];
        let resume = sessions.get("resume").is_some();
        let cwd = process::plain_path(&request.cwd);
        let servers = match &request.host_tools {
            Some(access) => vec![host::acp_server(access)?],
            None => Vec::new(),
        };
        let (key, reply) = match &request.resume {
            Some(_) if !resume => return Err(Error::invalid("这个 DSH 版本不支持恢复会话")),
            Some(key) => {
                let params = json!({"sessionId": key.as_str(), "cwd": cwd, "mcpServers": servers});
                (key.as_str().to_owned(), self.call("session/resume", params, session::left(deadline))?)
            }
            None => {
                let reply = self.call("session/new", json!({"cwd": cwd, "mcpServers": servers}), session::left(deadline))?;
                let key = reply["sessionId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::protocol("DSH 没有返回 sessionId"))?
                    .to_owned();
                (key, reply)
            }
        };
        key.clone_into(&mut self.key.lock());
        let mut p = self.live.state.lock();
        p.set_key(NativeSessionKey::from(key));
        p.set_capabilities(Capabilities { resume, ..CAPABILITIES });
        p.set_model(model(&reply["configOptions"]));
        p.publish();
        Ok(())
    }

    fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = new_id();
        self.live.request(id.clone(), &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}), timeout)
    }

    fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.live.process.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
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
                    self.live.process.send(&json!({"jsonrpc": "2.0", "id": id, "error": error}))?;
                }
                debug!(method, "unhandled ACP method");
            }
        }
        Ok(())
    }

    fn on_update(&self, update: &Value) {
        apply_update(&mut self.live.state.lock(), update);
    }

    fn on_permission(&self, id: Value, params: &Value) -> Result<()> {
        if params["sessionId"] != *self.key.lock() {
            let cancelled = json!({"outcome": {"outcome": "cancelled"}});
            return self.live.process.send(&json!({"jsonrpc": "2.0", "id": id, "result": cancelled}));
        }
        let options = params["options"]
            .as_array()
            .filter(|o| !o.is_empty())
            .ok_or_else(|| Error::protocol("权限请求缺少 options"))?
            .iter()
            .map(|o| InteractionOption { id: str_of(&o["optionId"]), label: str_of(&o["name"]) })
            .collect();
        let tool_call_id = params["toolCall"]["toolCallId"].as_str().unwrap_or_default();
        let title = {
            let p = self.live.state.lock();
            p.tool_owner(tool_call_id).and_then(|owner| tool_name(&p.snapshot(), &owner, tool_call_id))
        }
        .unwrap_or_default();
        let interaction = Interaction {
            request_id: new_id(),
            kind: InteractionKind::Permission,
            title,
            message: String::new(),
            options,
            detail: params.clone(),
        };
        self.permissions.lock().insert(interaction.request_id.clone(), id);
        self.live.state.lock().add_interaction(interaction);
        Ok(())
    }

    fn reply_permission(&self, rpc_id: Value, outcome: Value) -> Result<()> {
        self.live.process.send(&json!({"jsonrpc": "2.0", "id": rpc_id, "result": {"outcome": outcome}}))
    }

    /// ACP: a client cancelling a turn answers its open permission requests `cancelled` first.
    fn cancel_permissions(&self) {
        let open: Vec<Value> = self.permissions.lock().drain().map(|(_, id)| id).collect();
        let had = !self.live.state.lock().take_interactions().is_empty();
        for rpc_id in open {
            if let Err(error) = self.reply_permission(rpc_id, json!({"outcome": "cancelled"})) {
                debug!(%error, "cannot cancel permission request");
            }
        }
        if had {
            self.live.state.lock().publish();
        }
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
            let content = vec![ContentBlock::Text { text: text.to_owned() }];
            p.upsert(Message { id: format!("user-{id}"), role: Role::User, content });
            p.set_running(true);
        }
        let key = self.key.lock().clone();
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": "session/prompt",
            "params": {"sessionId": key, "prompt": [{"type": "text", "text": text}]}});
        if let Err(error) = self.live.process.send(&frame) {
            self.live.forget(&id);
            self.live.state.lock().set_running(false);
            return Err(error);
        }
        Ok(())
    }

    fn stop(&self) -> Result<()> {
        self.live.ensure_open()?;
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
        if !self.live.begin_dispose() {
            return Ok(());
        }
        self.cancel_permissions();
        let key = self.key.lock().clone();
        if self.live.state.lock().is_running()
            && let Err(error) = self.notify("session/cancel", json!({"sessionId": key}))
        {
            debug!(%error, "cannot cancel prompt during dispose");
        }
        let id = new_id();
        let close = json!({"jsonrpc": "2.0", "id": id, "method": "session/close", "params": {"sessionId": key}});
        if let Err(error) = self.live.request(id, &close, CLOSE) {
            debug!(%error, "session/close did not complete");
        }
        self.live.shutdown();
        Ok(())
    }
}

fn chunk_block(content: &Value, thought: bool) -> Option<ContentBlock> {
    match content["type"].as_str()? {
        "text" if thought => Some(ContentBlock::Thinking { text: str_of(&content["text"]) }),
        "text" => Some(ContentBlock::Text { text: str_of(&content["text"]) }),
        "image" => Some(ContentBlock::Image { mime_type: str_of(&content["mimeType"]) }),
        _ => None,
    }
}

/// A failed turn shows as a system note in the transcript, like a pi/OMP model error.
fn turn_failed(p: &mut Projection, note_id: String, error: &Error) {
    let note = Message { id: note_id, role: Role::System, content: vec![ContentBlock::Text { text: error.to_string() }] };
    p.upsert(note);
    p.error(session_error(error));
}

fn tool_status(status: &Value) -> Option<ToolStatus> {
    match status.as_str()? {
        "pending" | "in_progress" => Some(ToolStatus::Running),
        "completed" => Some(ToolStatus::Completed),
        "failed" => Some(ToolStatus::Failed),
        _ => None,
    }
}

fn tool_output(items: &[Value]) -> String {
    items
        .iter()
        .filter_map(|item| match item["type"].as_str()? {
            "content" => match item["content"]["type"].as_str()? {
                "text" => item["content"]["text"].as_str().map(str::to_owned),
                "image" => Some("[图片]".to_owned()),
                _ => None,
            },
            "diff" => Some(format!("[diff] {}", str_of(&item["path"]))),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_name(snapshot: &SessionSnapshot, owner: &str, tool_call_id: &str) -> Option<String> {
    snapshot.messages.iter().find(|m| m.id == owner)?.content.iter().find_map(|block| match block {
        ContentBlock::Tool { tool_call_id: id, name, .. } if id == tool_call_id => Some(name.clone()),
        _ => None,
    })
}

/// The model option's `currentValue` is a JSON-encoded `[provider, model]`.
fn model(options: &Value) -> Option<ModelInfo> {
    let option = options.as_array()?.iter().find(|o| o["category"] == "model")?;
    let value = option["currentValue"].as_str()?;
    match serde_json::from_str::<(String, String)>(value) {
        Ok((provider, id)) => Some(ModelInfo { provider: Some(provider), id }),
        Err(_) => Some(ModelInfo { provider: None, id: value.to_owned() }),
    }
}

fn str_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// Projects one ACP `session/update`. Chunks are committed blocks (not token deltas): every chunk
/// of one `messageId` appends a block to that message.
fn apply_update(p: &mut Projection, update: &Value) {
    match update["sessionUpdate"].as_str() {
        Some(kind @ ("agent_message_chunk" | "agent_thought_chunk" | "user_message_chunk")) => {
            let Some(block) = chunk_block(&update["content"], kind == "agent_thought_chunk") else { return };
            let role = if kind == "user_message_chunk" { Role::User } else { Role::Assistant };
            let id = update["messageId"].as_str().map_or_else(new_id, str::to_owned);
            if !p.update(&id, |m| m.content.push(block.clone())) {
                p.upsert(Message { id, role, content: vec![block] });
            }
        }
        Some("tool_call") => {
            let Some(tool_call_id) = update["toolCallId"].as_str() else { return };
            let tool = ContentBlock::Tool {
                tool_call_id: tool_call_id.to_owned(),
                name: str_of(&update["title"]),
                status: tool_status(&update["status"]).unwrap_or(ToolStatus::Running),
                input: update.get("rawInput").cloned(),
                output: None,
            };
            p.upsert(Message { id: format!("tool-{tool_call_id}"), role: Role::Assistant, content: vec![tool] });
        }
        Some("tool_call_update") => {
            let Some(tool_call_id) = update["toolCallId"].as_str() else { return };
            let status = tool_status(&update["status"]).unwrap_or(ToolStatus::Running);
            let output = update["content"].as_array().map(|items| tool_output(items));
            p.set_tool(tool_call_id, status, output);
        }
        Some("config_option_update") => p.set_model(model(&update["configOptions"])),
        _ => {}
    }
}

/// The ACP permission `outcome` for a user's answer; only offered options or a cancel are valid.
fn permission_outcome(interaction: &Interaction, answer: &InteractionAnswer) -> Result<Value> {
    match answer {
        InteractionAnswer::Cancel => Ok(json!({"outcome": "cancelled"})),
        InteractionAnswer::Select { option_id } if interaction.options.iter().any(|o| &o.id == option_id) => {
            Ok(json!({"outcome": "selected", "optionId": option_id}))
        }
        _ => Err(Error::invalid("权限请求只能选择给出的选项或取消")),
    }
}

#[cfg(test)]
mod tests;
