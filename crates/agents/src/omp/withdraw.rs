//! Withdrawal of a request the user stopped before OMP answered it.

use std::{collections::HashMap, sync::atomic::Ordering};

use serde_json::{Value, json};
use thinkrail_core::ErrorCode as _;
use tracing::{info, warn};

use super::OmpSession;
use crate::session::COMMAND;

impl OmpSession {
    /// OMP drops an aborted reply with no text or tool call from the model context but keeps its
    /// request, so the model would still answer a request the user stopped. Moving the leaf back
    /// before that request withdraws it; the host transcript keeps showing it.
    pub(super) fn withdraw_stopped_request(&self) {
        if !self.interrupted.swap(false, Ordering::AcqRel) {
            return;
        }
        let withdrawn = self
            .call("get_entries", json!({}), COMMAND)
            .and_then(|data| {
                let Some(request) = stopped_request(&data["entries"], data["leafId"].as_str())
                else {
                    return Ok(false);
                };
                self.call(
                    "navigate_tree",
                    json!({"entryId": request, "summarize": false}),
                    COMMAND,
                )
                .map(|_| true)
            });
        match withdrawn {
            Ok(true) => info!("stopped request withdrawn from the OMP context"),
            Ok(false) => {}
            Err(error) => warn!(code = error.code(), "cannot withdraw the stopped request"),
        }
    }
}

/// The earliest user entry of the trailing run of requests whose only replies are aborted without
/// text or tool calls, walking the active path back from `leaf`. Those requests reach the model
/// unanswered.
pub(super) fn stopped_request(entries: &Value, leaf: Option<&str>) -> Option<String> {
    let by_id: HashMap<&str, &Value> = entries
        .as_array()?
        .iter()
        .filter_map(|e| e["id"].as_str().map(|id| (id, e)))
        .collect();
    let mut cursor = leaf;
    let (mut stopped, mut earliest) = (false, None);
    while let Some(entry) = cursor.and_then(|id| by_id.get(id)) {
        if entry["type"] == "message" {
            let message = &entry["message"];
            match message["role"].as_str() {
                Some("assistant")
                    if message["stopReason"] == "aborted" && !answers(&message["content"]) =>
                {
                    stopped = true;
                }
                Some("user") if stopped => {
                    earliest = entry["id"].as_str().map(str::to_owned);
                    stopped = false;
                }
                _ => break,
            }
        }
        cursor = entry["parentId"].as_str();
    }
    earliest
}

fn answers(content: &Value) -> bool {
    content.as_array().is_some_and(|blocks| {
        blocks.iter().any(|b| {
            b["type"] == "toolCall"
                || (b["type"] == "text" && b["text"].as_str().is_some_and(|t| !t.trim().is_empty()))
        })
    })
}
