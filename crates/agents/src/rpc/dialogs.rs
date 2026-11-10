//! Engine `extension_ui_*` dialogs: request parsing, answer frames and per-session expiry.

use std::{
    collections::HashMap,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use thinkrail_core::{
    Error, ErrorKind, Interaction, InteractionAnswer, InteractionKind, InteractionOption, Result,
};
use tracing::debug;

use super::str_of;
use crate::session::Live;

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
    let trace = live.process.trace.clone();
    let live = Arc::downgrade(live);
    let spawned = thread::Builder::new()
        .name("agent-dialog-timeout".into())
        .spawn(move || {
            let _entered = trace.enter("agent.dialog.timeout");
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
