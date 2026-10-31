//! The Balanced layout (04 §9.2): top bar, activity bar + Projects, chat column, file/diff +
//! Changes, bottom terminal. Only this view arranges panels; panels never know where they sit.

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, base::{h_flex, v_flex}, component::resizable::{h_resizable, resizable_panel, v_resizable},
    div, prelude::FluentBuilder as _, px,
};
use thinkrail_core::SessionEvent;
use thinkrail_ui::{
    ThinkrailTheme,
    components::{ActivityItem, Breadcrumb, IconButton, Status, StatusDot, TextStyled as _, label},
    metrics::{size, space, text},
};
use gpui_kit::assets::IconName;

use crate::{
    panels::{changes::ChangesPanel, chat::ChatPanel, files::FilePanel, projects::ProjectsPanel, settings::SettingsDialog, terminal::TerminalPanel},
    services::{Channels, Services},
    state::AppState,
    strings::shell as s,
};

pub struct Workbench {
    state: Entity<AppState>,
    projects: Entity<ProjectsPanel>,
    chat: Entity<ChatPanel>,
    files: Entity<FilePanel>,
    changes: Entity<ChangesPanel>,
    terminal: Entity<TerminalPanel>,
    show_projects: bool,
    show_terminal: bool,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    pub fn new(services: Services, channels: Channels, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| AppState::new(services, cx));
        let projects = cx.new(|cx| ProjectsPanel::new(state.clone(), window, cx));
        let chat = cx.new(|cx| ChatPanel::new(state.clone(), channels.session_sink.clone(), window, cx));
        let files = cx.new(|cx| FilePanel::new(state.clone(), window, cx));
        let changes = cx.new(|cx| ChangesPanel::new(state.clone(), files.clone(), window, cx));
        let terminal = cx.new(|cx| TerminalPanel::new(state.clone(), window, cx));
        Self::pump_sessions(&chat, channels.sessions, cx);
        Self::pump_plans(&chat, channels.plans, cx);
        Self::pump_terminals(&terminal, channels.terminals, cx);
        let subscriptions = vec![cx.observe(&state, |_, _, cx| cx.notify())];
        Self { state, projects, chat, files, changes, terminal, show_projects: true, show_terminal: false, _subscriptions: subscriptions }
    }

    /// One foreground task per channel; each event goes to the panel that owns it.
    fn pump_sessions(chat: &Entity<ChatPanel>, events: async_channel::Receiver<SessionEvent>, cx: &mut Context<Self>) {
        let chat = chat.downgrade();
        cx.spawn(async move |_, cx| {
            while let Ok(event) = events.recv().await {
                if chat.update(cx, |chat, cx| chat.on_session_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn pump_plans(chat: &Entity<ChatPanel>, changes: async_channel::Receiver<thinkrail_core::PlanScope>, cx: &mut Context<Self>) {
        let chat = chat.downgrade();
        cx.spawn(async move |_, cx| {
            while let Ok(scope) = changes.recv().await {
                if chat.update(cx, |chat, cx| chat.on_plan_changed(&scope.chat, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn pump_terminals(
        terminal: &Entity<TerminalPanel>,
        events: async_channel::Receiver<thinkrail_core::TerminalEvent>,
        cx: &mut Context<Self>,
    ) {
        let terminal = terminal.downgrade();
        cx.spawn(async move |_, cx| {
            while let Ok(event) = events.recv().await {
                if terminal.update(cx, |terminal, cx| terminal.on_terminal_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let state = self.state.read(cx);
        let project = state.current_project().map_or_else(|| s::NONE.to_owned(), |p| p.name.clone());
        let workspace = state.current_workspace();
        let workspace_name = workspace.map_or_else(|| s::NONE.to_owned(), |w| w.name.clone());
        let branch = workspace.map_or_else(|| s::NONE.to_owned(), |w| w.branch.clone());
        h_flex()
            .h(size::TOP_BAR)
            .flex_none()
            .px(space::S12)
            .gap(space::S16)
            .bg(r.container_header_bg)
            .border_b_1()
            .border_color(r.border_muted)
            .child(label(text::HEADING_SM, s::BRAND).font_family(thinkrail_ui::fonts::BRAND).text_color(r.primary))
            .child(Breadcrumb::new("crumb-project", s::PROJECT, project).icon(IconName::Folder))
            .child(Breadcrumb::new("crumb-workspace", s::WORKSPACE, workspace_name).icon(IconName::FolderGit2))
            .child(Breadcrumb::new("crumb-branch", s::BRANCH, branch).icon(IconName::GitBranch).mono(true))
            .child(div().flex_1())
            .child(StatusDot::new(Status::Success).label(s::READY))
            .child(IconButton::new("open-settings", IconName::Settings).tooltip(s::SETTINGS).on_click(cx.listener(
                |this, _, window, cx| {
                    let state = this.state.clone();
                    SettingsDialog::open(state, window, cx);
                },
            )))
    }

    fn activity_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        v_flex()
            .w(size::ACTIVITY_BAR)
            .flex_none()
            .h_full()
            .bg(r.container_sidebar_bg)
            .border_r_1()
            .border_color(r.border_muted)
            .child(ActivityItem::new("activity-projects", IconName::Folder, s::ACTIVITY_PROJECTS).active(self.show_projects).on_click(
                cx.listener(|this, _, _, cx| {
                    this.show_projects = !this.show_projects;
                    cx.notify();
                }),
            ))
            .child(ActivityItem::new("activity-terminal", IconName::SquareTerminal, s::ACTIVITY_TERMINAL).active(self.show_terminal).on_click(
                cx.listener(|this, _, _, cx| {
                    this.show_terminal = !this.show_terminal;
                    cx.notify();
                }),
            ))
    }

    fn notice(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let notice = self.state.read(cx).notice.clone()?;
        let r = ThinkrailTheme::global(cx).roles;
        Some(
            h_flex()
                .flex_none()
                .px(space::S12)
                .py(space::S6)
                .gap(space::S8)
                .bg(r.feedback_error_subtle)
                .border_b_1()
                .border_color(r.feedback_error_muted)
                .child(label(text::UI, notice.message).flex_1().text_color(r.text_default))
                .child(label(text::METADATA, format!("{}：{}", s::ERROR_CODE, notice.code)).text_color(r.text_muted))
                .child(IconButton::new("dismiss-notice", IconName::X).tooltip(s::DISMISS).on_click(cx.listener(
                    |this, _, _, cx| this.state.update(cx, |state, cx| state.dismiss_notice(cx)),
                ))),
        )
    }
}

impl Render for Workbench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let center = h_resizable("center")
            .child(resizable_panel().size(px(460.)).child(self.chat.clone()))
            .child(
                resizable_panel().child(
                    v_resizable("right")
                        .child(resizable_panel().child(self.files.clone()))
                        .child(resizable_panel().size(px(260.)).child(self.changes.clone())),
                ),
            );
        let main = v_resizable("main")
            .child(resizable_panel().child(center))
            .when(self.show_terminal, |group| group.child(resizable_panel().size(px(240.)).child(self.terminal.clone())));
        v_flex()
            .id("workbench")
            .size_full()
            .bg(r.container_workspace_bg)
            .text_color(r.text_default)
            .text_style(text::UI)
            .child(self.top_bar(cx))
            .children(self.notice(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(self.activity_bar(cx))
                    .when(self.show_projects, |row| {
                        row.child(div().w(px(280.)).flex_none().h_full().child(self.projects.clone()))
                    })
                    .child(div().flex_1().min_w(px(0.)).h_full().child(main)),
            )
    }
}
