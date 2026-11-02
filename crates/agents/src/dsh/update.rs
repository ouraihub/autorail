//! Projection of ACP `session/update` notifications and of DSH config options (model and
//! thinking level) into the session snapshot, plus the `session/set_config_option` params.

use serde_json::{Value, json};
use thinkrail_core::{
    ContentBlock, Error, Message, ModelInfo, Result, Role, ThinkingLevel, ToolStatus,
};

use super::str_of;
use crate::{projection::Projection, projection_engine::level_name, rpc, session::new_id};

const MODEL_OPTION: &str = "model";

fn chunk_block(content: &Value, thought: bool) -> Option<ContentBlock> {
    match content["type"].as_str()? {
        "text" if thought => Some(ContentBlock::Thinking {
            text: str_of(&content["text"]),
        }),
        "text" => Some(ContentBlock::Text {
            text: str_of(&content["text"]),
        }),
        "image" => Some(ContentBlock::Image {
            mime_type: str_of(&content["mimeType"]),
        }),
        _ => None,
    }
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

/// `session/set_config_option` params selecting `provider`/`model_id`. The value is the option's
/// opaque id: the JSON-encoded `[provider, model]` DSH itself advertises.
pub(super) fn model_option(session: &str, provider: &str, model_id: &str) -> Value {
    json!({"sessionId": session, "configId": MODEL_OPTION, "value": json!([provider, model_id]).to_string()})
}

/// `session/set_config_option` params setting the `thought_level` option of `options` to `level`.
pub(super) fn effort_option(session: &str, options: &Value, level: ThinkingLevel) -> Result<Value> {
    let id = thought_level(options)
        .and_then(|option| option["id"].as_str())
        .ok_or_else(|| {
            Error::invalid(format!(
                "DSH 当前模型不支持设置推理力度（{}）",
                level_name(level)
            ))
        })?;
    Ok(json!({"sessionId": session, "configId": id, "value": level_name(level)}))
}

fn thought_level(options: &Value) -> Option<&Value> {
    options
        .as_array()?
        .iter()
        .find(|o| o["category"] == "thought_level")
}

/// The model option's `currentValue` is a JSON-encoded `[provider, model]`; the effort is the
/// `thought_level` option's `currentValue` (`""` = provider default, recorded as `None`).
pub(super) fn model(options: &Value) -> Option<ModelInfo> {
    let option = options
        .as_array()?
        .iter()
        .find(|o| o["category"] == "model")?;
    let value = option["currentValue"].as_str()?;
    let effort = thought_level(options)
        .and_then(|o| o["currentValue"].as_str())
        .and_then(rpc::thinking_level);
    match serde_json::from_str::<(String, String)>(value) {
        Ok((provider, id)) => Some(ModelInfo {
            provider: Some(provider),
            id,
            effort,
        }),
        Err(_) => Some(ModelInfo {
            provider: None,
            id: value.to_owned(),
            effort,
        }),
    }
}

/// Projects one ACP `session/update`. Chunks are committed blocks (not token deltas): every chunk
/// of one `messageId` appends a block to that message.
pub(super) fn apply_update(p: &mut Projection, update: &Value) {
    match update["sessionUpdate"].as_str() {
        Some(kind @ ("agent_message_chunk" | "agent_thought_chunk" | "user_message_chunk")) => {
            let Some(block) = chunk_block(&update["content"], kind == "agent_thought_chunk") else {
                return;
            };
            let role = if kind == "user_message_chunk" {
                Role::User
            } else {
                Role::Assistant
            };
            let id = update["messageId"]
                .as_str()
                .map_or_else(new_id, str::to_owned);
            if !p.update(&id, |m| m.content.push(block.clone())) {
                p.upsert(Message {
                    id,
                    role,
                    content: vec![block],
                });
            }
        }
        Some("tool_call") => {
            let Some(tool_call_id) = update["toolCallId"].as_str() else {
                return;
            };
            let tool = ContentBlock::Tool {
                tool_call_id: tool_call_id.to_owned(),
                name: str_of(&update["title"]),
                status: tool_status(&update["status"]).unwrap_or(ToolStatus::Running),
                input: update.get("rawInput").cloned(),
                output: None,
            };
            p.upsert(Message {
                id: format!("tool-{tool_call_id}"),
                role: Role::Assistant,
                content: vec![tool],
            });
        }
        Some("tool_call_update") => {
            let Some(tool_call_id) = update["toolCallId"].as_str() else {
                return;
            };
            let status = tool_status(&update["status"]).unwrap_or(ToolStatus::Running);
            let output = update["content"].as_array().map(|items| tool_output(items));
            p.set_tool(tool_call_id, status, output);
        }
        Some("config_option_update") => p.set_model(model(&update["configOptions"])),
        _ => {}
    }
}
