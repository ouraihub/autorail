//! Live session traffic: prompts and host slash commands, stop, interaction answers, plan
//! reloads and session events.

use gpui_kit::{Context, Window};
use thinkrail_core::{ChatId, Error, InteractionAnswer, PlanScope, SessionEvent, SessionPhase};
use tracing::debug;

use super::{
    ChatPanel,
    commands::{self, ModelChange, Submit},
    engine::dispose_engine,
    projection,
};
use crate::state::run;

impl ChatPanel {
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let Some(id) = self.active.clone() else {
            return;
        };
        let Some(chat) = self.chats.get(&id) else {
            return;
        };
        if chat.blocked.is_some() || chat.starting || chat.closing {
            return;
        }
        let host = self
            .host_offer(chat, cx)
            .map(|(_, host)| host)
            .unwrap_or_default();
        if let Some(parsed) = commands::parse(&text, &host) {
            let accepted = match parsed {
                Ok(submit) => self.run_host(&id, submit, cx),
                Err(message) => {
                    self.report(&Error::invalid(message), cx);
                    false
                }
            };
            if accepted {
                self.composer
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            return;
        }
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        let session = chat.live().cloned();
        if let Some(chat) = self.chats.get_mut(&id) {
            chat.usage.invalidate();
        }
        match session {
            Some(session) => {
                let id = id.clone();
                run(
                    cx,
                    move || session.prompt(&text),
                    move |this, result, cx| {
                        if let Err(error) = result {
                            this.report(&error, cx);
                        }
                        this.refresh_usage(&id, cx);
                    },
                );
            }
            None => self.start(id, Some(text), cx),
        }
    }

    /// Runs a host built-in; `true` when it was dispatched, so the draft can be cleared. Without
    /// a live engine the chat starts and the draft stays for a second Enter (07 C8).
    fn run_host(&mut self, id: &ChatId, submit: Submit, cx: &mut Context<Self>) -> bool {
        let Some(chat) = self.chats.get(id) else {
            return false;
        };
        let Some(session) = chat.live().cloned() else {
            self.start(id.clone(), None, cx);
            return false;
        };
        let report =
            |this: &mut Self, result: thinkrail_core::Result<()>, cx: &mut Context<Self>| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            };
        match submit {
            Submit::Compact { instructions } => {
                let id = id.clone();
                if let Some(chat) = self.chats.get_mut(&id) {
                    chat.usage.invalidate();
                    chat.usage.compacting = true;
                }
                run(
                    cx,
                    move || session.compact(instructions.as_deref()),
                    move |this, result, cx| {
                        if let Some(chat) = this.chats.get_mut(&id) {
                            chat.usage.compacting = false;
                        }
                        report(this, result, cx);
                        this.refresh_usage(&id, cx);
                        cx.notify();
                    },
                );
                true
            }
            Submit::Name { title } => {
                run(cx, move || session.rename(&title), report);
                true
            }
            Submit::Model { query, effort } => {
                let current = chat.snapshot.as_ref().and_then(|s| s.model.as_ref());
                match commands::resolve_model(&query, effort, &chat.catalog, current) {
                    Ok(ModelChange::Switch { choice, effort }) => {
                        self.set_model(id.clone(), choice, effort, cx)
                    }
                    Ok(ModelChange::Effort(effort)) => self.set_effort(id.clone(), effort, cx),
                    Err(message) => {
                        self.report(&Error::invalid(message), cx);
                        false
                    }
                }
            }
        }
    }

    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self
            .active
            .as_ref()
            .and_then(|id| self.chats.get(id))
            .and_then(|c| c.live().cloned())
        else {
            return;
        };
        run(
            cx,
            move || session.stop(),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }

    pub(super) fn respond(
        &mut self,
        request_id: String,
        answer: InteractionAnswer,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .active
            .as_ref()
            .and_then(|id| self.chats.get(id))
            .and_then(|c| c.live().cloned())
        else {
            return;
        };
        self.answers.retain(|key, _| !key.starts_with(&request_id));
        self.picks.retain(|key, _| !key.starts_with(&request_id));
        run(
            cx,
            move || session.respond(&request_id, answer),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }

    /// A host tool call may have changed this chat's plan.
    pub fn on_plan_changed(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        if self.chats.contains_key(id) {
            self.load_plan(id, cx);
        }
    }

    pub(super) fn load_plan(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(id) else {
            return;
        };
        let scope = PlanScope {
            workspace: chat.workspace.clone(),
            chat: id.clone(),
        };
        let plans = self.services(cx).plans;
        let id = id.clone();
        run(
            cx,
            move || plans.plan(&scope),
            move |this, result, cx| match result {
                Ok(plan) => {
                    if let Some(chat) = this.chats.get_mut(&id) {
                        chat.plan = Some(plan);
                        cx.notify();
                    }
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    pub fn on_session_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        let services = self.services(cx);
        let id = event.chat().clone();
        let Some(chat) = self.chats.get_mut(&id) else {
            debug!(chat_id = %id, "chat.event.unknown_chat");
            return;
        };
        let previous_phase = chat.phase();
        let previous_model = chat.snapshot.as_ref().and_then(|s| s.model.clone());
        if projection::apply(&mut chat.snapshot, event) {
            let phase = chat.phase();
            let model_changed =
                chat.snapshot.as_ref().and_then(|s| s.model.as_ref()) != previous_model.as_ref();
            if model_changed
                || (previous_phase == SessionPhase::Idle && phase != SessionPhase::Idle)
            {
                chat.usage.invalidate();
            }
            let refresh_usage =
                phase == SessionPhase::Idle && (previous_phase != phase || model_changed);
            if chat.restart_pending && chat.phase() == SessionPhase::Idle {
                chat.restart_pending = false;
                chat.closing = true;
                let session = chat.session.take();
                dispose_engine(&services, id.clone(), session, false, cx);
            }
            if refresh_usage {
                self.refresh_usage(&id, cx);
            }
            if self.active.as_ref() == Some(&id) {
                self.scroll.scroll_to_bottom();
                cx.notify();
            }
        }
    }
}
