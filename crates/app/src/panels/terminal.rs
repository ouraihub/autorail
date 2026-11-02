//! Bottom terminal panel (04 §5, §9.2): workspace-scoped tabs over real PTYs. The PTY lives in
//! `TerminalService`; this view keeps a vt100 grid, paints it with the theme's ANSI palette and
//! sends keys / IME text. Replay and live output are joined by byte offset, never duplicated.
//! Not supported: mouse reporting, drag selection, OSC clipboard, images.

mod input;

use std::{num::NonZeroU16, ops::Range};

use gpui_kit::{
    App, Bounds, ClipboardItem, Context, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Focusable, FontStyle, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Pixels, Point, Render, Rgba, ScrollEvent,
    SharedString, Styled as _, Subscription, TextAlign, TextRun, UTF16Selection, UnderlineStyle,
    Window,
    assets::IconName,
    base::{h_flex, v_flex},
    canvas, div, fill, font, point,
    prelude::FluentBuilder as _,
    px, size,
};
use thinkrail_core::{
    AttachRequest, Attached, GridSize, Shell, TerminalEvent, TerminalRuntimeId, TerminalTab,
    WorkspaceId,
};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, Tab, TabStrip, label},
    fonts,
    metrics::{space, text},
};
use tracing::debug;

use self::input::{encode_key, replace_composition, utf16_byte};
use crate::{
    state::{AppEvent, AppState, run},
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

/// A vt100 color in the theme's palette (16 ANSI + the xterm 6×6×6 cube and gray ramp).
fn color(value: vt100::Color, default: Rgba, ansi: &[Rgba; 16]) -> Rgba {
    let rgb = |r: u8, g: u8, b: u8| Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    };
    match value {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => rgb(r, g, b),
        vt100::Color::Idx(index) if index < 16 => ansi[usize::from(index)],
        vt100::Color::Idx(index) if index < 232 => {
            let n = index - 16;
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        vt100::Color::Idx(index) => {
            let n = 8 + (index - 232) * 10;
            rgb(n, n, n)
        }
    }
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

    fn set_workspace(&mut self, workspace: Option<WorkspaceId>, cx: &mut Context<Self>) {
        self.workspace = workspace.clone();
        self.tabs.clear();
        self.selected = None;
        self.detach();
        cx.notify();
        let Some(workspace) = workspace else { return };
        let service = self.state.read(cx).services.terminals.clone();
        let target = workspace.clone();
        run(
            cx,
            move || service.tabs(&target),
            move |this, result, cx| {
                if this.workspace.as_ref() != Some(&workspace) {
                    return;
                }
                match result {
                    Ok(tabs) => {
                        this.tabs = tabs;
                        if let Some(first) = this.tabs.first().map(|t| t.tab_key.clone()) {
                            this.attach(first, cx);
                        }
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn detach(&mut self) {
        self.attempt += 1;
        self.link = Link::Detached;
        self.composition.clear();
        self.composition_selection = 0..0;
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let key = format!("t-{}", uuid_like());
        let title = format!("{} {}", s::TITLE, self.tabs.len() + 1);
        let service = self.state.read(cx).services.terminals.clone();
        let target = workspace.clone();
        run(
            cx,
            move || service.reserve(&target, &key, &title),
            move |this, result, cx| {
                if this.workspace.as_ref() != Some(&workspace) {
                    return;
                }
                match result {
                    Ok(tab) => {
                        let key = tab.tab_key.clone();
                        this.tabs.push(tab);
                        this.attach(key, cx);
                    }
                    Err(error) => this.report(&error, cx),
                }
            },
        );
    }

    fn attach(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        self.detach();
        self.selected = Some(key.clone());
        self.link = Link::Attaching;
        let attempt = self.attempt;
        let request = AttachRequest {
            workspace,
            tab_key: key,
            shell: SHELL,
            size: self.size,
        };
        let service = self.state.read(cx).services.terminals.clone();
        run(
            cx,
            move || service.attach(request),
            move |this, result: thinkrail_core::Result<Attached>, cx| {
                if this.attempt != attempt {
                    return;
                }
                match result {
                    Ok(attached) => {
                        this.parser = parser(attached.size);
                        this.size = attached.size;
                        this.parser.process(attached.replay.as_bytes());
                        this.truncated = attached.replay_truncated;
                        this.link = Link::Attached {
                            runtime: attached.runtime,
                            offset: attached.replay_end_offset,
                        };
                        if attached.size != this.size {
                            this.resize(cx);
                        }
                    }
                    Err(error) => {
                        this.link = Link::Detached;
                        this.report(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn close(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        if self.selected.as_ref() == Some(&key) {
            self.selected = None;
            self.detach();
        }
        self.tabs.retain(|t| t.tab_key != key);
        cx.notify();
        let service = self.state.read(cx).services.terminals.clone();
        run(
            cx,
            move || service.close(&workspace, &key),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }

    fn runtime(&self) -> Option<&TerminalRuntimeId> {
        match &self.link {
            Link::Attached { runtime, .. } => Some(runtime),
            Link::Detached | Link::Attaching | Link::Exited(_) => None,
        }
    }

    pub fn on_terminal_event(&mut self, event: TerminalEvent, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::Data {
                runtime,
                byte_offset,
                data,
            } => {
                let Link::Attached {
                    runtime: current,
                    offset,
                } = &mut self.link
                else {
                    return;
                };
                if *current != runtime || byte_offset < *offset {
                    return;
                }
                if byte_offset > *offset {
                    debug!(
                        gap = byte_offset - *offset,
                        "terminal output gap; reattaching for replay"
                    );
                    if let Some(key) = self.selected.clone() {
                        self.attach(key, cx);
                    }
                    return;
                }
                self.parser.process(data.as_bytes());
                *offset = byte_offset + data.len() as u64;
                cx.notify();
            }
            TerminalEvent::Exit { runtime, exit_code } => {
                if self.runtime() == Some(&runtime) {
                    self.link = Link::Exited(exit_code);
                    self.composition.clear();
                    cx.notify();
                }
            }
        }
    }

    /// Writes go through the background executor in order (one task per chunk).
    fn write(&mut self, data: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime().cloned() else {
            return;
        };
        if data.is_empty() {
            return;
        }
        self.parser.screen_mut().set_scrollback(0);
        let service = self.state.read(cx).services.terminals.clone();
        let data = data.to_owned();
        run(
            cx,
            move || service.write(&runtime, &data),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }

    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let text = if self.parser.screen().bracketed_paste() {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        self.write(&text, cx);
    }

    fn resize(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime().cloned() else {
            return;
        };
        let service = self.state.read(cx).services.terminals.clone();
        let size = self.size;
        run(
            cx,
            move || service.resize(&runtime, size),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }

    fn layout(&mut self, bounds: Bounds<Pixels>, cell_width: f32, cx: &mut Context<Self>) {
        self.bounds = Some(bounds);
        self.cell_width = cell_width;
        let cols = ((f32::from(bounds.size.width) / cell_width).floor() as u16).clamp(2, 500);
        let rows =
            ((f32::from(bounds.size.height) / self.line_height).floor() as u16).clamp(1, 300);
        if self.size.cols != cols || self.size.rows != rows {
            self.size = GridSize { cols, rows };
            self.parser.set_size(nonzero(rows), nonzero(cols));
            if matches!(self.link, Link::Attached { .. }) {
                self.resize(cx);
            }
            cx.notify();
        }
    }

    fn scroll(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.parser.screen().alternate_screen() {
            return;
        }
        let offset = self
            .parser
            .screen()
            .scrollback()
            .saturating_add_signed(delta);
        self.parser.screen_mut().set_scrollback(offset);
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let m = key.modifiers;
        if event.prefer_character_input || !self.composition.is_empty() {
            return;
        }
        if ((m.control && m.shift) || m.platform) && key.key == "v"
            || (m.shift && key.key == "insert")
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.paste_text(&text, cx);
            }
            cx.stop_propagation();
            return;
        }
        if ((m.control && m.shift) || m.platform) && key.key == "c" {
            cx.write_to_clipboard(ClipboardItem::new_string(self.parser.screen().contents()));
            cx.stop_propagation();
            return;
        }
        if m.shift && matches!(key.key.as_str(), "pageup" | "pagedown") {
            let half = isize::try_from(self.size.rows / 2).unwrap_or(1);
            self.scroll(if key.key == "pageup" { half } else { -half }, cx);
            cx.stop_propagation();
            return;
        }
        if let Some(data) = encode_key(key, self.parser.screen().application_cursor()) {
            self.write(&data, cx);
            cx.stop_propagation();
            window.prevent_default();
        }
    }

    fn cursor_bounds(&self) -> Option<Bounds<Pixels>> {
        let bounds = self.bounds?;
        let (row, col) = self.parser.screen().cursor_position();
        let origin = bounds.origin
            + point(
                px(f32::from(col) * self.cell_width),
                px(f32::from(row) * self.line_height),
            );
        Some(Bounds::new(
            origin,
            size(px(self.cell_width), px(self.line_height)),
        ))
    }

    fn paint_screen(&self, window: &mut Window, cx: &mut App) {
        let Some(bounds) = self.bounds else { return };
        let theme = ThinkrailTheme::global(cx);
        let (ansi, background, foreground, cursor_color) = (
            theme.ansi.colors(),
            theme.roles.container_terminal_bg,
            theme.roles.text_default,
            theme.roles.primary,
        );
        let font_size = text::CODE_TEXT.size;
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        for row in 0..rows.get() {
            for col in 0..cols.get() {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut fg = color(cell.fgcolor(), foreground, &ansi);
                let mut bg = color(cell.bgcolor(), background, &ansi);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let origin = bounds.origin
                    + point(
                        px(f32::from(col) * self.cell_width),
                        px(f32::from(row) * self.line_height),
                    );
                if bg != background {
                    let width = self.cell_width * if cell.is_wide() { 2. } else { 1. };
                    window.paint_quad(fill(
                        Bounds::new(origin, size(px(width), px(self.line_height))),
                        bg,
                    ));
                }
                if !cell.has_contents() {
                    continue;
                }
                let mut cell_font = font(fonts::CODE);
                if cell.bold() {
                    cell_font.weight = FontWeight::BOLD;
                }
                if cell.italic() {
                    cell_font.style = FontStyle::Italic;
                }
                if cell.dim() {
                    fg.a *= 0.6;
                }
                let content = cell.contents();
                let run = TextRun {
                    len: content.len(),
                    font: cell_font,
                    color: fg.into(),
                    background_color: None,
                    underline: cell.underline().then_some(UnderlineStyle {
                        color: Some(fg.into()),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(
                    SharedString::from(content.to_owned()),
                    font_size,
                    &[run],
                    None,
                );
                if let Err(error) = line.paint(
                    origin,
                    px(self.line_height),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                ) {
                    debug!(%error, "terminal cell paint failed");
                }
            }
        }
        if screen.scrollback() == 0
            && !screen.hide_cursor()
            && self.runtime().is_some()
            && let Some(cursor) = self.cursor_bounds()
        {
            let thickness = if self.focus.is_focused(window) {
                2.
            } else {
                1.
            };
            let bar = Bounds::new(
                point(cursor.left(), cursor.bottom() - px(thickness)),
                size(cursor.size.width, px(thickness)),
            );
            window.paint_quad(fill(bar, cursor_color));
        }
        if !self.composition.is_empty()
            && let Some(cursor) = self.cursor_bounds()
        {
            let run = TextRun {
                len: self.composition.len(),
                font: font(fonts::CODE),
                color: foreground.into(),
                background_color: Some(background.into()),
                underline: Some(UnderlineStyle {
                    color: None,
                    thickness: px(1.),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                self.composition.clone().into(),
                font_size,
                &[run],
                None,
            );
            if let Err(error) = line.paint(
                cursor.origin,
                px(self.line_height),
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                debug!(%error, "terminal composition paint failed");
            }
        }
    }

    fn status_line(&self) -> String {
        let state = match &self.link {
            Link::Detached if self.tabs.is_empty() => s::NO_TABS.to_owned(),
            Link::Detached => s::DETACHED.to_owned(),
            Link::Attaching => s::ATTACHING.to_owned(),
            Link::Attached { .. } => format!("{}×{}", self.size.cols, self.size.rows),
            Link::Exited(code) => {
                format!(
                    "{}（{} {}）",
                    s::EXITED,
                    s::EXIT_CODE,
                    code.map_or_else(|| "?".to_owned(), |c| c.to_string())
                )
            }
        };
        let truncated = if self.truncated {
            format!(" · {}", s::TRUNCATED)
        } else {
            String::new()
        };
        format!("{state}{truncated} · {}", s::COPY_HINT)
    }
}

/// A short unique tab key (time + counter); tab keys are opaque to the service.
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("{millis:x}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
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

impl EntityInputHandler for TerminalPanel {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let start = utf16_byte(&self.composition, range.start);
        let end = utf16_byte(&self.composition, range.end).max(start);
        *actual = Some(
            self.composition[..start].encode_utf16().count()
                ..self.composition[..end].encode_utf16().count(),
        );
        Some(self.composition[start..end].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.composition_selection.clone(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.composition.is_empty()).then(|| 0..self.composition.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let text = std::mem::take(&mut self.composition);
        self.composition_selection = 0..0;
        self.write(&text, cx);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = replace_composition(&self.composition, range, text);
        self.composition.clear();
        self.composition_selection = 0..0;
        self.write(&text, cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.runtime().is_none() {
            return;
        }
        let start = range
            .as_ref()
            .map_or(0, |r| r.start.min(self.composition.encode_utf16().count()));
        self.composition = replace_composition(&self.composition, range, text);
        let length = self.composition.encode_utf16().count();
        self.composition_selection = selection.map_or(length..length, |r| {
            (start + r.start).min(length)..(start + r.end).min(length)
        });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds()
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        self.runtime().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vt_grid_keeps_wide_cells_colors_and_alternate_screen() {
        let mut p = vt100::Parser::new(nonzero(2), nonzero(8), 20);
        p.process("\x1b[31m中文\x1b[0m\r\nsecond\r\nthird".as_bytes());
        p.screen_mut().set_scrollback(1);
        let cell = p.screen().cell(0, 0).cloned();
        assert_eq!(
            cell.as_ref().map(vt100::Cell::fgcolor),
            Some(vt100::Color::Idx(1))
        );
        assert!(cell.is_some_and(|c| c.is_wide()));
        p.screen_mut().set_scrollback(0);
        p.process(b"\x1b[?1049hALT");
        assert!(p.screen().alternate_screen());
        p.process(b"\x1b[?1049l");
        assert!(p.screen().contents().contains("third"));
    }

    #[test]
    fn palette_maps_ansi_cube_and_gray() {
        let ansi = [Rgba {
            r: 0.5,
            g: 0.,
            b: 0.,
            a: 1.,
        }; 16];
        let default = Rgba {
            r: 1.,
            g: 1.,
            b: 1.,
            a: 1.,
        };
        assert_eq!(color(vt100::Color::Default, default, &ansi), default);
        assert_eq!(color(vt100::Color::Idx(3), default, &ansi), ansi[3]);
        let cube = color(vt100::Color::Idx(196), default, &ansi);
        assert!((cube.r - 1.).abs() < 1e-6 && cube.g == 0. && cube.b == 0.);
        let gray = color(vt100::Color::Idx(232), default, &ansi);
        assert!((gray.r - 8. / 255.).abs() < 1e-6);
    }
}
