//! Chat column (04 §8, §9.2): one tab per chat of the current workspace, the live transcript,
//! interaction cards (never auto-answered) and the composer. A chat is pinned to the backend it
//! was created with; a closed engine is resumed from its native locator on send.

mod commands;
mod engine;
mod interaction;
mod model_picker;
mod plan;
mod projection;
mod routing;
mod session;
mod slash;
mod tabs;
mod transcript;
mod usage;
mod view;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    ScrollHandle, Styled as _, Subscription, Window,
    base::v_flex,
    component::input::{InputEvent, InputState, TextareaState},
    div, px,
};
use thinkrail_core::{
    AgentSession, ChatBinding, ChatId, Error, ModelChoice, Plan, ProjectId, Readiness,
    SessionPhase, SessionSink, SessionSnapshot, WorkspaceId,
};
use thinkrail_ui::{
    Roles, ThinkrailTheme,
    components::label,
    metrics::{space, text},
};

use super::agent_guide;
use crate::{
    services::Services,
    state::{AppEvent, AppState},
    strings::chat as s,
};

struct Chat {
    workspace: WorkspaceId,
    binding: Option<ChatBinding>,
    session: Option<Arc<dyn AgentSession>>,
    snapshot: Option<SessionSnapshot>,
    usage: usage::Usage,
    routing_signature: Option<String>,
    catalog: Vec<ModelChoice>,
    restart_pending: bool,
    closing: bool,
    starting: bool,
    /// Set when this chat's engine is not installed (07 §5.3): history stays, sending is off.
    blocked: Option<Readiness>,
    ordinal: usize,
    /// The host-owned TODO plan; `None` until first read.
    plan: Option<Plan>,
}

impl Chat {
    fn live(&self) -> Option<&Arc<dyn AgentSession>> {
        let closed = self
            .snapshot
            .as_ref()
            .is_some_and(|s| s.phase == SessionPhase::Closed);
        self.session.as_ref().filter(|_| !closed)
    }

    fn phase(&self) -> SessionPhase {
        match (&self.snapshot, self.live()) {
            (Some(snapshot), Some(_)) => snapshot.phase,
            _ => SessionPhase::Closed,
        }
    }

    /// Tab title (07 C10): the engine's session name, else the first user message, else `聊天 N`.
    fn title(&self) -> String {
        let named = self
            .snapshot
            .as_ref()
            .and_then(|s| s.session_name.clone())
            .filter(|name| !name.trim().is_empty());
        let first = named.or_else(|| self.first_message());
        match first.filter(|t| !t.trim().is_empty()) {
            Some(text) if text.chars().count() > 24 => {
                format!("{}…", text.chars().take(24).collect::<String>())
            }
            Some(text) => text,
            None => format!("{} {}", s::CHAT, self.ordinal),
        }
    }

    fn first_message(&self) -> Option<String> {
        self.snapshot.as_ref().and_then(|s| {
            s.messages
                .iter()
                .find(|m| m.role == thinkrail_core::Role::User)
                .and_then(|m| {
                    m.content.iter().find_map(|b| match b {
                        thinkrail_core::ContentBlock::Text { text } => {
                            Some(text.lines().next().unwrap_or_default().to_owned())
                        }
                        thinkrail_core::ContentBlock::Thinking { .. }
                        | thinkrail_core::ContentBlock::Image { .. }
                        | thinkrail_core::ContentBlock::Tool { .. } => None,
                    })
                })
        })
    }
}

/// What a background start/resume produced.
struct Started {
    session: Arc<dyn AgentSession>,
    binding: ChatBinding,
    routing_signature: String,
    /// A prompt sent right after starting that the engine refused.
    prompt_error: Option<Error>,
}
struct StartInput {
    services: Services,
    chat: ChatId,
    workspace: WorkspaceId,
    project: ProjectId,
    cwd: std::path::PathBuf,
    binding: Option<ChatBinding>,
    sink: Arc<dyn SessionSink>,
    prompt: Option<String>,
}

pub struct ChatPanel {
    state: Entity<AppState>,
    sink: Arc<dyn SessionSink>,
    chats: HashMap<ChatId, Chat>,
    /// Tabs of the current workspace, in display order.
    order: Vec<ChatId>,
    active: Option<ChatId>,
    workspace: Option<WorkspaceId>,
    composer: Entity<TextareaState>,
    open: HashSet<String>,
    answers: HashMap<String, Entity<InputState>>,
    picks: HashMap<String, Vec<String>>,
    scroll: ScrollHandle,
    slash: slash::SlashMenu,
    model_picker: model_picker::ModelPicker,
    usage_view: Entity<lumakit_usage_status::ContextStatusView>,
    /// Readiness guidance shown in place of the empty state when a new chat could not start.
    guide: Option<Readiness>,
    _subscriptions: Vec<Subscription>,
}

impl ChatPanel {
    pub fn new(
        state: Entity<AppState>,
        sink: Arc<dyn SessionSink>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(3)
                .submit_on_enter(true)
                .placeholder(s::PLACEHOLDER)
        });
        let subscriptions = vec![
            cx.subscribe_in(&state, window, |this, _, event: &AppEvent, window, cx| {
                if *event == AppEvent::Workspace || *event == AppEvent::Projects {
                    this.sync_workspace(cx);
                    this.start_setup(window, cx);
                } else if *event == AppEvent::Routing {
                    this.on_routing_changed(cx);
                }
            }),
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                    InputEvent::Change => this.on_draft_changed(cx),
                    InputEvent::PressEnter { shift: true, .. }
                    | InputEvent::Focus
                    | InputEvent::Blur => (),
                },
            ),
        ];
        let model_picker = Self::new_model_picker(window, cx);
        let usage_view = cx.new(|cx| lumakit_usage_status::ContextStatusView::new(window, cx));
        let mut panel = Self {
            state,
            sink,
            chats: HashMap::new(),
            order: Vec::new(),
            active: None,
            workspace: None,
            composer,
            open: HashSet::new(),
            answers: HashMap::new(),
            picks: HashMap::new(),
            scroll: ScrollHandle::new(),
            guide: None,
            slash: slash::SlashMenu::default(),
            model_picker,
            usage_view,
            _subscriptions: subscriptions,
        };
        panel.sync_workspace(cx);
        panel
    }

    fn services(&self, cx: &Context<Self>) -> Services {
        self.state.read(cx).services.clone()
    }

    fn report(&self, error: &Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }
}

impl Render for ChatPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let strip = self.tab_strip(cx).into_any_element();
        let body: AnyElement = match (self.workspace.is_some(), self.active.clone()) {
            (false, _) => empty(s::NO_WORKSPACE, &r),
            (true, None) => match self.guide.and_then(|readiness| {
                agent_guide::row("chat-guide", self.state.clone(), readiness, cx)
            }) {
                Some(guide) => div()
                    .size_full()
                    .p(space::S16)
                    .child(guide)
                    .into_any_element(),
                None => empty(s::NO_CHATS, &r),
            },
            (true, Some(id)) => self.chat_body(&id, r, window, cx),
        };
        v_flex()
            .size_full()
            .bg(r.container_workspace_bg)
            .border_r_1()
            .border_color(r.border_muted)
            .child(strip)
            .child(div().flex_1().min_h(px(0.)).child(body))
    }
}

fn empty(message: &'static str, r: &Roles) -> AnyElement {
    div()
        .size_full()
        .p(space::S16)
        .child(label(text::METADATA, message).text_color(r.text_subtle))
        .into_any_element()
}
