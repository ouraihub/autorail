//! Projection of the pi-family RPC event stream (pi `--mode rpc`, OMP `--mode rpc-ui`): both emit
//! the same `AgentMessage` shapes, assistant streaming events and `extension_ui_*` dialogs.

use std::collections::HashMap;

use serde_json::Value;
use thinkrail_core::{ContentBlock, Message, Role, ToolStatus};

use crate::{
    projection::Projection,
    session::{engine_error, new_id, session_error},
};

pub(crate) mod commands;
mod dialogs;
mod model;
pub(crate) mod usage;

pub(crate) use dialogs::Dialogs;
pub(crate) use dialogs::cancel;
pub(crate) use model::{model, model_event, switch_model, thinking_level};

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

fn str_of(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests;
