//! Bottom terminal panel (04 §5, §9.2): workspace-scoped tabs over real PTYs. The PTY lives in
//! `TerminalService`; this view keeps a vt100 grid, paints it with the theme's ANSI palette and
//! sends keys / IME text. Replay and live output are joined by byte offset, never duplicated.
//! Not supported: mouse reporting, drag selection, OSC clipboard, images.

mod input;
mod input_handler;
mod screen;
mod session;
#[cfg(test)]
mod tests;

use std::{num::NonZeroU16, ops::Range};

use gpui_kit::{
    App, Bounds, Context, ElementInputHandler, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels, Render,
    ScrollEvent, Styled as _, Subscription, TextRun, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    canvas, div, font,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{GridSize, Shell, TerminalRuntimeId, TerminalTab, WorkspaceId};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, Tab, TabStrip, label},
    fonts,
    metrics::{space, text},
};

use crate::{
    state::{AppEvent, AppState},
    strings::terminal as s,
};

const HISTORY: usize = 10_000;
const DEFAULT_SIZE: GridSize = GridSize {
    cols: 120,
    rows: 30,
};

#[cfg(windows)]
const SHELL: Shell = Shell::PowerShell;
#[cfg(not(windows))]
const SHELL: Shell = Shell::Bash;

fn nonzero(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value.max(1)).unwrap_or(NonZeroU16::MIN)
}

fn parser(size: GridSize) -> vt100::Parser {
    vt100::Parser::new(nonzero(size.rows), nonzero(size.cols), HISTORY)
}

enum Link {
    Detached,
    Attaching,
    Attached {
        runtime: TerminalRuntimeId,
        offset: u64,
    },
    Exited(Option<u32>),
}

pub struct TerminalPanel {
    state: Entity<AppState>,
    focus: FocusHandle,
    workspace: Option<WorkspaceId>,
    tabs: Vec<TerminalTab>,
    selected: Option<String>,
    link: Link,
    /// Bumped on every attach/workspace change; late results for an older attempt are dropped.
    attempt: u64,
    parser: vt100::Parser,
    size: GridSize,
    truncated: bool,
    composition: String,
    composition_selection: Range<usize>,
    bounds: Option<Bounds<Pixels>>,
    cell_width: f32,
    line_height: f32,
    _subscriptions: Vec<Subscription>,
}

impl TerminalPanel {
    pub fn new(state: Entity<AppState>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.subscribe(&state, |this, state, event: &AppEvent, cx| {
            let workspace = state.read(cx).workspace.clone();
            if *event == AppEvent::Workspace || workspace != this.workspace {
                this.set_workspace(workspace, cx);
            }
        })];
        let workspace = state.read(cx).workspace.clone();
        let style = text::CODE_TEXT;
        let mut panel = Self {
            state,
            focus: cx.focus_handle(),
            workspace: None,
            tabs: Vec::new(),
            selected: None,
            link: Link::Detached,
            attempt: 0,
            parser: parser(DEFAULT_SIZE),
            size: DEFAULT_SIZE,
            truncated: false,
            composition: String::new(),
            composition_selection: 0..0,
            bounds: None,
            cell_width: f32::from(style.size) * 0.6,
            line_height: f32::from(style.size) * style.line_height,
            _subscriptions: subscriptions,
        };
        panel.set_workspace(workspace, cx);
        panel
    }

    fn report(&self, error: &thinkrail_core::Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }
}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThinkrailTheme::global(cx);
        let r = theme.roles;
        let mut strip = TabStrip::new();
        for tab in &self.tabs {
            let (select, close) = (tab.tab_key.clone(), tab.tab_key.clone());
            strip = strip.tab(
                Tab::new(format!("term-tab-{}", tab.tab_key), tab.title.clone())
                    .icon(IconName::SquareTerminal)
                    .active(self.selected.as_ref() == Some(&tab.tab_key))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.attach(select.clone(), cx);
                        window.focus(&this.focus, cx);
                    }))
                    .on_close(cx.listener(move |this, _, _, cx| this.close(close.clone(), cx))),
            );
        }
        let can_reconnect =
            self.selected.is_some() && matches!(self.link, Link::Detached | Link::Exited(_));
        let strip = strip
            .action(
                IconButton::new("term-new", IconName::Plus)
                    .tooltip(s::NEW)
                    .disabled(self.workspace.is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.create(cx);
                        window.focus(&this.focus, cx);
                    })),
            )
            .action(
                IconButton::new("term-reconnect", IconName::RefreshCw)
                    .tooltip(s::RECONNECT)
                    .disabled(!can_reconnect)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(key) = this.selected.clone() {
                            this.attach(key, cx);
                            window.focus(&this.focus, cx);
                        }
                    })),
            );
        let layout_entity = cx.entity();
        let paint_entity = cx.entity();
        let screen = canvas(
            move |bounds, window, cx| {
                let run = TextRun {
                    len: 1,
                    font: font(fonts::CODE),
                    color: r.text_default.into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let width = f32::from(
                    window
                        .text_system()
                        .shape_line("M".into(), text::CODE_TEXT.size, &[run], None)
                        .width,
                );
                layout_entity.update(cx, |this, cx| this.layout(bounds, width.max(1.), cx));
            },
            move |bounds, _, window, cx| {
                let focus = paint_entity.read(cx).focus.clone();
                window.handle_input(
                    &focus,
                    ElementInputHandler::new(bounds, paint_entity.clone()),
                    cx,
                );
                paint_entity.update(cx, |this, cx| this.paint_screen(window, cx));
            },
        )
        .size_full();
        let line_height = self.line_height;
        v_flex()
            .size_full()
            .bg(r.container_terminal_bg)
            .text_color(r.text_default)
            .border_t_1()
            .border_color(r.border_muted)
            .child(strip)
            .child(
                div()
                    .id("terminal-screen")
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .px(space::S8)
                    .py(space::S4)
                    .overflow_hidden()
                    .track_focus(&self.focus)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
                    )
                    .on_key_down(cx.listener(Self::key_down))
                    .on_scroll(cx.listener(move |this, event: &ScrollEvent, _, cx| {
                        let delta =
                            f32::from(event.delta.pixel_delta(px(line_height)).y) / line_height;
                        if delta.abs() > 0.05 {
                            this.scroll(
                                if delta > 0. {
                                    delta.ceil() as isize
                                } else {
                                    delta.floor() as isize
                                },
                                cx,
                            );
                        }
                        cx.stop_propagation();
                    }))
                    .child(screen),
            )
            .child(
                h_flex().px(space::S8).py(space::S2).child(
                    label(text::METADATA, self.status_line())
                        .text_color(r.text_subtle)
                        .truncate(),
                ),
            )
            .when(self.workspace.is_none(), |d| {
                d.child(
                    label(text::METADATA, s::NO_WORKSPACE)
                        .p(space::S8)
                        .text_color(r.text_subtle),
                )
            })
    }
}
