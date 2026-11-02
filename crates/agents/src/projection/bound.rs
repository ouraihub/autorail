//! Size accounting and bounding of projected messages, and carrying tool results across a
//! message replacement.

use thinkrail_core::{ContentBlock, Message, ToolStatus};

pub(super) const MAX_MESSAGE_BYTES: usize = 256 * 1024;
pub(super) const TRUNCATED: &str = "\n…[已截断]";

pub(super) fn carry_tool_results(old: &Message, new: &mut Message) {
    for block in &mut new.content {
        let ContentBlock::Tool {
            tool_call_id,
            status,
            output,
            ..
        } = block
        else {
            continue;
        };
        if *status != ToolStatus::Running || output.is_some() {
            continue;
        }
        let previous = old.content.iter().find_map(|b| match b {
            ContentBlock::Tool {
                tool_call_id: id,
                status,
                output,
                ..
            } if id == tool_call_id => Some((status, output)),
            ContentBlock::Tool { .. }
            | ContentBlock::Text { .. }
            | ContentBlock::Thinking { .. }
            | ContentBlock::Image { .. } => None,
        });
        if let Some((old_status, old_output)) = previous {
            *status = *old_status;
            output.clone_from(old_output);
        }
    }
}

pub(super) fn size_of(message: &Message) -> usize {
    message
        .content
        .iter()
        .map(|block| {
            64 + match block {
                ContentBlock::Text { text } | ContentBlock::Thinking { text } => text.len(),
                ContentBlock::Image { mime_type } => mime_type.len(),
                ContentBlock::Tool {
                    name,
                    input,
                    output,
                    ..
                } => {
                    name.len()
                        + input.as_ref().map_or(0, |v| v.to_string().len())
                        + output.as_ref().map_or(0, String::len)
                }
            }
        })
        .sum()
}

/// Shrinks an oversized message so each block fits an equal share of the per-message bound.
pub(super) fn bound(message: &mut Message) -> bool {
    if size_of(message) <= MAX_MESSAGE_BYTES {
        return false;
    }
    let share = MAX_MESSAGE_BYTES / message.content.len().max(1);
    for block in &mut message.content {
        match block {
            ContentBlock::Text { text } | ContentBlock::Thinking { text } => cut(text, share),
            ContentBlock::Image { .. } => {}
            ContentBlock::Tool { input, output, .. } => {
                if let Some(value) = input.as_mut() {
                    let text = value.to_string();
                    if text.len() > share / 2 {
                        let mut text = text;
                        cut(&mut text, share / 2);
                        *value = serde_json::Value::String(text);
                    }
                }
                if let Some(text) = output.as_mut() {
                    cut(text, share / 2);
                }
            }
        }
    }
    true
}

fn cut(text: &mut String, limit: usize) {
    if text.len() <= limit {
        return;
    }
    let mut end = limit.saturating_sub(TRUNCATED.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(TRUNCATED);
}
