//! 后台日志 (native-multi-agent 06 §5, after AutoNote `LogSettings` / `LogPanel`): the 显示后台日志
//! switch and level, and — while shown and this page is open — the newest log lines, refreshed
//! every 2 s, filterable by `op{id=…`, `chat=…` or module, copyable as raw lines.
//!
//! The reads here bypass `state::run` on purpose: polling must not write an `op` line every 2 s.

use std::time::Duration;

use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::Button,
        input::{Input, InputEvent, InputState},
        switch::Switch,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{AppPrefs, Error, LogLevel};
use thinkrail_observability::LogEntry;
use thinkrail_ui::{
    ThinkrailTheme,
    components::{Segment, label},
    metrics::{radius, space, text},
};
use tracing::warn;

use crate::{
    state::{AppState, run},
    strings::settings_logs as s,
};

const LIMIT: usize = 300;
const EVERY: Duration = Duration::from_secs(2);

pub struct LogsPage {
    state: Entity<AppState>,
    prefs: Option<AppPrefs>,
    entries: Vec<LogEntry>,
    only_problems: bool,
    filter: Entity<InputState>,
    read_error: Option<String>,
    copied: bool,
    /// Set by the settings shell: the page is the one on screen.
    active: bool,
    /// The polling loop; dropping it stops polling.
    poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl LogsPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(s::FILTER));
        let subscriptions = vec![cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify()
            }
        })];
        let mut page = Self {
            state,
            prefs: None,
            entries: Vec::new(),
            only_problems: false,
            filter,
            read_error: None,
            copied: false,
            active: false,
            poll: None,
            _subscriptions: subscriptions,
        };
        page.load_prefs(cx);
        page
    }

    fn report(&mut self, error: &Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }

    fn load_prefs(&mut self, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.app_prefs.clone();
        run(
            cx,
            move || service.app_prefs(),
            |this, result, cx| match result {
                Ok(prefs) => {
                    this.prefs = Some(prefs);
                    this.sync_polling(cx);
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    /// The settings shell calls this when the page is shown or left.
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        self.active = active;
        self.sync_polling(cx);
    }

    /// Polls only while the page is open and 显示后台日志 is on; otherwise drops the loop.
    fn sync_polling(&mut self, cx: &mut Context<Self>) {
        let wanted = self.active && self.prefs.as_ref().is_some_and(|p| p.logs_visible);
        if !wanted {
            self.poll = None;
            return;
        }
        if self.poll.is_some() {
            return;
        }
        let log = self.state.read(cx).services.log.clone();
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let log = log.clone();
                let read = cx
                    .background_executor()
                    .spawn(async move { log.tail(LIMIT) })
                    .await;
                if this.update(cx, |this, cx| this.loaded(read, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(EVERY).await;
            }
        }));
    }

    fn loaded(&mut self, read: std::io::Result<Vec<LogEntry>>, cx: &mut Context<Self>) {
        match read {
            Ok(entries) => {
                self.entries = entries;
                self.read_error = None;
            }
            Err(error) => self.read_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.poll = None;
        self.sync_polling(cx);
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.app_prefs.clone();
        run(
            cx,
            move || service.set_logs_visible(visible),
            |this, result, cx| match result {
                Ok(prefs) => {
                    this.prefs = Some(prefs);
                    this.sync_polling(cx);
                    cx.notify();
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    fn set_level(&mut self, level: LogLevel, cx: &mut Context<Self>) {
        let services = self.state.read(cx).services.clone();
        run(
            cx,
            move || {
                let prefs = services.app_prefs.set_log_level(level)?;
                if let Err(error) = services.log.set_level(level) {
                    warn!(error = %format!("{error:#}"), "log level not switched");
                }
                Ok(prefs)
            },
            |this, result, cx| match result {
                Ok(prefs) => {
                    this.prefs = Some(prefs);
                    cx.notify();
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    /// Newest first, after 只看问题 and the filter text.
    fn visible(&self, cx: &Context<Self>) -> Vec<&LogEntry> {
        let query = self.filter.read(cx).value().trim().to_owned();
        self.entries
            .iter()
            .rev()
            .filter(|e| !self.only_problems || e.level == "ERROR" || e.level == "WARN")
            .filter(|e| query.is_empty() || e.raw.contains(&query))
            .collect()
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let mut lines: Vec<&str> = self
            .visible(cx)
            .into_iter()
            .map(|e| e.raw.as_str())
            .collect();
        lines.reverse();
        cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")));
        self.copied = true;
        cx.notify();
    }

    /// A TraceId tag selects its whole chain; the same tag again clears the filter.
    fn filter_trace(&mut self, trace_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.filter.read(cx).value().trim() == trace_id {
            String::new()
        } else {
            trace_id.to_owned()
        };
        self.filter
            .update(cx, |input, cx| input.set_value(next, window, cx));
        cx.notify();
    }
}

mod row;

use row::row;

impl Render for LogsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let prefs = self.prefs.clone();
        let shown = prefs.as_ref().is_some_and(|p| p.logs_visible);
        let level = prefs.as_ref().map_or(LogLevel::Info, |p| p.log_level);
        let pinned = self.state.read(cx).services.log.pinned();
        let visible = self.visible(cx);
        let empty = if self.entries.is_empty() {
            s::EMPTY
        } else if self.only_problems {
            s::NO_PROBLEMS
        } else {
            s::NO_MATCHES
        };
        let rows: Vec<_> = visible.iter().map(|e| row(e, &r, cx)).collect();
        let count = visible.len();
        let setting = |title: &'static str, hint: &'static str, control: gpui_kit::AnyElement| {
            h_flex()
                .gap(space::S16)
                .py(space::S12)
                .border_b_1()
                .border_color(r.border_muted)
                .child(
                    v_flex()
                        .flex_1()
                        .gap(space::S2)
                        .child(label(text::UI, title))
                        .child(label(text::METADATA, hint).text_color(r.text_subtle)),
                )
                .child(control)
        };
        v_flex()
            .id("settings-logs")
            .gap(space::S12)
            .child(setting(
                s::SHOW,
                s::SHOW_HINT,
                Switch::new("logs-show")
                    .checked(shown)
                    .disabled(prefs.is_none())
                    .accessibility_label(s::SHOW)
                    .on_click(
                        cx.listener(|this, checked: &bool, _, cx| this.set_visible(*checked, cx)),
                    )
                    .into_any_element(),
            ))
            .child(setting(
                s::LEVEL,
                if pinned {
                    s::LEVEL_PINNED
                } else {
                    s::LEVEL_HINT
                },
                Segment::new(
                    "logs-level",
                    [s::INFO, s::DEBUG],
                    usize::from(level == LogLevel::Debug),
                )
                .on_select(cx.listener(|this, index: &usize, _, cx| {
                    this.set_level(
                        if *index == 1 {
                            LogLevel::Debug
                        } else {
                            LogLevel::Info
                        },
                        cx,
                    )
                }))
                .into_any_element(),
            ))
            .when(shown, |d| {
                d.child(
                    v_flex()
                        .gap(space::S8)
                        .p(space::S12)
                        .rounded(radius::LG)
                        .border_1()
                        .border_color(r.border_muted)
                        .child(
                            h_flex()
                                .gap(space::S8)
                                .items_center()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .child(label(text::TITLE_SECTION, s::TITLE))
                                        .child(
                                            label(text::METADATA, s::recent(count))
                                                .text_color(r.text_subtle),
                                        ),
                                )
                                .child(
                                    Switch::new("logs-problems")
                                        .checked(self.only_problems)
                                        .label(s::ONLY_PROBLEMS)
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.only_problems = *checked;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("logs-refresh")
                                        .outline()
                                        .small()
                                        .label(s::REFRESH)
                                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                                )
                                .child(
                                    Button::new("logs-copy")
                                        .outline()
                                        .small()
                                        .label(s::COPY)
                                        .disabled(count == 0)
                                        .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                                )
                                .child(
                                    Button::new("logs-open-dir")
                                        .outline()
                                        .small()
                                        .label(s::OPEN_DIR)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let dir =
                                                this.state.read(cx).services.log.dir().to_owned();
                                            cx.reveal_path(&dir);
                                        })),
                                ),
                        )
                        .child(Input::new(&self.filter).small())
                        .when(self.copied, |d| {
                            d.child(label(text::METADATA, s::COPIED).text_color(r.feedback_success))
                        })
                        .children(self.read_error.clone().map(|e| {
                            label(text::METADATA, format!("{}{e}", s::READ_FAILED))
                                .text_color(r.feedback_error)
                        }))
                        .child(
                            div()
                                .id("logs-rows")
                                .max_h(px(520.))
                                .overflow_y_scroll()
                                .child(v_flex().children(rows))
                                .when(count == 0, |d| {
                                    d.child(
                                        label(text::METADATA, empty)
                                            .text_color(r.text_subtle)
                                            .p(space::S8),
                                    )
                                }),
                        ),
                )
            })
    }
}
