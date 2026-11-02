//! Live session traffic: prompts, stop, interaction answers, plan reloads and session events.

use gpui_kit::{Context, Window};
use thinkrail_core::{ChatId, InteractionAnswer, PlanScope, SessionEvent, SessionPhase};
use tracing::debug;

use super::{ChatPanel, engine::dispose_engine, projection};
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
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        match chat.live().cloned() {
            Some(session) => run(
                cx,
                move || session.prompt(&text),
                |this, result, cx| {
                    if let Err(error) = result {
                        this.report(&error, cx);
                    }
                },
            ),
            None => self.start(id, Some(text), cx),
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
            debug!(chat = %id, "event for an unknown chat");
            return;
        };
        if projection::apply(&mut chat.snapshot, event) {
            if chat.restart_pending && chat.phase() == SessionPhase::Idle {
                chat.restart_pending = false;
                chat.closing = true;
                let session = chat.session.take();
                dispose_engine(&services, id.clone(), session, false, cx);
            }
            if self.active.as_ref() == Some(&id) {
                self.scroll.scroll_to_bottom();
                cx.notify();
            }
        }
    }
}
