//! One line of the 后台日志 list: time, level, the short TraceId tag (filters its chain), 复制 on
//! the ENTRY line only, operation, component and the message or lifecycle summary.

use gpui_kit::{
    ClipboardItem, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    base::h_flex,
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_observability::LogEntry;
use thinkrail_ui::{
    Roles,
    components::label,
    metrics::{radius, space, text},
};

use super::LogsPage;
use crate::strings::settings_logs as s;

pub(super) fn row(entry: &LogEntry, r: &Roles, cx: &Context<LogsPage>) -> impl IntoElement + use<> {
    let color = match entry.level.as_str() {
        "ERROR" => r.feedback_error,
        "WARN" => r.feedback_warning,
        _ => r.text_default,
    };
    let time = entry.time.get(11..19).unwrap_or("—").to_owned();
    let component = entry.component.clone();
    let operation = entry.operation.clone().unwrap_or_default();
    let is_entry = entry.event.as_deref() == Some("ENTRY");
    h_flex()
        .gap(space::S8)
        .items_start()
        .px(space::S8)
        .py(space::S2)
        .rounded(radius::SM)
        .hover(|d| d.bg(r.control_bg_hovered))
        .child(
            label(text::CODE_SMALL, time)
                .text_color(r.text_subtle)
                .flex_none(),
        )
        .child(
            label(text::CODE_SMALL, entry.level.clone())
                .text_color(color)
                .w(px(40.))
                .flex_none(),
        )
        .when_some(entry.trace_id.clone(), |row, trace_id| {
            let short: String = trace_id.chars().take(8).collect();
            let tag_label = s::filter_trace(&short);
            let filter_id = trace_id.clone();
            row.child(
                Button::new(("log-trace", entry.seq))
                    .link()
                    .xsmall()
                    .label(short)
                    .tooltip(tag_label.clone())
                    .accessibility_label(tag_label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.filter_trace(&filter_id, window, cx)
                    })),
            )
            .when(is_entry, |row| {
                row.child(
                    Button::new(("log-trace-copy", entry.seq))
                        .link()
                        .xsmall()
                        .label(s::COPY_TRACE)
                        .accessibility_label(s::COPY_TRACE)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(trace_id.clone()))
                        }),
                )
            })
        })
        .when(!operation.is_empty(), |row| {
            row.child(
                label(text::CODE_SMALL, operation)
                    .text_color(r.primary)
                    .flex_none(),
            )
        })
        .child(
            label(text::CODE_SMALL, component)
                .text_color(r.text_muted)
                .flex_none(),
        )
        .child(
            label(text::CODE_SMALL, summary(entry))
                .text_color(color)
                .flex_1()
                .min_w_0(),
        )
}

/// Lifecycle lines carry no message; show their event, duration and terminal code instead, so
/// ENTRY and EXIT/ERROR of the same operation are distinguishable in the list.
fn summary(entry: &LogEntry) -> String {
    let Some(event) = &entry.event else {
        return entry.message.clone();
    };
    let mut text = event.clone();
    if let Some(ms) = entry.duration_ms {
        text.push_str(&format!(" {ms}ms"));
    }
    for key in ["|Error:", "|Reason:"] {
        if let Some(value) = entry
            .raw
            .split(key)
            .nth(1)
            .and_then(|v| v.split('|').next())
        {
            text.push(' ');
            text.push_str(value);
        }
    }
    text
}
