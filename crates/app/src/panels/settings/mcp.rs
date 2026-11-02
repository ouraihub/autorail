//! MCP server settings: global entries and project-effective overrides.

use gpui_kit::{
    AppContext as _, Context, Entity, Subscription, Window,
    component::input::{InputEvent, InputState, TextareaState},
};
use thinkrail_core::{
    EffectiveMcpServer, McpOrigin, McpScope, McpServer, McpServerId, McpServerInput, ProjectId,
};

use crate::{
    state::{AppEvent, AppState, run},
    strings::settings_mcp as s,
};

mod editor;
mod form;
mod import;
#[cfg(test)]
mod tests;
mod view;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewScope {
    Global,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportKind {
    Stdio,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportPreview {
    valid: Vec<McpServerInput>,
    invalid: usize,
}

struct PairFields {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

struct Editor {
    id: Option<McpServerId>,
    revision: u64,
    name: Entity<InputState>,
    command: Entity<InputState>,
    url: Entity<InputState>,
    args: Entity<TextareaState>,
    env: Vec<PairFields>,
    headers: Vec<PairFields>,
    description: Entity<InputState>,
    transport: TransportKind,
    enabled: bool,
    scope: McpScope,
    saving: bool,
}

pub struct McpPage {
    state: Entity<AppState>,
    scope: ViewScope,
    global: Vec<McpServer>,
    effective: Vec<EffectiveMcpServer>,
    editor: Option<Editor>,
    import: Entity<TextareaState>,
    import_preview: Option<ImportPreview>,
    loading: bool,
    load_generation: u64,
    importing: bool,
    confirm_delete: Option<(McpServerId, u64)>,
    error: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl McpPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let import = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(5)
                .placeholder(s::IMPORT_JSON)
        });
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
                if matches!(event, AppEvent::Projects | AppEvent::Workspace) {
                    this.editor = None;
                    this.import_preview = None;
                    if this.project(cx).is_none() {
                        this.scope = ViewScope::Global;
                    }
                    this.reload(cx);
                }
            }),
            cx.subscribe(&import, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.import_preview = None;
                    cx.notify();
                }
            }),
        ];
        let mut page = Self {
            state,
            scope: ViewScope::Global,
            global: Vec::new(),
            effective: Vec::new(),
            editor: None,
            import,
            import_preview: None,
            loading: false,
            load_generation: 0,
            importing: false,
            confirm_delete: None,
            error: None,
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        page
    }

    fn project(&self, cx: &Context<Self>) -> Option<ProjectId> {
        self.state.read(cx).project.clone()
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_generation += 1;
        let generation = self.load_generation;
        let service = self.state.read(cx).services.mcp.clone();
        let project = self.project(cx);
        let requested_scope = self.scope;
        self.loading = true;
        match requested_scope {
            ViewScope::Global => run(
                cx,
                move || service.global(),
                move |this, result, cx| {
                    if this.load_generation != generation {
                        return;
                    }
                    this.loading = false;
                    match result {
                        Ok(value) => {
                            this.global = value;
                            this.error = None;
                            cx.notify();
                        }
                        Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                    }
                },
            ),
            ViewScope::Project => {
                let Some(project) = project else {
                    self.loading = false;
                    self.effective.clear();
                    return;
                };
                let expected = project.clone();
                run(
                    cx,
                    move || service.effective(&project),
                    move |this, result, cx| {
                        if this.load_generation != generation
                            || this.scope != requested_scope
                            || this.project(cx).as_ref() != Some(&expected)
                        {
                            return;
                        }
                        this.loading = false;
                        match result {
                            Ok(value) => {
                                this.effective = value;
                                this.error = None;
                                cx.notify();
                            }
                            Err(error) => {
                                this.state.update(cx, |state, cx| state.report(&error, cx))
                            }
                        }
                    },
                );
            }
        }
    }

    fn toggle(&mut self, server: McpServer, enabled: bool, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.mcp.clone();
        let project = self.project(cx);
        let origin_is_global = self.scope == ViewScope::Project
            && self
                .effective
                .iter()
                .any(|row| row.server.id == server.id && row.origin == McpOrigin::Global);
        run(
            cx,
            move || {
                if origin_is_global {
                    let project = project
                        .ok_or_else(|| thinkrail_core::Error::not_found(s::PROJECT_MISSING))?;
                    service.set_override(&project, &server.id, Some(enabled))
                } else {
                    let mut input = server.input.clone();
                    input.enabled = enabled;
                    service.update(&server.id, server.revision, input).map(drop)
                }
            },
            |this, result, cx| match result {
                Ok(()) => {
                    this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    this.reload(cx);
                }
                Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
            },
        );
    }

    fn delete(&mut self, server: McpServer, cx: &mut Context<Self>) {
        if self.confirm_delete.as_ref() != Some(&(server.id.clone(), server.revision)) {
            self.confirm_delete = Some((server.id.clone(), server.revision));
            cx.notify();
            return;
        }
        self.confirm_delete = None;
        let service = self.state.read(cx).services.mcp.clone();
        run(
            cx,
            move || service.delete(&server.id, server.revision),
            |this, result, cx| match result {
                Ok(()) => {
                    this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    this.reload(cx);
                }
                Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
            },
        );
    }
}
