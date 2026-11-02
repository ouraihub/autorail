//! Projection of the pi-family RPC event stream (pi `--mode rpc`, OMP `--mode rpc-ui`): both emit
//! the same `AgentMessage` shapes, assistant streaming events and `extension_ui_*` dialogs.

use std::{
    collections::HashMap,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use thinkrail_core::{
    ContentBlock, Error, ErrorKind, Interaction, InteractionAnswer, InteractionKind,
    InteractionOption, Message, ModelInfo, Result, Role, ThinkingLevel, ToolStatus,
};
use tracing::debug;

use crate::{
    projection::Projection,
    projection_engine::{LEVELS, level_name},
    session::{Live, engine_error, new_id, session_error},
};

const IMAGE: &str = "[图片]";

/// Streaming state of the message currently being produced.
#[derive(Default)]
pub(crate) struct Stream {
    current: Option<String>,
    /// Engine `contentIndex` → index in the projected message's content.
    blocks: HashMap<usize, usize>,
}

impl Stream {
    fn id(&mut self, frame: &Value) -> String {
        match frame["messageId"].as_str() {
            Some(id) => id.to_owned(),
            None => self.current.clone().unwrap_or_else(new_id),
        }
    }

    pub(crate) fn message_start(&mut self, p: &mut Projection, frame: &Value) {
        let id = frame["messageId"]
            .as_str()
            .map_or_else(new_id, str::to_owned);
        self.current = Some(id.clone());
        self.blocks.clear();
        if let Some(blocks) = put(p, &frame["message"], id, false) {
            self.blocks = blocks;
        }
    }

    pub(crate) fn message_end(&mut self, p: &mut Projection, frame: &Value) {
        let id = self.id(frame);
        put(p, &frame["message"], id, true);
        self.current = None;
        self.blocks.clear();
    }

    pub(crate) fn message_update(&mut self, p: &mut Projection, frame: &Value) {
        let id = self.id(frame);
        let event = &frame["assistantMessageEvent"];
        let index = event["contentIndex"]
            .as_u64()
            .and_then(|i| usize::try_from(i).ok());
        let kind = event["type"].as_str().unwrap_or("");
        if kind == "start" {
            if let Some(blocks) = put(p, &event["message"], id, false) {
                self.blocks = blocks;
            }
            return;
        }
        if !p.contains(&id) {
            p.upsert(Message {
                id: id.clone(),
                role: Role::Assistant,
                content: Vec::new(),
            });
        }
        let Some(index) = index else { return };
        let block = &event["block"];
        match kind {
            "text_start" => {
                self.open(
                    p,
                    &id,
                    index,
                    ContentBlock::Text {
                        text: str_of(&block["text"]),
                    },
                );
            }
            "thinking_start" => {
                self.open(
                    p,
                    &id,
                    index,
                    ContentBlock::Thinking {
                        text: str_of(&block["thinking"]),
                    },
                );
            }
            "toolcall_start" => {
                let tool_call_id = event["id"]
                    .as_str()
                    .or(block["id"].as_str())
                    .unwrap_or_default()
                    .to_owned();
                let name = event["toolName"]
                    .as_str()
                    .or(block["name"].as_str())
                    .unwrap_or_default()
                    .to_owned();
                let tool = ContentBlock::Tool {
                    tool_call_id,
                    name,
                    status: ToolStatus::Running,
                    input: None,
                    output: None,
                };
                self.open(p, &id, index, tool);
            }
            "text_delta" | "thinking_delta" => {
                let delta = event["delta"].as_str().unwrap_or_default();
                let target = match self.blocks.get(&index) {
                    Some(&target) => target,
                    None => {
                        let empty = if kind == "text_delta" {
                            ContentBlock::Text {
                                text: String::new(),
                            }
                        } else {
                            ContentBlock::Thinking {
                                text: String::new(),
                            }
                        };
                        self.open(p, &id, index, empty)
                    }
                };
                p.append(&id, target, delta);
            }
            "text_end" | "thinking_end" => {
                if let (Some(&target), Some(content)) =
                    (self.blocks.get(&index), event["content"].as_str())
                {
                    p.update(&id, |m| {
                        if let Some(ContentBlock::Text { text } | ContentBlock::Thinking { text }) =
                            m.content.get_mut(target)
                            && text != content
                        {
                            content.clone_into(text);
                        }
                    });
                }
            }
            "toolcall_end" => {
                if let Some(&target) = self.blocks.get(&index) {
                    let arguments = event["toolCall"]["arguments"].clone();
                    p.update(&id, |m| {
                        if let Some(ContentBlock::Tool { input, .. }) = m.content.get_mut(target) {
                            *input = Some(arguments);
                        }
                    });
                }
            }
            _ => {}
        }
    }

    fn open(&mut self, p: &mut Projection, id: &str, index: usize, block: ContentBlock) -> usize {
        let mut position = 0;
        p.update(id, |m| {
            position = m.content.len();
            m.content.push(block);
        });
        self.blocks.insert(index, position);
        position
    }
}

/// Applies a tool execution event (pi and OMP share them).
pub(crate) fn tool_event(p: &mut Projection, frame: &Value) {
    let Some(id) = frame["toolCallId"].as_str() else {
        return;
    };
    match frame["type"].as_str() {
        Some("tool_execution_start") => {
            p.set_tool(id, ToolStatus::Running, None);
        }
        Some("tool_execution_update") => {
            p.set_tool(
                id,
                ToolStatus::Running,
                Some(result_text(&frame["partialResult"])),
            );
        }
        Some("tool_execution_end") => {
            let status = if frame["isError"] == true {
                ToolStatus::Failed
            } else {
                ToolStatus::Completed
            };
            p.set_tool(id, status, Some(result_text(&frame["result"])));
        }
        _ => {}
    }
}

/// Replays a stored transcript page into a fresh projection.
pub(crate) fn replay(p: &mut Projection, messages: &[Value], first: usize) {
    for (offset, message) in messages.iter().enumerate() {
        put(p, message, format!("history-{}", first + offset), false);
    }
}

/// Projects one engine message. Tool results merge into the tool block that called them; an
/// assistant failure becomes a visible system note (and, when `live`, the session's last error).
/// Returns the content-index map of the projected message.
fn put(
    p: &mut Projection,
    message: &Value,
    id: String,
    live: bool,
) -> Option<HashMap<usize, usize>> {
    let role = message["role"].as_str()?;
    if role == "toolResult" {
        let tool_call_id = message["toolCallId"].as_str()?;
        let status = if message["isError"] == true {
            ToolStatus::Failed
        } else {
            ToolStatus::Completed
        };
        p.set_tool(tool_call_id, status, Some(result_text(message)));
        return None;
    }
    let (role, content, blocks) = match role {
        "user" => (
            Role::User,
            content_blocks(&message["content"]),
            HashMap::new(),
        ),
        "assistant" => {
            let (content, blocks) = assistant_blocks(&message["content"]);
            (Role::Assistant, content, blocks)
        }
        "custom" if message["display"] != true => return None,
        "custom" => (
            Role::System,
            content_blocks(&message["content"]),
            HashMap::new(),
        ),
        "bashExecution" => {
            let text = format!(
                "$ {}\n{}",
                str_of(&message["command"]),
                str_of(&message["output"])
            );
            (
                Role::System,
                vec![ContentBlock::Text { text }],
                HashMap::new(),
            )
        }
        "compactionSummary" | "branchSummary" => (
            Role::System,
            vec![ContentBlock::Text {
                text: str_of(&message["summary"]),
            }],
            HashMap::new(),
        ),
        _ => return None,
    };
    p.upsert(Message {
        id: id.clone(),
        role,
        content,
    });
    if message["stopReason"] == "error" {
        let error = engine_error(message["errorMessage"].as_str().unwrap_or("模型请求失败"));
        let note = Message {
            id: format!("{id}-error"),
            role: Role::System,
            content: vec![ContentBlock::Text {
                text: error.to_string(),
            }],
        };
        p.upsert(note);
        if live {
            p.error(session_error(&error));
        }
    }
    Some(blocks)
}

fn assistant_blocks(content: &Value) -> (Vec<ContentBlock>, HashMap<usize, usize>) {
    let mut blocks = Vec::new();
    let mut map = HashMap::new();
    for (index, block) in content.as_array().into_iter().flatten().enumerate() {
        let projected = match block["type"].as_str() {
            Some("text") => ContentBlock::Text {
                text: str_of(&block["text"]),
            },
            Some("thinking") => ContentBlock::Thinking {
                text: str_of(&block["thinking"]),
            },
            Some("image") => ContentBlock::Image {
                mime_type: str_of(&block["mimeType"]),
            },
            Some("toolCall") => ContentBlock::Tool {
                tool_call_id: str_of(&block["id"]),
                name: str_of(&block["name"]),
                status: ToolStatus::Running,
                input: Some(block["arguments"].clone()),
                output: None,
            },
            _ => continue,
        };
        map.insert(index, blocks.len());
        blocks.push(projected);
    }
    (blocks, map)
}

fn content_blocks(content: &Value) -> Vec<ContentBlock> {
    if let Some(text) = content.as_str() {
        return vec![ContentBlock::Text {
            text: text.to_owned(),
        }];
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| match block["type"].as_str() {
            Some("text") => Some(ContentBlock::Text {
                text: str_of(&block["text"]),
            }),
            Some("image") => Some(ContentBlock::Image {
                mime_type: str_of(&block["mimeType"]),
            }),
            _ => None,
        })
        .collect()
}

/// Model-facing text of a tool result (`{ content: [...] }`).
pub(crate) fn result_text(result: &Value) -> String {
    match &result["content"] {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item["type"].as_str() {
                Some("text") => item["text"].as_str().map(str::to_owned),
                Some("image") => Some(IMAGE.to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The current model of a pi/OMP `get_state` (or `config_update`) payload, with its thinking level.
pub(crate) fn model(state: &Value) -> Option<ModelInfo> {
    let model = &state["model"];
    let id = model["id"].as_str()?;
    Some(ModelInfo {
        provider: model["provider"].as_str().map(str::to_owned),
        id: id.to_owned(),
        effort: state["thinkingLevel"].as_str().and_then(thinking_level),
    })
}

/// The host level an engine spells `name`; `None` for anything else (OMP `inherit`, DSH's
/// provider-default `""`, unknown future levels).
pub(crate) fn thinking_level(name: &str) -> Option<ThinkingLevel> {
    LEVELS.into_iter().find(|&level| level_name(level) == name)
}

/// The pi-family commands (without `type`/`id`) that switch this session's model: `set_model`,
/// then `set_thinking_level` when a level is given. Nothing is persisted (RPC sends no `persist`).
pub(crate) fn switch_commands(
    provider: &str,
    model_id: &str,
    effort: Option<ThinkingLevel>,
) -> Vec<(&'static str, Value)> {
    let mut commands = vec![(
        "set_model",
        json!({"provider": provider, "modelId": model_id}),
    )];
    if let Some(level) = effort {
        commands.push(("set_thinking_level", json!({"level": level_name(level)})));
    }
    commands
}

/// Switches a pi/OMP session's model through `call`, then re-reads `get_state` so the snapshot
/// shows what the engine actually applied (it clamps the level to what the model supports).
pub(crate) fn switch_model(
    live: &Live,
    call: impl Fn(&str, Value) -> Result<Value>,
    provider: &str,
    model_id: &str,
    effort: Option<ThinkingLevel>,
) -> Result<()> {
    live.ensure_idle()?;
    for (kind, fields) in switch_commands(provider, model_id, effort) {
        call(kind, fields)?;
    }
    let state = call("get_state", json!({}))?;
    live.state.lock().set_model(model(&state));
    Ok(())
}

/// Folds an engine-initiated model change into the snapshot (publishes only on a change):
/// `thinking_level_changed {level | thinkingLevel}` (pi | OMP) and OMP `config_update`.
pub(crate) fn model_event(p: &mut Projection, frame: &Value) {
    if frame["type"] == "config_update" && frame["model"].is_object() {
        p.set_model(model(frame));
        return;
    }
    let level = frame
        .get("thinkingLevel")
        .or_else(|| frame.get("level"))
        .and_then(Value::as_str);
    let Some(level) = level else { return };
    if let Some(current) = p.model() {
        let updated = ModelInfo {
            effort: thinking_level(level),
            ..current.clone()
        };
        p.set_model(Some(updated));
    }
}

/// What an `extension_ui_request` asks of the host.
pub(crate) enum UiRequest {
    Show(Interaction, Option<Duration>),
    /// OMP withdrew an earlier request.
    Withdraw(String),
    /// Status/widget/notify frames carry no question.
    Ignore,
}

pub(crate) fn ui_request(frame: &Value) -> Result<UiRequest> {
    let method = frame["method"].as_str().unwrap_or("");
    if method == "cancel" {
        return Ok(frame["targetId"]
            .as_str()
            .map_or(UiRequest::Ignore, |t| UiRequest::Withdraw(t.to_owned())));
    }
    let kind = match method {
        "select" => InteractionKind::Select,
        "confirm" => InteractionKind::Confirm,
        "input" => InteractionKind::Input,
        "editor" => InteractionKind::Editor,
        "ask" => InteractionKind::Ask,
        _ => return Ok(UiRequest::Ignore),
    };
    let request_id = frame["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::protocol("交互请求缺少 id"))?;
    let options = match kind {
        InteractionKind::Select => frame["options"]
            .as_array()
            .ok_or_else(|| Error::protocol("select 请求缺少 options"))?
            .iter()
            .enumerate()
            .map(|(index, label)| InteractionOption {
                id: index.to_string(),
                label: str_of(label),
            })
            .collect(),
        InteractionKind::Permission
        | InteractionKind::Confirm
        | InteractionKind::Input
        | InteractionKind::Editor
        | InteractionKind::Ask => Vec::new(),
    };
    if kind == InteractionKind::Ask && !frame["questions"].as_array().is_some_and(|q| !q.is_empty())
    {
        return Err(Error::protocol("ask 请求缺少 questions"));
    }
    let message = match kind {
        InteractionKind::Confirm => str_of(&frame["message"]),
        InteractionKind::Input => str_of(&frame["placeholder"]),
        InteractionKind::Editor => str_of(&frame["prefill"]),
        InteractionKind::Permission | InteractionKind::Select | InteractionKind::Ask => {
            String::new()
        }
    };
    let interaction = Interaction {
        request_id: request_id.to_owned(),
        kind,
        title: str_of(&frame["title"]),
        message,
        options,
        detail: frame.clone(),
    };
    let timeout = frame["timeout"]
        .as_u64()
        .filter(|&ms| ms > 0)
        .map(Duration::from_millis);
    Ok(UiRequest::Show(interaction, timeout))
}

/// The `extension_ui_response` answering `interaction`. The answer must fit the question.
pub(crate) fn ui_response(interaction: &Interaction, answer: &InteractionAnswer) -> Result<Value> {
    let id = &interaction.request_id;
    let wrong = || Error::invalid("回答的类型与问题不匹配");
    Ok(match (interaction.kind, answer) {
        (_, InteractionAnswer::Cancel) => cancel(id),
        (InteractionKind::Confirm, InteractionAnswer::Confirm { confirmed }) => {
            json!({"type":"extension_ui_response","id":id,"confirmed":confirmed})
        }
        (InteractionKind::Select, InteractionAnswer::Select { option_id }) => {
            let option = interaction
                .options
                .iter()
                .find(|o| &o.id == option_id)
                .ok_or_else(wrong)?;
            json!({"type":"extension_ui_response","id":id,"value":option.label})
        }
        (InteractionKind::Input | InteractionKind::Editor, InteractionAnswer::Text { text }) => {
            json!({"type":"extension_ui_response","id":id,"value":text})
        }
        (InteractionKind::Ask, InteractionAnswer::Answers { answers }) => {
            let expected = interaction.detail["questions"]
                .as_array()
                .map_or(0, Vec::len);
            if answers.as_array().map(Vec::len) != Some(expected) {
                return Err(Error::invalid(format!(
                    "需要按顺序回答全部 {expected} 个问题"
                )));
            }
            json!({"type":"extension_ui_response","id":id,"answers":answers})
        }
        _ => return Err(wrong()),
    })
}

pub(crate) fn cancel(id: &str) -> Value {
    json!({"type":"extension_ui_response","id":id,"cancelled":true})
}

/// Engine dialogs of one session. A dialog with a `timeout` is resolved by the engine itself when
/// it expires, so its card is dropped then and a late answer is `InteractionExpired`.
#[derive(Default)]
pub(crate) struct Dialogs {
    deadlines: Mutex<HashMap<String, Instant>>,
}

impl Dialogs {
    pub(crate) fn on_request(&self, live: &Arc<Live>, frame: &Value) -> Result<()> {
        match ui_request(frame)? {
            UiRequest::Show(interaction, timeout) => {
                if let Some(timeout) = timeout {
                    self.deadlines
                        .lock()
                        .insert(interaction.request_id.clone(), Instant::now() + timeout);
                    expire_later(live, interaction.request_id.clone(), timeout);
                }
                live.state.lock().add_interaction(interaction);
            }
            UiRequest::Withdraw(id) => {
                self.deadlines.lock().remove(&id);
                let mut p = live.state.lock();
                if p.take_interaction(&id).is_some() {
                    p.publish();
                }
            }
            UiRequest::Ignore => {}
        }
        Ok(())
    }

    /// Sends the user's answer; the card leaves only once the engine has it.
    pub(crate) fn respond(
        &self,
        live: &Live,
        request_id: &str,
        answer: &InteractionAnswer,
    ) -> Result<()> {
        live.ensure_open()?;
        let expired = self
            .deadlines
            .lock()
            .get(request_id)
            .is_some_and(|d| *d <= Instant::now());
        let mut p = live.state.lock();
        let Some(interaction) = p.take_interaction(request_id) else {
            return Err(expired_error());
        };
        if expired {
            p.publish();
            return Err(expired_error());
        }
        let sent = ui_response(&interaction, answer).and_then(|frame| live.process.send(&frame));
        if let Err(error) = sent {
            p.restore_interaction(interaction);
            return Err(error);
        }
        self.deadlines.lock().remove(request_id);
        p.publish();
        Ok(())
    }

    /// Cancels every open dialog (dispose path).
    pub(crate) fn cancel_all(&self, live: &Live) {
        self.deadlines.lock().clear();
        let open = live.state.lock().take_interactions();
        for interaction in open {
            if let Err(error) = live.process.send(&cancel(&interaction.request_id)) {
                debug!(%error, "cannot cancel dialog during dispose");
            }
        }
    }
}

fn expire_later(live: &Arc<Live>, id: String, timeout: Duration) {
    let live = Arc::downgrade(live);
    let spawned = thread::Builder::new()
        .name("agent-dialog-timeout".into())
        .spawn(move || {
            thread::sleep(timeout);
            if let Some(live) = live.upgrade() {
                let mut p = live.state.lock();
                if p.take_interaction(&id).is_some() {
                    p.publish();
                }
            }
        });
    if let Err(error) = spawned {
        debug!(%error, "cannot schedule dialog expiry; the card stays until answered");
    }
}

pub(crate) fn expired_error() -> Error {
    Error::new(ErrorKind::InteractionExpired, "该问题已超时或已被回答")
}

fn str_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests;
