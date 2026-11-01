//! DSH over ACP (JSON-RPC 2.0 on stdio): `initialize`, `session/new|resume|prompt|close`,
//! `session/cancel`, `session/update` notifications and `session/request_permission`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use thinkrail_core::{
    AgentRuntime, AgentSession, BackendKind, Capabilities, ContentBlock, Error, ErrorKind, Interaction,
    InteractionAnswer, InteractionKind, InteractionOption, IoContext, Message, ModelInfo, NativeSessionKey, Result, Role,
    SessionSink, SessionSnapshot, StartSession, ThinkingLevel, ToolStatus,
};
use tracing::{debug, info};

use crate::{
    framing::MAX_FRAME,
    host,
    process::{self, LaunchSpec, Process},
    projection::Projection,
    projection_engine::{self, level_name},
    rpc,
    session::{self, COMMAND, HANDSHAKE, Live, engine_error, new_id, session_error},
    workflow,
};

const CLOSE: Duration = Duration::from_secs(10);
const CAPABILITIES: Capabilities =
    Capabilities { resume: false, history_replay: false, token_deltas: false, questions: false, tool_permissions: false, model_switch: true };
const MODEL_OPTION: &str = "model";
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

    fn start(&self, request: StartSession, sink: Arc<dyn SessionSink>) -> Result<Arc<dyn AgentSession>> {
        session::validate(&request, BackendKind::Dsh)?;
        let command = process::command(&launch_spec(&request)?)?;
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

/// Host arguments and environment of one DSH launch: the generated patch (bundled skills,
/// workflow rules, built-in TODO tool off, providers, `acp` default model).
fn launch_spec(request: &StartSession) -> Result<LaunchSpec<'_>> {
    let home = dsh_home(request)?;
    let mut host_args = vec!["--profile".to_owned(), "acp".to_owned()];
    let mut host_env: Vec<_> = process::routing_env(&request.routing).collect();
    let skills = workflow::stage_skills(&request.support_dir)?;
    let patch = projection_engine::dsh_patch(request.host_tools.is_some(), &request.routing, &skills)?;
    let patch = preserve_user_routes(&home, patch, &mut host_env)?;
    let path = host::write_support(&request.support_dir, projection_engine::DSH_PATCH_FILE, &patch)?;
    host_args.extend(["--patch".to_owned(), path.display().to_string()]);
    Ok(LaunchSpec {
        launch: &request.launch,
        config_root: request.config_root.as_deref(),
        host_args,
        // DSH loads `.env` from its process cwd; keep the workspace out of that lookup.
        cwd: home,
        config_var: "DSH_HOME",
        host_env,
    })
}


fn preserve_user_routes(home: &Path, patch: String, env: &mut Vec<(String, std::ffi::OsString)>) -> Result<String> {
    use serde_yaml_ng::Value as Yaml;
    if !patch.contains("- id: llm-pi-ai\n") {
        return Ok(patch);
    }
    let parse = |text: &str| {
        let text = if text.trim_start().starts_with("%") {
            text.to_owned()
        } else {
            format!("%TAG !! !\n---\n{}", text.trim_start().strip_prefix("---").unwrap_or(text))
        };
        serde_yaml_ng::from_str::<Yaml>(&text)
            .map_err(|_| Error::invalid("DSH 覆盖层不是有效的 YAML，请检查 cordis.patch.yml"))
    };
    let mut original = serde_json::Map::new();
    for path in [home.join("profiles/acp/cordis.patch.yml"), home.join("cordis.patch.yml")] {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(Error::invalid("无法读取 DSH 用户覆盖层，请检查配置目录权限")),
        };
        let parsed = parse(&text)?;
        if let Some(entries) = parsed.as_sequence() {
            for entry in entries {
                if entry["id"].as_str() == Some("llm-pi-ai")
                    && let Some(config) = entry.get("config")
                {
                    original = match config {
                        Yaml::Null => serde_json::Map::new(),
                        Yaml::Tagged(_) => {
                            let value = yaml_expressions(config)?;
                            serde_json::Map::from_iter([("__jsExpr".into(), value["__jsExpr"].clone())])
                        }
                        _ => yaml_expressions(config)?.as_object().cloned()
                            .ok_or_else(|| Error::invalid("DSH 提供方 config 必须是对象"))?,
                    };
                }
            }
        }
    }
    if original.is_empty() {
        return Ok(patch);
    }
    let projected = parse(&patch)?;
    let mut output = Vec::new();
    for entry in projected.as_sequence().ok_or_else(|| Error::protocol("DSH 投影不是列表"))? {
        if entry["id"].as_str() == Some("llm-pi-ai") {
            let projected = serde_json::to_value(&entry["config"]).map_err(|_| Error::protocol("无法编码 DSH 提供方投影"))?;
            let value = serde_json::to_string(&json!({"base": original, "projected": projected}))
                .map_err(|_| Error::protocol("无法编码 DSH 合并配置"))?;
            env.push(("THINKRAIL_DSH_MERGED_CONFIG".into(), value.into()));
            output.push(concat!("- id: llm-pi-ai\n  config: !!js >-\n    ",
                "((raw) => { const visit = (v) => v && typeof v === \"object\" ? ",
                "(typeof v.__jsExpr === \"string\" ? eval(v.__jsExpr) : ",
                "Array.isArray(v) ? v.map(visit) : Object.fromEntries(Object.entries(v).map(([k,x]) => [k,visit(x)]))) : v; ",
                "const base = visit(raw.base); return { ...base, providers: { ...base.providers, ...raw.projected.providers } }; ",
                "})(JSON.parse(process.env.THINKRAIL_DSH_MERGED_CONFIG))\n").to_owned());
        } else {
            output.push(serde_yaml_ng::to_string(&vec![entry]).map_err(|_| Error::protocol("无法编码 DSH 覆盖层"))?);
        }
    }
    Ok(output.concat())
}

fn yaml_expressions(value: &serde_yaml_ng::Value) -> Result<Value> {
    use serde_yaml_ng::Value as Yaml;
    match value {
        Yaml::Tagged(tagged) if tagged.tag == "tag:yaml.org,2002:js" || tagged.tag == "!js" => {
            let expression = tagged.value.as_str().ok_or_else(|| Error::invalid("DSH !!js 表达式必须是字符串"))?;
            Ok(json!({"__jsExpr": expression}))
        }
        Yaml::Tagged(_) => Err(Error::invalid("DSH 提供方使用了不支持的 YAML 标签，请使用普通值或 !!js")),
        Yaml::Sequence(items) => items.iter().map(yaml_expressions).collect::<Result<Vec<_>>>().map(Value::Array),
        Yaml::Mapping(items) => items.iter().map(|(key, value)| {
            let key = key.as_str().ok_or_else(|| Error::invalid("DSH 提供方字段名必须是字符串"))?;
            Ok((key.to_owned(), yaml_expressions(value)?))
        }).collect::<Result<serde_json::Map<_, _>>>().map(Value::Object),
        _ => serde_json::to_value(value).map_err(|_| Error::invalid("无法编码 DSH 配置值")),
    }
}
/// The config root the process runs in: the request's, else DSH's default `~/.dsh`, created when
/// missing so a fresh install starts.
fn dsh_home(request: &StartSession) -> Result<PathBuf> {
    let home = process::config_dir(request.config_root.as_deref(), ".dsh")?;
    std::fs::create_dir_all(&home).context(format!("无法创建 DSH 配置目录 {}", home.display()))?;
    Ok(home)
}

/// `session/new` / `session/resume` parameters: the host tool server (when on) followed by the
/// projected MCP servers, whose values travel inline (04 §6.4).
fn session_params(request: &StartSession, cwd: &Path) -> Result<Value> {
    let mut servers = match &request.host_tools {
        Some(access) => vec![host::acp_server(access)?],
        None => Vec::new(),
    };
    let mut projected = projection_engine::acp_mcp_servers(&request.routing)?;
    for server in &mut projected {
        if let Some(command) = server["command"].as_str() {
            server["command"] = json!(mcp_executable(command)?);
        }
    }
    servers.extend(projected);
    let mut params = json!({"cwd": cwd, "mcpServers": servers});
    if let Some(key) = &request.resume {
        params["sessionId"] = json!(key.as_str());
    }
    Ok(params)
}

fn mcp_executable(command: &str) -> Result<PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() && path.is_file() {
        return Ok(process::plain_path(path));
    }
    if path.components().count() == 1 {
        let extensions = std::env::var_os("PATHEXT").unwrap_or_else(|| ".EXE;.CMD;.BAT;.COM".into());
        let extensions = extensions.to_string_lossy();
        if let Some(search) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&search) {
                if !directory.is_absolute() {
                    continue;
                }
                let candidate = directory.join(path);
                if candidate.is_file() {
                    return Ok(process::plain_path(&candidate));
                }
                #[cfg(windows)]
                if path.extension().is_none() {
                    for extension in extensions.split(';').filter(|value| !value.is_empty()) {
                        let candidate = directory.join(format!("{command}{extension}"));
                        if candidate.is_file() {
                            return Ok(process::plain_path(&candidate));
                        }
                    }
                }
            }
        }
    }
    Err(Error::invalid(format!("找不到 MCP 命令 {command}，请填写可执行文件的绝对路径，或将它加入 PATH")))
}

/// DSH fails session creation when one of its MCP servers cannot start; point at the settings.
fn mcp_hint(error: Error, request: &StartSession) -> Error {
    match error {
        Error::Domain { kind, context } if !request.routing.mcp.is_empty() => {
            let names: Vec<&str> = request.routing.mcp.iter().map(|s| s.name.as_str()).collect();
            Error::new(kind, format!("{context}；若是 MCP 服务器（{}）无法启动，请检查 设置 → MCP 服务器", names.join("、")))
        }
        error => error,
    }
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
        let params = session_params(request, &cwd)?;
        let (key, reply) = match &request.resume {
            Some(_) if !resume => return Err(Error::invalid("这个 DSH 版本不支持恢复会话")),
            Some(key) => {
                let reply = self.call("session/resume", params, session::left(deadline));
                (key.as_str().to_owned(), reply.map_err(|e| mcp_hint(e, request))?)
            }
            None => {
                let reply = self.call("session/new", params, session::left(deadline)).map_err(|e| mcp_hint(e, request))?;
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
        let mut options = reply.get_mut("configOptions").map(Value::take).unwrap_or_default();
        // `routing.default_model` is only given for a new session (DSH starts at the `acp` entry's
        // model); its level is applied right after `session/new`.
        if let Some(level) = request.routing.default_model.as_ref().and_then(|m| m.effort) {
            options = self.set_option(effort_option(&key, &options, level)?, session::left(deadline))?;
        }
        let mut p = self.live.state.lock();
        p.set_key(NativeSessionKey::from(key));
        p.set_capabilities(Capabilities { resume, ..CAPABILITIES });
        p.set_model(model(&options));
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

    /// `session/set_config_option`; DSH answers with the complete resulting `configOptions`.
    fn set_option(&self, params: Value, timeout: Duration) -> Result<Value> {
        let mut reply = self.call("session/set_config_option", params, timeout)?;
        Ok(reply.get_mut("configOptions").map(Value::take).unwrap_or_default())
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

    /// Switches the model, then the effort the new model offers; the snapshot follows each reply.
    fn set_model(&self, provider: &str, model_id: &str, effort: Option<ThinkingLevel>) -> Result<()> {
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

/// `session/set_config_option` params selecting `provider`/`model_id`. The value is the option's
/// opaque id: the JSON-encoded `[provider, model]` DSH itself advertises.
fn model_option(session: &str, provider: &str, model_id: &str) -> Value {
    json!({"sessionId": session, "configId": MODEL_OPTION, "value": json!([provider, model_id]).to_string()})
}

/// `session/set_config_option` params setting the `thought_level` option of `options` to `level`.
fn effort_option(session: &str, options: &Value, level: ThinkingLevel) -> Result<Value> {
    let id = thought_level(options)
        .and_then(|option| option["id"].as_str())
        .ok_or_else(|| Error::invalid(format!("DSH 当前模型不支持设置推理力度（{}）", level_name(level))))?;
    Ok(json!({"sessionId": session, "configId": id, "value": level_name(level)}))
}

fn thought_level(options: &Value) -> Option<&Value> {
    options.as_array()?.iter().find(|o| o["category"] == "thought_level")
}

/// The model option's `currentValue` is a JSON-encoded `[provider, model]`; the effort is the
/// `thought_level` option's `currentValue` (`""` = provider default, recorded as `None`).
fn model(options: &Value) -> Option<ModelInfo> {
    let option = options.as_array()?.iter().find(|o| o["category"] == "model")?;
    let value = option["currentValue"].as_str()?;
    let effort = thought_level(options).and_then(|o| o["currentValue"].as_str()).and_then(rpc::thinking_level);
    match serde_json::from_str::<(String, String)>(value) {
        Ok((provider, id)) => Some(ModelInfo { provider: Some(provider), id, effort }),
        Err(_) => Some(ModelInfo { provider: None, id: value.to_owned(), effort }),
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
