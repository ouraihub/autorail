//! Chat tab lifecycle: workspace sync, new and setup chats, engine start/resume and closing.

use gpui_kit::{Context, Window};
use thinkrail_core::{ChatId, WorkspaceId};
use tracing::info;

use super::{
    Chat, ChatPanel, StartInput,
    engine::{dispose_engine, start_engine},
};
use crate::state::run;

/// The bundled skill that turns a new project into a spec graph (original `SETUP_PROMPT`).
const SETUP_SKILL: &str = "setting-up-a-project";
impl ChatPanel {
    /// Shows the current workspace's chats: live ones stay, stored bindings become closed tabs.
    pub(super) fn sync_workspace(&mut self, cx: &mut Context<Self>) {
        let workspace = self.state.read(cx).workspace.clone();
        if workspace == self.workspace && workspace.is_some() {
            return;
        }
        self.workspace = workspace.clone();
        self.order = self
            .chats
            .iter()
            .filter(|(_, c)| Some(&c.workspace) == workspace.as_ref())
            .map(|(id, _)| id.clone())
            .collect();
        self.order
            .sort_by_key(|id| self.chats.get(id).map_or(0, |c| c.ordinal));
        self.active = self.order.first().cloned();
        self.guide = None;
        cx.notify();
        let Some(workspace) = workspace else { return };
        let bindings = self.services(cx).bindings;
        let ws = workspace.clone();
        run(
            cx,
            move || bindings.bindings(&ws),
            move |this, result, cx| {
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
                            this.chats.insert(
                                id.clone(),
                                Chat {
                                    workspace: workspace.clone(),
                                    binding: Some(binding),
                                    session: None,
                                    snapshot: None,
                                    usage: super::usage::Usage::default(),
                                    routing_signature: None,
                                    catalog: Vec::new(),
                                    restart_pending: false,
                                    closing: false,
                                    starting: false,
                                    blocked: None,
                                    ordinal,
                                    plan: None,
                                },
                            );
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
            },
        );
    }

    fn next_ordinal(&self, workspace: &WorkspaceId) -> usize {
        self.chats
            .values()
            .filter(|c| &c.workspace == workspace)
            .map(|c| c.ordinal)
            .max()
            .unwrap_or(0)
            + 1
    }

    pub(super) fn new_chat(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let id = ChatId::generate();
        let ordinal = self.next_ordinal(&workspace);
        self.chats.insert(
            id.clone(),
            Chat {
                workspace,
                binding: None,
                session: None,
                snapshot: None,
                usage: super::usage::Usage::default(),
                routing_signature: None,
                catalog: Vec::new(),
                restart_pending: false,
                closing: false,
                starting: false,
                blocked: None,
                ordinal,
                plan: None,
            },
        );
        self.order.push(id.clone());
        self.active = Some(id.clone());
        self.start(id, None, cx);
    }

    /// 新建项目 + 让 Agent 起草规格: once that project's workspace is shown, a new chat opens with
    /// the setting-up-a-project skill command in the composer for the user to finish (original
    /// `startProjectSetupChat`; the engine expands the command). Spelled for the default Agent,
    /// which the new chat binds to.
    pub(super) fn start_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let ready = state
            .current_workspace()
            .is_some_and(|w| state.setup_project.as_ref() == Some(&w.project_id));
        if !ready || self.workspace != state.workspace {
            return;
        }
        self.state.update(cx, |state, _| state.setup_project = None);
        self.new_chat(cx);
        let prefs = self.services(cx).prefs;
        let handle = window.window_handle();
        run(
            cx,
            move || prefs.prefs(),
            move |this, result, cx| {
                let Some(backend) = result.ok().and_then(|prefs| prefs.default_agent) else {
                    return;
                };
                let draft = thinkrail_agents::skill_command(backend, SETUP_SKILL);
                let composer = this.composer.clone();
                let _ = handle.update(cx, |_, window, cx| {
                    composer.update(cx, |input, cx| {
                        let end = draft.len();
                        input.set_value(draft, window, cx);
                        // The user types the project idea after the command.
                        input.set_selected_range(end..end, cx);
                        input.focus(window, cx);
                    })
                });
            },
        );
    }

    /// Starts (new chat) or resumes (bound chat) the engine, then sends `prompt` if given.
    pub(super) fn start(&mut self, id: ChatId, prompt: Option<String>, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get_mut(&id) else {
            return;
        };
        if chat.starting || chat.closing {
            return;
        }
        let services = self.state.read(cx).services.clone();
        let Some((cwd, project)) = self
            .state
            .read(cx)
            .workspaces
            .iter()
            .find(|w| w.id == chat.workspace)
            .map(|w| (w.worktree_path.clone(), w.project_id.clone()))
        else {
            return;
        };
        chat.starting = true;
        chat.usage.invalidate();
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
                    if let Ok(Ok(started)) = result {
                        dispose_engine(
                            &this.services(cx),
                            id.clone(),
                            Some(started.session),
                            true,
                            cx,
                        );
                    }
                    return;
                };
                chat.starting = false;
                match result {
                    Ok(Ok(started)) => {
                        info!(chat = %id, backend = started.binding.backend.label(), "chat engine started");
                        chat.snapshot = Some(started.session.snapshot());
                        chat.session = Some(started.session);
                        chat.binding = Some(started.binding);
                        chat.blocked = None;
                        chat.routing_signature = Some(started.routing_signature);
                        chat.catalog = Vec::new();
                        chat.restart_pending = false;
                        chat.closing = false;
                        if let Some(error) = started.prompt_error {
                            this.report(&error, cx);
                        }
                        this.refresh_catalog(&id, cx);
                        this.on_routing_changed(cx);
                        this.refresh_usage(&id, cx);
                    }
                    Ok(Err(readiness)) => {
                        if chat.binding.is_some() {
                            chat.blocked = Some(readiness);
                        } else {
                            this.chats.remove(&id);
                            this.order.retain(|c| c != &id);
                            if this.active.as_ref() == Some(&id) {
                                this.active = this.order.last().cloned();
                            }
                            this.guide = Some(readiness);
                        }
                    }
                    Err(error) => {
                        if chat.binding.is_none() && chat.session.is_none() {
                            this.chats.remove(&id);
                            this.order.retain(|c| c != &id);
                            if this.active.as_ref() == Some(&id) {
                                this.active = this.order.last().cloned();
                            }
                        }
                        this.report(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }
    /// Closes a chat tab: disposes its engine and drops its binding, so it does not come back on
    /// the next start. The engine's own transcript stays where the engine keeps it.
    pub(super) fn close_chat(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.remove(id) else {
            return;
        };
        let index = self.order.iter().position(|c| c == id);
        self.order.retain(|c| c != id);
        if self.active.as_ref() == Some(id) {
            self.active = index
                .and_then(|i| self.order.get(i.min(self.order.len().saturating_sub(1))))
                .cloned();
            self.scroll.scroll_to_bottom();
        }
        if let Some(snapshot) = &chat.snapshot {
            for interaction in &snapshot.interactions {
                self.answers
                    .retain(|key, _| !key.starts_with(&interaction.request_id));
                self.picks
                    .retain(|key, _| !key.starts_with(&interaction.request_id));
            }
        }
        cx.notify();
        dispose_engine(&self.services(cx), id.clone(), chat.session, true, cx);
    }
}
