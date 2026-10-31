//! Chat column (04 §8, §9.2): one tab per chat of the current workspace, the live transcript,
//! interaction cards (never auto-answered) and the composer. A chat is pinned to the profile
//! revision it was created with; a closed engine is resumed from its native locator on send.

mod plan;
mod projection;
mod transcript;

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use serde_json::{Value, json};
use thinkrail_core::{
    AgentSession, BackendKind, ChatBinding, ChatId, Error, ErrorKind, Interaction, InteractionAnswer, InteractionKind,
    ModelChoice, Plan, PlanScope, Profile, ProjectId, Result, RouteRequest, SessionEvent, SessionPhase, SessionSink, SessionSnapshot,
    StartSession, ThinkingLevel, WorkspaceId,
};
use thinkrail_ui::{
    Roles, ThinkrailTheme,
    components::{IconButton, Status, StatusDot, Tab, TabStrip, label},
    metrics::{radius, size, space, text},
};
use tracing::{debug, info};

use crate::{
    services::Services,
    state::{AppEvent, AppState, run},
    strings::chat as s,
};

struct Chat {
    workspace: WorkspaceId,
    binding: Option<ChatBinding>,
    profile: Option<Profile>,
    session: Option<Arc<dyn AgentSession>>,
    snapshot: Option<SessionSnapshot>,
    routing_signature: Option<String>,
    catalog: Vec<ModelChoice>,
    restart_pending: bool,
    closing: bool,
    starting: bool,
    /// Why this chat cannot send (profile revision missing or disabled).
    blocked: bool,
    ordinal: usize,
    /// The host-owned TODO plan; `None` until first read.
    plan: Option<Plan>,
}

impl Chat {
    fn live(&self) -> Option<&Arc<dyn AgentSession>> {
        let closed = self.snapshot.as_ref().is_some_and(|s| s.phase == SessionPhase::Closed);
        self.session.as_ref().filter(|_| !closed)
    }

    fn phase(&self) -> SessionPhase {
        match (&self.snapshot, self.live()) {
            (Some(snapshot), Some(_)) => snapshot.phase,
            _ => SessionPhase::Closed,
        }
    }

    fn title(&self) -> String {
        let first = self.snapshot.as_ref().and_then(|s| {
            s.messages.iter().find(|m| m.role == thinkrail_core::Role::User).and_then(|m| {
                m.content.iter().find_map(|b| match b {
                    thinkrail_core::ContentBlock::Text { text } => Some(text.lines().next().unwrap_or_default().to_owned()),
                    _ => None,
                })
            })
        });
        match first.filter(|t| !t.trim().is_empty()) {
            Some(text) if text.chars().count() > 24 => format!("{}…", text.chars().take(24).collect::<String>()),
            Some(text) => text,
            None => format!("{} {}", s::CHAT, self.ordinal),
        }
    }
}
type RoutingCheck = (ChatId, RouteRequest, Option<String>, Arc<dyn AgentSession>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingAction { Unchanged, Pending, RestartNow }

fn routing_action(old_signature: Option<&str>, new_signature: Option<&str>, phase: SessionPhase) -> RoutingAction {
    if new_signature.is_some() && old_signature == new_signature { return RoutingAction::Unchanged; }
    if phase == SessionPhase::Idle { RoutingAction::RestartNow } else { RoutingAction::Pending }
}

/// What a background start/resume produced.
struct Started {
    profile: Profile,
    session: Arc<dyn AgentSession>,
    binding: ChatBinding,
    routing_signature: String,
    /// A prompt sent right after starting that the engine refused.
    prompt_error: Option<Error>,
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
    hint: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl ChatPanel {
    pub fn new(state: Entity<AppState>, sink: Arc<dyn SessionSink>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx).rows(3).submit_on_enter(true).placeholder(s::PLACEHOLDER)
        });
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
                if *event == AppEvent::Workspace || *event == AppEvent::Projects {
                    this.sync_workspace(cx);
                } else if *event == AppEvent::Routing {
                    this.on_routing_changed(cx);
                }
            }),
            cx.subscribe_in(&composer, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = event {
                    this.send(window, cx);
                }
            }),
        ];
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
            hint: None,
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

    /// Shows the current workspace's chats: live ones stay, stored bindings become closed tabs.
    fn sync_workspace(&mut self, cx: &mut Context<Self>) {
        let workspace = self.state.read(cx).workspace.clone();
        if workspace == self.workspace && workspace.is_some() {
            return;
        }
        self.workspace = workspace.clone();
        self.order = self.chats.iter().filter(|(_, c)| Some(&c.workspace) == workspace.as_ref()).map(|(id, _)| id.clone()).collect();
        self.order.sort_by_key(|id| self.chats.get(id).map_or(0, |c| c.ordinal));
        self.active = self.order.first().cloned();
        self.hint = None;
        cx.notify();
        let Some(workspace) = workspace else { return };
        let profiles = self.services(cx).profiles;
        let ws = workspace.clone();
        run(cx, move || profiles.bindings(&ws), move |this, result, cx| {
            if this.workspace.as_ref() != Some(&workspace) {
                return;
            }
            match result {
                Ok(bindings) => {
                    for binding in bindings {
                        if this.chats.contains_key(&binding.chat_id) {
                            continue;
                        }
                        let ordinal = this.next_ordinal(&workspace);
                        let id = binding.chat_id.clone();
                        this.chats.insert(id.clone(), Chat {
                            workspace: workspace.clone(),
                            binding: Some(binding),
                            profile: None,
                            session: None,
                            snapshot: None,
                            routing_signature: None,
                            catalog: Vec::new(),
                            restart_pending: false,
                            closing: false,
                            starting: false,
                            blocked: false,
                            ordinal,
                            plan: None,
                        });
                        this.load_plan(&id, cx);
                        this.order.push(id);
                    }
                    if this.active.is_none() {
                        this.active = this.order.first().cloned();
                    }
                    cx.notify();
                }
                Err(error) => this.report(&error, cx),
            }
        });
    }

    fn next_ordinal(&self, workspace: &WorkspaceId) -> usize {
        self.chats.values().filter(|c| &c.workspace == workspace).map(|c| c.ordinal).max().unwrap_or(0) + 1
    }

    fn new_chat(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else { return };
        let id = ChatId::generate();
        let ordinal = self.next_ordinal(&workspace);
        self.chats.insert(id.clone(), Chat {
            workspace,
            binding: None,
            profile: None,
            session: None,
            snapshot: None,
            routing_signature: None,
            catalog: Vec::new(),
            restart_pending: false,
            closing: false,
            starting: false,
            blocked: false,
            ordinal,
            plan: None,
        });
        self.order.push(id.clone());
        self.active = Some(id.clone());
        self.start(id, None, cx);
    }

    /// Starts (new chat) or resumes (bound chat) the engine, then sends `prompt` if given.
    fn start(&mut self, id: ChatId, prompt: Option<String>, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get_mut(&id) else { return };
        if chat.starting || chat.closing {
            return;
        }
        let services = self.state.read(cx).services.clone();
        let Some((cwd, project)) = self.state.read(cx).workspaces.iter().find(|w| w.id == chat.workspace).map(|w| (w.worktree_path.clone(), w.project_id.clone()))
        else {
            return;
        };
        chat.starting = true;
        cx.notify();
        let input = StartInput {
            services: services.clone(),
            chat: id.clone(),
            workspace: chat.workspace.clone(),
            project,
            cwd,
            binding: chat.binding.clone(),
            sink: self.sink.clone(),
            prompt,
        };
        run(
            cx,
            move || start_engine(input),
            move |this, result, cx| {
                let Some(chat) = this.chats.get_mut(&id) else {
                    if let Ok(started) = result {
                        dispose_engine(&this.services(cx), id.clone(), Some(started.session), true, cx);
                    }
                    return;
                };
                chat.starting = false;
                match result {
                    Ok(started) => {
                        info!(chat = %id, backend = started.profile.input.backend_id.label(), "chat engine started");
                        chat.snapshot = Some(started.session.snapshot());
                        chat.session = Some(started.session);
                        chat.profile = Some(started.profile);
                        chat.binding = Some(started.binding);
                        chat.routing_signature = Some(started.routing_signature);
                        chat.catalog = Vec::new();
                        chat.restart_pending = false;
                        chat.closing = false;
                        if let Some(error) = started.prompt_error { this.report(&error, cx); }
                        this.refresh_catalog(&id, cx);
                        this.on_routing_changed(cx);
                    }
                    Err(error) => {
                        if matches!(error.kind(), Some(ErrorKind::NotFound | ErrorKind::ProfileDisabled)) && chat.binding.is_some() {
                            chat.blocked = true;
                        }
                        if chat.binding.is_none() && chat.session.is_none() {
                            this.chats.remove(&id);
                            this.order.retain(|c| c != &id);
                            if this.active.as_ref() == Some(&id) {
                                this.active = this.order.last().cloned();
                            }
                            if error.kind() == Some(ErrorKind::NotFound) {
                                this.hint = Some(s::NO_PROFILE);
                            }
                        }
                        this.report(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let Some(id) = self.active.clone() else { return };
        let Some(chat) = self.chats.get(&id) else { return };
        if chat.blocked || chat.starting || chat.closing {
            return;
        }
        self.composer.update(cx, |input, cx| input.set_value("", window, cx));
        match chat.live().cloned() {
            Some(session) => run(cx, move || session.prompt(&text), |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            }),
            None => self.start(id, Some(text), cx),
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.active.as_ref().and_then(|id| self.chats.get(id)).and_then(|c| c.live().cloned()) else {
            return;
        };
        run(cx, move || session.stop(), |this, result, cx| {
            if let Err(error) = result {
                this.report(&error, cx);
            }
        });
    }

    /// Closes a chat tab: disposes its engine and drops its binding, so it does not come back on
    /// the next start. The engine's own transcript stays where the engine keeps it.
    fn close_chat(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.remove(id) else { return };
        let index = self.order.iter().position(|c| c == id);
        self.order.retain(|c| c != id);
        if self.active.as_ref() == Some(id) {
            self.active = index.and_then(|i| self.order.get(i.min(self.order.len().saturating_sub(1)))).cloned();
            self.scroll.scroll_to_bottom();
        }
        if let Some(snapshot) = &chat.snapshot {
            for interaction in &snapshot.interactions {
                self.answers.retain(|key, _| !key.starts_with(&interaction.request_id));
                self.picks.retain(|key, _| !key.starts_with(&interaction.request_id));
            }
        }
        cx.notify();
        dispose_engine(&self.services(cx), id.clone(), chat.session, true, cx);
    }

    fn respond(&mut self, request_id: String, answer: InteractionAnswer, cx: &mut Context<Self>) {
        let Some(session) = self.active.as_ref().and_then(|id| self.chats.get(id)).and_then(|c| c.live().cloned()) else {
            return;
        };
        self.answers.retain(|key, _| !key.starts_with(&request_id));
        self.picks.retain(|key, _| !key.starts_with(&request_id));
        run(cx, move || session.respond(&request_id, answer), |this, result, cx| {
            if let Err(error) = result {
                this.report(&error, cx);
            }
        });
    }

    /// A host tool call may have changed this chat's plan.
    pub fn on_plan_changed(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        if self.chats.contains_key(id) {
            self.load_plan(id, cx);
        }
    }

    fn load_plan(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(id) else { return };
        let scope = PlanScope { workspace: chat.workspace.clone(), chat: id.clone() };
        let plans = self.services(cx).plans;
        let id = id.clone();
        run(cx, move || plans.plan(&scope), move |this, result, cx| match result {
            Ok(plan) => {
                if let Some(chat) = this.chats.get_mut(&id) {
                    chat.plan = Some(plan);
                    cx.notify();
                }
            }
            Err(error) => this.report(&error, cx),
        });
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

    /// Rechecks live chats with `new_session: false`, so default-model preferences never alter
    /// the signature of an existing native session.
    fn on_routing_changed(&mut self, cx: &mut Context<Self>) {
        let services = self.services(cx);
        let project_ids: HashMap<WorkspaceId, ProjectId> = self.state.read(cx).workspaces.iter().map(|w| (w.id.clone(), w.project_id.clone())).collect();
        let requests: Vec<RoutingCheck> = self
            .chats
            .iter()
            .filter_map(|(id, chat)| {
                let profile = chat.profile.as_ref()?;
                let project = project_ids.get(&chat.workspace)?.clone();
                let session = chat.live()?.clone();
                Some((
                    id.clone(),
                    RouteRequest { chat: id.clone(), project, backend: profile.input.backend_id, new_session: false },
                    chat.routing_signature.clone(),
                    session,
                ))
            })
            .collect();
        for (id, request, old_signature, session) in requests {
            self.refresh_catalog(&id, cx);
            let router = services.router.clone();
            run(cx, move || router.signature(&request), move |this, result, cx| {
                let services = this.services(cx);
                let Some(chat) = this.chats.get_mut(&id) else { return };
                if !chat.session.as_ref().is_some_and(|current| Arc::ptr_eq(current, &session)) { return; }
                let signature = match result {
                    Ok(signature) => signature,
                    Err(error) => {
                        chat.restart_pending = true;
                        if chat.phase() == SessionPhase::Idle {
                            chat.closing = true;
                            let current = chat.session.take();
                            dispose_engine(&services, id.clone(), current, false, cx);
                        }
                        this.report(&error, cx);
                        return;
                    }
                };
                match routing_action(old_signature.as_deref(), Some(signature.as_str()), chat.phase()) {
                    RoutingAction::Unchanged => (),
                    RoutingAction::Pending => {
                        chat.restart_pending = true;
                        cx.notify();
                    }
                    RoutingAction::RestartNow => {
                        let current_matches = chat.session.as_ref().is_some_and(|current| Arc::ptr_eq(current, &session));
                        if !current_matches { return; }
                        chat.restart_pending = false;
                        chat.closing = true;
                        let current = chat.session.take();
                        dispose_engine(&services, id.clone(), current, false, cx);
                    }
                }
            });
        }
    }

    fn refresh_catalog(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(id) else { return };
        let Some(profile) = chat.profile.as_ref() else { return };
        let router = self.services(cx).router;
        let backend = profile.input.backend_id;
        let id = id.clone();
        run(cx, move || router.catalog(backend), move |this, result, cx| {
            match result {
                Ok(catalog) => {
                    if let Some(chat) = this.chats.get_mut(&id) {
                        chat.catalog = catalog;
                        cx.notify();
                    }
                }
                Err(error) => this.report(&error, cx),
            }
        });
    }

    fn set_model(&mut self, id: ChatId, choice: ModelChoice, effort: Option<ThinkingLevel>, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(&id) else { return };
        let Some(session) = chat.live().cloned() else { return };
        if !chat.snapshot.as_ref().is_some_and(|snapshot| snapshot.capabilities.model_switch)
            || chat.closing || chat.phase() != SessionPhase::Idle { return; }
        let effort = effort.filter(|effort| choice.model.efforts.contains(effort));
        run(cx, move || session.set_model(&choice.route, &choice.model.id, effort), |this, result, cx| {
            if let Err(error) = result { this.report(&error, cx); }
        });
    }

    fn set_effort(&mut self, id: ChatId, effort: ThinkingLevel, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(&id) else { return };
        let Some(session) = chat.live().cloned() else { return };
        let Some(model) = chat.snapshot.as_ref().and_then(|s| s.model.as_ref()) else { return };
        let provider = model.provider.clone().unwrap_or_default();
        let model_id = model.id.clone();
        let Some(choice) = chat.catalog.iter().find(|choice| choice.route == provider && choice.model.id == model_id) else { return };
        if !chat.snapshot.as_ref().is_some_and(|snapshot| snapshot.capabilities.model_switch)
            || chat.closing || chat.phase() != SessionPhase::Idle
            || !choice.model.efforts.contains(&effort) { return; }
        run(cx, move || session.set_model(&provider, &model_id, Some(effort)), |this, result, cx| {
            if let Err(error) = result { this.report(&error, cx); }
        });
    }

    fn header(&self, chat: &Chat, phase: SessionPhase, r: Roles, cx: &mut Context<Self>) -> AnyElement {
        let backend = chat.profile.as_ref().map(|p| p.input.backend_id).or(chat.snapshot.as_ref().map(|s| s.backend));
        let backend_label = backend.map(|b| match b { BackendKind::Dsh => format!("{} · {}", b.label(), s::PRERELEASE), _ => b.label().to_owned() });
        let profile = chat.profile.as_ref().map(|p| format!("{} · {} {}", p.input.label, s::REVISION, p.config_revision));
        let current = chat.snapshot.as_ref().and_then(|s| s.model.as_ref());
        let current_label = current.map_or_else(|| s::NO_MODEL.to_owned(), |m| m.provider.as_ref().map_or_else(|| m.id.clone(), |p| format!("{p}/{}", m.id)));
        let current_choice = current.and_then(|model| chat.catalog.iter().find(|choice| choice.route == model.provider.as_deref().unwrap_or_default() && choice.model.id == model.id));
        let current_effort = current.and_then(|model| model.effort);
        let selectable = chat.snapshot.as_ref().is_some_and(|s| s.capabilities.model_switch) && chat.live().is_some() && !chat.closing && phase == SessionPhase::Idle;
        let Some(id) = chat.binding.as_ref().map(|b| b.chat_id.clone()) else { return empty(s::NO_MODEL, &r) };
        let choices = chat.catalog.clone();
        let efforts = current_choice.map(|choice| choice.model.efforts.clone()).unwrap_or_default();
        let model_entity = cx.entity();
        let effort_entity = cx.entity();
        let model_id = id.clone();
        let effort_id = id;
        let model_menu = Button::new("chat-model").ghost().h(size::HEADER_CONTROL).label(current_label).icon(IconName::ChevronDown).disabled(!selectable).dropdown_menu(move |menu, _, _| {
            choices.iter().cloned().fold(menu, |menu, choice| {
                let id = model_id.clone();
                let effort = choice.model.efforts.iter().copied().find(|effort| Some(*effort) == current_effort).or_else(|| choice.model.efforts.first().copied());
                let model_entity = model_entity.clone();
                menu.item(PopupMenuItem::new(format!("{} · {}", choice.provider_name, choice.model.name)).on_click(move |_, _, cx| {
                    model_entity.update(cx, |this, cx| this.set_model(id.clone(), choice.clone(), effort, cx));
                }))
            })
        });
        let effort_menu = Button::new("chat-effort").ghost().h(size::HEADER_CONTROL).label(current_effort.map(s::effort_label).unwrap_or(s::EFFORT_DEFAULT)).icon(IconName::ChevronDown).disabled(!selectable || efforts.is_empty()).dropdown_menu(move |menu, _, _| {
            efforts.iter().copied().fold(menu, |menu, effort| {
                let id = effort_id.clone();
                let effort_entity = effort_entity.clone();
                menu.item(PopupMenuItem::new(s::effort_label(effort)).on_click(move |_, _, cx| {
                    effort_entity.update(cx, |this, cx| this.set_effort(id.clone(), effort, cx));
                }))
            })
        });
        let (status, phase_label) = match phase { SessionPhase::Idle => (Status::Success, s::PHASE_IDLE), SessionPhase::Running => (Status::Info, s::PHASE_RUNNING), SessionPhase::Waiting => (Status::Warning, s::PHASE_WAITING), SessionPhase::Closed => (Status::Idle, s::PHASE_CLOSED) };
        h_flex().h(size::PANEL_HEADER).flex_none().px(size::PANEL_PADDING).gap(space::S8).border_b_1().border_color(r.border_muted)
            .children(backend_label.map(|b| label(text::LABEL_PILL, b).text_color(r.primary)))
            .children(profile.map(|p| label(text::METADATA, p).text_color(r.text_muted).truncate()))
            .child(div().flex_1().min_w(px(0.)).overflow_hidden().child(model_menu))
            .child(effort_menu).child(StatusDot::new(status).label(phase_label)).into_any_element()
    }
    fn answer_input(&mut self, key: &str, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(input) = self.answers.get(key) {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        self.answers.insert(key.to_owned(), input.clone());
        input
    }
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

fn start_engine(input: StartInput) -> Result<Started> {
    let StartInput { services, chat, workspace, project, cwd, binding, sink, prompt } = input;
    let profile = match &binding {
        Some(b) => services.profiles.revision(&b.profile_id, b.config_revision)?,
        None => {
            let catalog = services.profiles.catalog()?;
            let default = catalog.default_profile_id.as_ref().and_then(|id| catalog.profiles.iter().find(|p| &p.id == id));
            default.cloned().ok_or_else(|| Error::not_found(s::NO_PROFILE))?
        }
    };
    if !profile.input.enabled {
        return Err(Error::new(ErrorKind::ProfileDisabled, &profile.input.label));
    }
    let backend = profile.input.backend_id;
    let route_request = RouteRequest { chat: chat.clone(), project, backend, new_session: binding.is_none() };
    let routing = match services.router.prepare(&route_request) {
        Ok(routing) => routing,
        Err(error) => {
            services.router.release(&chat);
            return Err(error);
        }
    };
    let routing_signature = match services.router.signature(&RouteRequest { new_session: false, ..route_request.clone() }) {
        Ok(signature) => signature,
        Err(error) => {
            services.router.release(&chat);
            return Err(error);
        }
    };
    let support_dir = match services.support_dir(&chat) {
        Ok(path) => path,
        Err(error) => {
            services.router.release(&chat);
            return Err(error);
        }
    };
    let host_tools = services.host_tools(&workspace, &chat);
    let request = StartSession {
        chat: chat.clone(),
        workspace: workspace.clone(),
        profile: profile.clone(),
        cwd,
        resume: binding.as_ref().map(|b| b.native_session_key.clone()),
        host_tools,
        routing,
        support_dir,
    };
    let session = match services.runtime(backend).start(request, sink) {
        Ok(session) => session,
        Err(error) => {
            services.router.release(&chat);
            let _ = services.remove_support_dir(&chat);
            return Err(error);
        }
    };
    let binding = match binding {
        Some(binding) => binding,
        None => {
            let key = match session.snapshot().native_session_key {
                Some(key) => key,
                None => {
                    let _ = session.dispose();
                    services.router.release(&chat);
                    let _ = services.remove_support_dir(&chat);
                    return Err(Error::protocol("Agent 没有返回会话 locator"));
                }
            };
            let binding = ChatBinding {
                chat_id: chat.clone(),
                workspace_id: workspace,
                profile_id: profile.id.clone(),
                config_revision: profile.config_revision,
                native_session_key: key,
            };
            if let Err(error) = services.profiles.bind(binding.clone()) {
                let _ = session.dispose();
                services.router.release(&chat);
                let _ = services.remove_support_dir(&chat);
                return Err(error);
            }
            binding
        }
    };
    let prompt_error = prompt.and_then(|text| session.prompt(&text).err());
    Ok(Started { profile, session, binding, routing_signature, prompt_error })
}

/// Disposes an engine, revokes routing and removes generated files. `unbind` only when closing.
fn dispose_engine(
    services: &Services,
    chat: ChatId,
    session: Option<Arc<dyn AgentSession>>,
    unbind: bool,
    cx: &mut Context<ChatPanel>,
) {
    let profiles = services.profiles.clone();
    let router = services.router.clone();
    let support = services.clone();
    let callback_chat = chat.clone();
    run(cx, move || {
        if let Some(session) = session && let Err(error) = session.dispose() {
            debug!(code = %error, "dispose of a closed chat failed");
        }
        router.release(&chat);
        let cleanup = support.remove_support_dir(&chat);
        if unbind {
            profiles.unbind(&chat)?;
        }
        info!(chat = %chat, "chat engine disposed");
        cleanup
    }, move |this, result, cx| {
        if let Some(chat) = this.chats.get_mut(&callback_chat) {
            chat.closing = false;
        }
        if let Err(error) = result {
            this.report(&error, cx);
        }
        cx.notify();
    });
}
impl Render for ChatPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let strip = self.tab_strip(cx).into_any_element();
        let body: AnyElement = match (self.workspace.is_some(), self.active.clone()) {
            (false, _) => empty(s::NO_WORKSPACE, &r),
            (true, None) => empty(self.hint.unwrap_or(s::NO_CHATS), &r),
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
    div().size_full().p(space::S16).child(label(text::METADATA, message).text_color(r.text_subtle)).into_any_element()
}

impl ChatPanel {
    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut strip = TabStrip::new();
        for id in &self.order {
            let Some(chat) = self.chats.get(id) else { continue };
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

    fn chat_body(&mut self, id: &ChatId, r: Roles, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(chat) = self.chats.get(id) else { return empty(s::NO_CHATS, &r) };
        let phase = chat.phase();
        let header = self.header(chat, phase, r, cx).into_any_element();
        let snapshot = chat.snapshot.clone();
        let (starting, blocked, bound) = (chat.starting || chat.closing, chat.blocked, chat.binding.is_some());
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
            .map(|s| s.messages.iter().map(|m| transcript::message(m, &self.open, &toggle, &r)).collect())
            .unwrap_or_default();
        let truncated = snapshot.as_ref().is_some_and(|s| s.output_truncated);
        let plan_key = format!("plan/{id}");
        let plan_strip =
            chat.plan.as_ref().and_then(|p| plan::strip(&plan_key, p, self.open.contains(&plan_key), &toggle, &r));
        let interactions: Vec<AnyElement> = snapshot
            .as_ref()
            .map(|s| s.interactions.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|i| self.interaction_card(i, window, cx))
            .collect();
        let status_line = if starting {
            Some(s::STARTING)
        } else if blocked {
            Some(s::PROFILE_MISSING)
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
                    .when(truncated, |d| d.child(label(text::METADATA, s::TRUNCATED).text_color(r.text_subtle)))
                    .children(messages)
                    .children(interactions)
                    .when_some(status_line, |d, line| d.child(label(text::METADATA, line).text_color(r.text_subtle))),
            )
            .child(self.composer_box(phase, starting || blocked, cx))
            .into_any_element()
    }

    fn composer_box(&self, phase: SessionPhase, disabled: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let busy = matches!(phase, SessionPhase::Running | SessionPhase::Waiting);
        let action = if busy {
            IconButton::new("chat-stop", IconName::Square).tooltip(s::STOP).on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
        } else {
            IconButton::new("chat-send", IconName::ArrowUp)
                .tooltip(s::SEND)
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
        };
        div().p(size::CHAT_GUTTER).border_t_1().border_color(r.border_muted).child(
            v_flex()
                .gap(space::S6)
                .p(space::S8)
                .rounded(radius::LG)
                .border_1()
                .border_color(r.control_border_default)
                .bg(r.control_bg)
                .child(Textarea::new(&self.composer).appearance(false))
                .child(h_flex().justify_end().child(action)),
        )
    }

    fn interaction_card(&mut self, interaction: Interaction, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let id = interaction.request_id.clone();
        let cancel_id = id.clone();
        let title = match (interaction.kind, interaction.title.is_empty()) {
            (InteractionKind::Permission, true) => s::PERMISSION.to_owned(),
            (_, true) => s::QUESTION.to_owned(),
            (_, false) => interaction.title.clone(),
        };
        let mut actions: Vec<AnyElement> = Vec::new();
        let mut body: Vec<AnyElement> = Vec::new();
        if !interaction.message.is_empty() && interaction.kind == InteractionKind::Confirm {
            body.push(label(text::UI, interaction.message.clone()).into_any_element());
        }
        match interaction.kind {
            InteractionKind::Confirm => {
                for (confirmed, text_label) in [(true, s::YES), (false, s::NO)] {
                    let rid = id.clone();
                    actions.push(
                        Button::new(format!("confirm-{id}-{confirmed}"))
                            .label(text_label)
                            .when(confirmed, |b| b.primary())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.respond(rid.clone(), InteractionAnswer::Confirm { confirmed }, cx)
                            }))
                            .into_any_element(),
                    );
                }
            }
            InteractionKind::Select | InteractionKind::Permission => {
                for option in &interaction.options {
                    let (rid, option_id) = (id.clone(), option.id.clone());
                    actions.push(
                        Button::new(format!("select-{id}-{}", option.id))
                            .label(option.label.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.respond(rid.clone(), InteractionAnswer::Select { option_id: option_id.clone() }, cx)
                            }))
                            .into_any_element(),
                    );
                }
            }
            InteractionKind::Input | InteractionKind::Editor => {
                let multi = interaction.kind == InteractionKind::Editor;
                let input = self.answer_input(&id, s::OTHER, window, cx);
                if multi && !interaction.message.is_empty() && input.read(cx).value().is_empty() {
                    let prefill = interaction.message.clone();
                    input.update(cx, |state, cx| state.set_value(prefill, window, cx));
                }
                body.push(Input::new(&input).into_any_element());
                let rid = id.clone();
                actions.push(
                    Button::new(format!("submit-{id}"))
                        .label(s::SUBMIT)
                        .primary()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let text = input.read(cx).value().to_string();
                            this.respond(rid.clone(), InteractionAnswer::Text { text }, cx)
                        }))
                        .into_any_element(),
                );
            }
            InteractionKind::Ask => {
                let questions = interaction.detail["questions"].as_array().cloned().unwrap_or_default();
                for question in &questions {
                    body.push(self.question(&id, question, window, cx));
                }
                let rid = id.clone();
                actions.push(
                    Button::new(format!("ask-submit-{id}"))
                        .label(s::SUBMIT)
                        .primary()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let answers = this.ask_answers(&rid, &questions, cx);
                            this.respond(rid.clone(), InteractionAnswer::Answers { answers }, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        actions.push(
            Button::new(format!("cancel-{id}"))
                .label(s::CANCEL)
                .ghost()
                .on_click(cx.listener(move |this, _, _, cx| this.respond(cancel_id.clone(), InteractionAnswer::Cancel, cx)))
                .into_any_element(),
        );
        v_flex()
            .gap(space::S8)
            .p(space::S12)
            .rounded(radius::MD)
            .border_1()
            .border_color(r.feedback_warning_muted)
            .bg(r.feedback_warning_subtle)
            .child(
                h_flex()
                    .gap(space::S6)
                    .child(StatusDot::new(Status::Warning))
                    .child(label(text::TITLE_COMPACT, title).text_color(r.text_default)),
            )
            .children(body)
            .child(h_flex().flex_wrap().gap(space::S6).children(actions))
            .into_any_element()
    }

    fn question(&mut self, request: &str, question: &Value, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let qid = question["id"].as_str().unwrap_or_default().to_owned();
        let key = format!("{request}/{qid}");
        let multi = question["multi"] == true;
        let picked = self.picks.get(&key).cloned().unwrap_or_default();
        let options: Vec<AnyElement> = question["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o["label"].as_str().map(str::to_owned))
            .enumerate()
            .map(|(index, option)| {
                let (key, chosen) = (key.clone(), picked.contains(&option));
                let value = option.clone();
                Button::new(format!("pick-{key}-{index}"))
                    .label(option)
                    .when(chosen, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let picks = this.picks.entry(key.clone()).or_default();
                        match picks.iter().position(|p| p == &value) {
                            Some(at) => {
                                picks.remove(at);
                            }
                            None if multi => picks.push(value.clone()),
                            None => *picks = vec![value.clone()],
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let other = self.answer_input(&key, s::OTHER, window, cx);
        let title = question["header"].as_str().map(|h| format!("{h} · ")).unwrap_or_default()
            + question["question"].as_str().unwrap_or_default();
        v_flex()
            .gap(space::S6)
            .child(label(text::UI, title).text_color(r.text_default))
            .child(h_flex().flex_wrap().gap(space::S6).children(options))
            .child(Input::new(&other))
            .into_any_element()
    }

    fn ask_answers(&self, request: &str, questions: &[Value], cx: &Context<Self>) -> Value {
        let answers: Vec<Value> = questions
            .iter()
            .map(|question| {
                let qid = question["id"].as_str().unwrap_or_default();
                let key = format!("{request}/{qid}");
                let mut answer = json!({"id": qid, "selectedOptions": self.picks.get(&key).cloned().unwrap_or_default()});
                if let Some(text) = self.answers.get(&key).map(|i| i.read(cx).value().trim().to_owned()).filter(|t| !t.is_empty()) {
                    answer["customInput"] = json!(text);
                }
                answer
            })
            .collect();
        Value::Array(answers)
    }
}


#[cfg(test)]
mod routing_tests {
    use super::{routing_action, RoutingAction};
    use thinkrail_core::SessionPhase;

    #[test]
    fn unchanged_signature_keeps_running_session() {
        assert_eq!(routing_action(Some("same"), Some("same"), SessionPhase::Running), RoutingAction::Unchanged);
    }

    #[test]
    fn changed_idle_session_restarts_now() {
        assert_eq!(routing_action(Some("old"), Some("new"), SessionPhase::Idle), RoutingAction::RestartNow);
    }

    #[test]
    fn changed_running_session_waits_until_idle() {
        assert_eq!(routing_action(Some("old"), Some("new"), SessionPhase::Running), RoutingAction::Pending);
    }
}
