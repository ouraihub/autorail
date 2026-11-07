//! The original's settings building blocks (`SettingsGroup`, `SettingsRow`, `SettingsNotice` and
//! the disclosure row) in the native settings pages' look, shared by every page.

use gpui_kit::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_ui::{
    Roles,
    components::label,
    metrics::{radius, size, space, text},
};

/// A titled group: an eyebrow title, an optional description, then its rows.
pub(super) fn group(
    id: &'static str,
    title: &'static str,
    description: Option<&'static str>,
    r: &Roles,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    v_flex()
        .id(id)
        .child(label(text::EYEBROW, title).text_color(r.text_subtle))
        .children(description.map(|d| {
            label(text::METADATA, d)
                .text_color(r.text_muted)
                .pt(space::S4)
        }))
}

/// A subheading inside a group (the original's `SettingsSubheading`).
pub(super) fn subheading(title: &'static str, description: &'static str, r: &Roles) -> AnyElement {
    v_flex()
        .pt(space::S16)
        .gap(space::S2)
        .child(label(text::TITLE_SECTION, title))
        .child(label(text::METADATA, description).text_color(r.text_subtle))
        .into_any_element()
}

/// One setting: label (and description) left, control right, a rule below.
pub(super) fn row(
    title: impl Into<gpui_kit::SharedString>,
    description: Option<String>,
    control: impl IntoElement,
    r: &Roles,
) -> AnyElement {
    h_flex()
        .gap(space::S16)
        .py(space::S12)
        .border_b_1()
        .border_color(r.border_muted)
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.))
                .gap(space::S2)
                .child(label(text::UI, title.into()))
                .children(description.map(|d| label(text::METADATA, d).text_color(r.text_subtle))),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

/// A warning or info line inside a group.
pub(super) fn notice(text_content: String, warning: bool, r: &Roles) -> AnyElement {
    h_flex()
        .gap(space::S8)
        .p(space::S8)
        .my(space::S8)
        .rounded(radius::SM)
        .bg(if warning {
            r.feedback_warning_subtle
        } else {
            r.primary_subtle
        })
        .child(
            Icon::new(if warning {
                IconName::TriangleAlert
            } else {
                IconName::Info
            })
            .size(size::ICON_SMALL)
            .text_color(if warning {
                r.feedback_warning
            } else {
                r.primary
            }),
        )
        .child(
            label(text::METADATA, text_content)
                .flex_1()
                .text_color(r.text_default),
        )
        .into_any_element()
}

/// A control with a reset button beside it while it differs from the default.
pub(super) fn with_reset(
    id: &'static str,
    dirty: bool,
    reset_label: &'static str,
    control: impl IntoElement,
    on_reset: impl Fn(&mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    h_flex()
        .gap(space::S4)
        .items_center()
        .when_some(dirty.then_some(()), |d, ()| {
            d.child(
                Button::new(id)
                    .ghost()
                    .small()
                    .icon(IconName::RotateCcw)
                    .tooltip(reset_label)
                    .accessibility_label(reset_label)
                    .on_click(move |_, window, cx| on_reset(window, cx)),
            )
        })
        .child(control)
        .into_any_element()
}

/// A disclosure row: label, summary and chevron; the body shows below when open.
pub(super) fn disclosure(
    id: &'static str,
    title: &'static str,
    summary: String,
    open: bool,
    r: &Roles,
    on_toggle: impl Fn(&mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    h_flex()
        .id(id)
        .gap(space::S16)
        .py(space::S12)
        .border_b_1()
        .border_color(r.border_muted)
        .cursor_pointer()
        .hover(|d| d.bg(r.control_bg_hovered))
        .on_click(move |_, window, cx| on_toggle(window, cx))
        .child(label(text::UI, title).flex_1())
        .child(label(text::UI, summary).text_color(r.text_muted))
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(size::ICON_SMALL)
            .text_color(r.text_muted),
        )
        .into_any_element()
}
