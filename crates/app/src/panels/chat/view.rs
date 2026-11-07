//! Chat column rendering: tab strip, model header, transcript body and composer.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::input::Textarea,
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{BackendKind, ChatId, SessionPhase};
use thinkrail_ui::{
    Roles, ThinkrailTheme,
    components::{IconButton, Status, StatusDot, Tab, TabStrip, label},
    metrics::{radius, size, space, text},
};

use super::{Chat, ChatPanel, empty, plan, transcript};
use crate::{panels::agent_guide, strings::chat as s};

impl ChatPanel {
    pub(super) fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut strip = TabStrip::new();
        for id in &self.order {
            let Some(chat) = self.chats.get(id) else {
                continue;
            };
            let (select, close) = (id.clone(), id.clone());
            strip = strip.tab(
                Tab::new(format!("chat-tab-{id}"), chat.title())
                    .icon(IconName::MessageSquare)
                    .active(self.active.as_ref() == Some(id))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = Some(select.clone());
                        this.scroll.scroll_to_bottom();
                        cx.notify();
                    }))
                    .on_close(cx.listener(move |this, _, _, cx| this.close_chat(&close, cx))),
            );
        }
        strip.action(
            IconButton::new("chat-new", IconName::MessageCirclePlus)
                .tooltip(s::NEW_CHAT)
                .disabled(self.workspace.is_none())
                .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
        )
    }

    pub(super) fn chat_body(
        &mut self,
        id: &ChatId,
        r: Roles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_model_picker(id, cx);
        self.sync_usage_view(id, cx);
        let Some(chat) = self.chats.get(id) else {
            return empty(s::NO_CHATS, &r);
        };
        let phase = chat.phase();
        let header = self.header(chat, phase, r).into_any_element();
        let snapshot = chat.snapshot.clone();
        let (starting, blocked, bound) = (
            chat.starting || chat.closing,
            chat.blocked,
            chat.binding.is_some(),
        );
        let guide = blocked.and_then(|readiness| {
            agent_guide::row("chat-blocked", self.state.clone(), readiness, cx)
        });
        let entity = cx.entity().downgrade();
        let toggle: transcript::Toggle = Rc::new(move |key, _, cx| {
            if let Some(panel) = entity.upgrade() {
                panel.update(cx, |panel, cx| {
                    if !panel.open.remove(&key) {
                        panel.open.insert(key);
                    }
                    cx.notify();
                });
            }
        });
        let messages: Vec<AnyElement> = snapshot
            .as_ref()
            .map(|s| {
                s.messages
                    .iter()
                    .map(|m| transcript::message(m, &self.open, &toggle, &r))
                    .collect()
            })
            .unwrap_or_default();
        let truncated = snapshot.as_ref().is_some_and(|s| s.output_truncated);
        let plan_key = format!("plan/{id}");
        let reviews = &chat.review_cards;
        let plan_strip = chat.plan.as_ref().and_then(|p| {
            plan::strip(
                &plan_key,
                p,
                reviews,
                self.open.contains(&plan_key),
                &toggle,
                &r,
            )
        });
        let interactions: Vec<AnyElement> = snapshot
            .as_ref()
            .map(|s| s.interactions.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|i| self.interaction_card(i, window, cx))
            .collect();
        let status_line = if starting {
            Some(s::STARTING)
        } else if blocked.is_some() {
            None
        } else if phase == SessionPhase::Closed && bound {
            Some(s::CLOSED_HINT)
        } else {
            None
        };
        v_flex()
            .size_full()
            .child(header)
            .children(plan_strip)
            .child(
                v_flex()
                    .id("chat-transcript")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(size::CHAT_GUTTER + space::S8)
                    .py(space::S12)
                    .gap(space::S16)
                    .when(truncated, |d| {
                        d.child(label(text::METADATA, s::TRUNCATED).text_color(r.text_subtle))
                    })
                    .children(messages)
                    .children(interactions)
                    .when_some(status_line, |d, line| {
                        d.child(label(text::METADATA, line).text_color(r.text_subtle))
                    })
                    .children(guide),
            )
            .child(self.composer_box(phase, starting || blocked.is_some(), cx))
            .into_any_element()
    }

    fn composer_box(
        &self,
        phase: SessionPhase,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let busy = matches!(phase, SessionPhase::Running | SessionPhase::Waiting);
        let action = if busy {
            IconButton::new("chat-stop", IconName::Square)
                .tooltip(s::STOP)
                .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
        } else {
            IconButton::new("chat-send", IconName::ArrowUp)
                .tooltip(s::SEND)
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
        };
        let menu = self.slash_menu(cx);
        let textarea = self.slash_keys(Textarea::new(&self.composer).appearance(false), cx);
        // `relative` anchors the `/` menu, which sits above this box over the transcript.
        div()
            .relative()
            .p(size::CHAT_GUTTER)
            .border_t_1()
            .border_color(r.border_muted)
            .child(
                v_flex()
                    .gap(space::S6)
                    .p(space::S8)
                    .rounded(radius::LG)
                    .border_1()
                    .border_color(r.control_border_default)
                    .bg(r.control_bg)
                    .child(textarea)
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(space::S8)
                            .child(self.model_picker.view())
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w(px(160.))
                                    .gap(space::S4)
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .justify_end()
                                            .overflow_hidden()
                                            .child(self.usage_view.clone()),
                                    )
                                    .child(action),
                            ),
                    ),
            )
            .children(menu)
    }
    fn header(&self, chat: &Chat, phase: SessionPhase, r: Roles) -> AnyElement {
        let backend = chat
            .binding
            .as_ref()
            .map(|b| b.backend)
            .or(chat.snapshot.as_ref().map(|s| s.backend));
        let backend_label = backend.map(|b| match b {
            BackendKind::Dsh => format!("{} · {}", b.label(), s::PRERELEASE),
            BackendKind::Pi | BackendKind::Omp => b.label().to_owned(),
        });
        let (status, phase_label) = match phase {
            SessionPhase::Idle => (Status::Success, s::PHASE_IDLE),
            SessionPhase::Running => (Status::Info, s::PHASE_RUNNING),
            SessionPhase::Waiting => (Status::Warning, s::PHASE_WAITING),
            SessionPhase::Closed => (Status::Idle, s::PHASE_CLOSED),
        };
        h_flex()
            .h(size::PANEL_HEADER)
            .flex_none()
            .px(size::PANEL_PADDING)
            .gap(space::S8)
            .border_b_1()
            .border_color(r.border_muted)
            .children(backend_label.map(|b| label(text::LABEL_PILL, b).text_color(r.primary)))
            .child(StatusDot::new(status).label(phase_label))
            .into_any_element()
    }
}
