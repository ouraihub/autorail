//! Renders one chat's messages (plain text; no markdown engine). Collapsible parts toggle through
//! the `toggle` handler the panel supplies, keyed by `"<message>/<block>"`.

use std::{collections::HashSet, rc::Rc};

use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::Icon,
    div,
    prelude::FluentBuilder as _,
};
use thinkrail_core::{ContentBlock, Message, Role, ToolStatus};
use thinkrail_ui::{
    Roles,
    components::label,
    metrics::{radius, size, space, text},
};

use crate::strings::chat as s;

/// Longest tool input/output shown inline; the engine keeps the full text.
const TOOL_PREVIEW: usize = 16 * 1024;

pub type Toggle = Rc<dyn Fn(String, &mut Window, &mut App)>;

pub fn message(m: &Message, open: &HashSet<String>, toggle: &Toggle, r: &Roles) -> AnyElement {
    match m.role {
        Role::User => user(m, r),
        Role::Assistant | Role::Tool => assistant(m, open, toggle, r),
        Role::System => v_flex()
            .gap(space::S4)
            .children(m.content.iter().filter_map(|b| match b {
                ContentBlock::Text { text: t } => Some(label(text::METADATA, t.clone()).text_color(r.text_muted)),
                _ => None,
            }))
            .into_any_element(),
    }
}

fn user(m: &Message, r: &Roles) -> AnyElement {
    let body = m
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::Image { mime_type } => Some(format!("[{} {mime_type}]", s::IMAGE)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    h_flex()
        .justify_end()
        .child(
            div()
                .max_w_4_5()
                .px(space::S12)
                .py(space::S8)
                .rounded(radius::LG)
                .border_1()
                .border_color(r.bubble_user_border)
                .bg(r.bubble_user_bg)
                .child(label(text::READING, body).text_color(r.text_default)),
        )
        .into_any_element()
}

fn assistant(m: &Message, open: &HashSet<String>, toggle: &Toggle, r: &Roles) -> AnyElement {
    v_flex()
        .gap(space::S8)
        .children(m.content.iter().enumerate().filter_map(|(index, block)| {
            let key = format!("{}/{index}", m.id);
            Some(match block {
                ContentBlock::Text { text: t } => label(text::READING, t.clone()).text_color(r.text_default).into_any_element(),
                ContentBlock::Thinking { text: t } if t.trim().is_empty() => return None,
                ContentBlock::Thinking { text: t } => thinking(&key, t, open.contains(&key), toggle, r),
                ContentBlock::Image { mime_type } => {
                    label(text::METADATA, format!("[{} {mime_type}]", s::IMAGE)).text_color(r.text_subtle).into_any_element()
                }
                ContentBlock::Tool { .. } => tool(&key, block, open.contains(&key), toggle, r),
            })
        }))
        .into_any_element()
}

fn disclosure(key: &str, open: bool, icon: IconName, title: String, toggle: &Toggle, r: &Roles) -> impl IntoElement {
    let toggle = toggle.clone();
    let key_owned = key.to_owned();
    h_flex()
        .id(gpui_kit::ElementId::Name(format!("disclose-{key}").into()))
        .gap(space::S6)
        .cursor_pointer()
        .text_color(r.text_muted)
        .child(Icon::new(if open { IconName::ChevronDown } else { IconName::ChevronRight }).size(size::ICON_SMALL))
        .child(Icon::new(icon).size(size::ICON_SMALL))
        .child(label(text::METADATA, title))
        .on_click(move |_, window, cx| toggle(key_owned.clone(), window, cx))
}

fn thinking(key: &str, body: &str, open: bool, toggle: &Toggle, r: &Roles) -> AnyElement {
    let title = format!("{} {} {}", s::THINKING, body.chars().count(), s::CHARS);
    v_flex()
        .gap(space::S4)
        .child(disclosure(key, open, IconName::Brain, title, toggle, r))
        .when(open, |d| {
            d.child(
                label(text::METADATA, body.to_owned())
                    .pl(space::S24)
                    .text_color(r.text_subtle),
            )
        })
        .into_any_element()
}

fn tool(key: &str, block: &ContentBlock, open: bool, toggle: &Toggle, r: &Roles) -> AnyElement {
    let ContentBlock::Tool { name, status, input, output, .. } = block else {
        return div().into_any_element();
    };
    let (icon, color) = match status {
        ToolStatus::Running => (IconName::LoaderCircle, r.feedback_info),
        ToolStatus::Completed => (IconName::CircleCheck, r.feedback_success),
        ToolStatus::Failed => (IconName::CircleX, r.feedback_error),
    };
    let input = input.as_ref().map(|v| bounded(serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())));
    let output = output.as_deref().filter(|o| !o.is_empty()).map(|o| bounded(o.to_owned()));
    v_flex()
        .gap(space::S4)
        .px(space::S8)
        .py(space::S6)
        .rounded(radius::MD)
        .border_1()
        .border_color(r.border_muted)
        .bg(r.container_elevated_bg)
        .child(
            h_flex()
                .gap(space::S6)
                .child(Icon::new(icon).size(size::ICON_SMALL).text_color(color))
                .child(div().flex_1().child(disclosure(key, open, IconName::Wrench, name.to_owned(), toggle, r))),
        )
        .when(open, |d| {
            d.when_some(input, |d, input| d.child(code_section(s::TOOL_INPUT, input, r)))
                .when_some(output, |d, output| d.child(code_section(s::TOOL_OUTPUT, output, r)))
        })
        .into_any_element()
}

fn code_section(title: &'static str, body: String, r: &Roles) -> impl IntoElement {
    v_flex()
        .gap(space::S2)
        .child(label(text::CAPTION, title).text_color(r.text_subtle))
        .child(
            div()
                .p(space::S6)
                .rounded(radius::SM)
                .bg(r.container_content_bg)
                .child(label(text::CODE_SMALL, body).text_color(r.text_default)),
        )
}

fn bounded(mut text: String) -> String {
    if text.len() > TOOL_PREVIEW {
        let mut end = TOOL_PREVIEW;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n…");
    }
    text
}
