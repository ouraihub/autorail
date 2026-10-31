//! Projects panel: open/close projects, list workspaces, create and remove managed worktrees.

use std::path::PathBuf;

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    StatefulInteractiveElement as _, Subscription, Window,
    assets::IconName,
    base::v_flex,
    component::input::{Input, InputEvent, InputState},
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{ProjectId, WorkspaceKind};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, PanelHeader, TreeRow, label},
    metrics::{size, space, text},
};

use crate::{
    state::{AppEvent, AppState},
    strings::projects as s,
};

pub struct ProjectsPanel {
    state: Entity<AppState>,
    path: Entity<InputState>,
    opening: bool,
    collapsed: Vec<ProjectId>,
    _subscriptions: Vec<Subscription>,
}

impl ProjectsPanel {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = cx.new(|cx| InputState::new(window, cx).placeholder(s::PATH_PLACEHOLDER));
        let subscriptions = vec![
            cx.subscribe(&state, |_, _, _: &AppEvent, cx| cx.notify()),
            cx.subscribe_in(&path, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.submit(window, cx);
                }
            }),
        ];
        Self { state, path, opening: false, collapsed: Vec::new(), _subscriptions: subscriptions }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.path.read(cx).value().trim().to_owned();
        if value.is_empty() {
            return;
        }
        self.state.update(cx, |state, cx| state.open_project(PathBuf::from(value), cx));
        self.path.update(cx, |input, cx| input.set_value("", window, cx));
        self.opening = false;
        cx.notify();
    }

    fn toggle(&mut self, project: &ProjectId, cx: &mut Context<Self>) {
        match self.collapsed.iter().position(|p| p == project) {
            Some(index) => {
                self.collapsed.remove(index);
            }
            None => self.collapsed.push(project.clone()),
        }
        cx.notify();
    }
}

impl Render for ProjectsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let header = PanelHeader::new(s::TITLE).action(IconButton::new("project-open", IconName::Plus).tooltip(s::OPEN).on_click(
            cx.listener(|this, _, _, cx| {
                this.opening = !this.opening;
                cx.notify();
            }),
        ));
        let state = self.state.read(cx);
        let selected = state.workspace.clone();
        let mut rows = Vec::new();
        for project in &state.projects {
            let open = !self.collapsed.contains(&project.id);
            let id = project.id.clone();
            let (toggle_id, new_id, close_id) = (id.clone(), id.clone(), id.clone());
            rows.push(
                TreeRow::new(format!("project-{}", project.id), project.name.clone())
                    .icon(if open { IconName::FolderOpen } else { IconName::Folder })
                    .expanded(Some(open))
                    .trailing(IconButton::new(format!("ws-new-{}", project.id), IconName::Plus).tooltip(s::NEW_WORKSPACE).on_click(
                        cx.listener(move |this, _, _, cx| {
                            let project = new_id.clone();
                            this.state.update(cx, |state, cx| state.create_workspace(project, cx));
                        }),
                    ))
                    .trailing(IconButton::new(format!("project-close-{}", project.id), IconName::X).tooltip(s::CLOSE).on_click(
                        cx.listener(move |this, _, _, cx| {
                            let project = close_id.clone();
                            this.state.update(cx, |state, cx| state.close_project(project, cx));
                        }),
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(&toggle_id, cx)))
                    .into_any_element(),
            );
            if !open {
                continue;
            }
            for workspace in state.workspaces_of(&project.id) {
                let is_default = workspace.kind == WorkspaceKind::Default;
                let name = if is_default { s::DEFAULT_WORKSPACE.to_owned() } else { workspace.name.clone() };
                let (select_id, remove_id) = (workspace.id.clone(), workspace.id.clone());
                rows.push(
                    TreeRow::new(format!("workspace-{}", workspace.id), name)
                        .depth(1)
                        .icon(if is_default { IconName::Folder } else { IconName::GitBranch })
                        .detail(workspace.branch.clone())
                        .selected(selected.as_ref() == Some(&workspace.id))
                        .when(!is_default, |row| {
                            row.trailing(
                                IconButton::new(format!("ws-remove-{}", workspace.id), IconName::X)
                                    .tooltip(s::REMOVE_WORKSPACE)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let id = remove_id.clone();
                                        this.state.update(cx, |state, cx| state.remove_workspace(id, cx));
                                    })),
                            )
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = select_id.clone();
                            this.state.update(cx, |state, cx| state.select_workspace(id, cx));
                        }))
                        .into_any_element(),
                );
            }
        }
        let empty = state.projects.is_empty();
        v_flex()
            .size_full()
            .bg(r.container_sidebar_bg)
            .border_r_1()
            .border_color(r.border_muted)
            .child(header)
            .when(self.opening, |panel| {
                panel.child(div().p(size::PANEL_PADDING).child(Input::new(&self.path)))
            })
            .child(
                v_flex()
                    .id("projects-tree")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p(space::S4)
                    .gap(space::S2)
                    .children(rows)
                    .when(empty, |list| {
                        list.child(label(text::METADATA, s::EMPTY).p(size::PANEL_PADDING).text_color(r.text_subtle))
                    }),
            )
    }
}
