//! The live font preview (original `FontsSettings.tsx:99-161`): chat text with bold and inline
//! code, a code block, two buttons, the editor sample 2 px under the code size and the terminal
//! sample at it. Drawn through the active typography, so every change shows at once.

use gpui_kit::{
    AnyElement, IntoElement, ParentElement as _, Styled as _,
    base::{h_flex, v_flex},
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
    },
    div, px,
};
use thinkrail_core::CodeFontSize;
use thinkrail_ui::{
    Roles,
    components::label,
    metrics::{radius, space, text},
};

use crate::strings::settings_appearance as s;

const CODE_BLOCK: [&str; 2] = [
    "const stack = fonts.code ?? defaults.code;",
    "if (size !== 13 && stack.length >= 1) apply(stack); // => -> <=",
];
const EDITOR: [&str; 4] = [
    "export function apply(fonts: FontSettings) {",
    "  const code = fonts.code ?? DEFAULT_CODE;",
    "  return code.length >= 1 && code[0] !== \"\" ? code : null;",
    "}",
];
const TERMINAL: [&str; 2] = ["~/thinkrail main ❯ bun run test", " ✓ 128 passed (1.42 s)"];

fn lines(style: thinkrail_ui::metrics::TextStyle, rows: &[&'static str], r: &Roles) -> AnyElement {
    v_flex()
        .children(rows.iter().map(|line| {
            label(style, *line)
                .whitespace_nowrap()
                .text_color(r.text_default)
        }))
        .into_any_element()
}

pub fn card(code_size: u8, r: &Roles) -> AnyElement {
    let editor_size = code_size.saturating_sub(CodeFontSize::EDITOR_DELTA);
    let editor = v_flex().children(EDITOR.iter().enumerate().map(|(index, line)| {
        h_flex()
            .child(label(text::CODE_SMALL, format!("{}  ", index + 1)).text_color(r.text_subtle))
            .child(
                label(text::CODE_SMALL, *line)
                    .whitespace_nowrap()
                    .text_color(r.text_default),
            )
    }));
    let column = |title: String, body: AnyElement, bg: Option<gpui_kit::Rgba>| {
        v_flex()
            .flex_1()
            .min_w(px(0.))
            .gap(space::S4)
            .child(label(text::EYEBROW, title).text_color(r.text_subtle))
            .child(
                div()
                    .overflow_hidden()
                    .p(space::S8)
                    .rounded(radius::SM)
                    .border_1()
                    .border_color(r.border_muted)
                    .when_some(bg, |d, bg| d.bg(bg))
                    .child(body),
            )
    };
    v_flex()
        .gap(space::S12)
        .p(space::S12)
        .rounded(radius::MD)
        .border_1()
        .border_color(r.border_default)
        .bg(r.container_workspace_bg)
        .child(
            h_flex()
                .justify_between()
                .gap(space::S8)
                .child(label(text::TITLE_COMPACT, s::PREVIEW_TITLE))
                .child(label(text::METADATA, s::PREVIEW_META).text_color(r.text_muted)),
        )
        .child(
            h_flex()
                .flex_wrap()
                .items_baseline()
                .child(label(text::READING, s::PREVIEW_BEFORE_BOLD))
                .child(
                    label(text::TITLE_ENTITY, s::PREVIEW_BOLD)
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD),
                )
                .child(label(text::READING, s::PREVIEW_BEFORE_CODE))
                .child(
                    label(text::CODE_TEXT, s::PREVIEW_CODE)
                        .px(space::S4)
                        .rounded(radius::XS)
                        .bg(r.container_elevated_bg),
                )
                .child(label(text::READING, s::PREVIEW_AFTER_CODE)),
        )
        .child(
            div()
                .overflow_hidden()
                .p(space::S8)
                .rounded(radius::SM)
                .bg(r.container_elevated_bg)
                .child(lines(text::CODE_TEXT, &CODE_BLOCK, r)),
        )
        .child(
            h_flex()
                .gap(space::S8)
                .child(
                    Button::new("preview-open-pr")
                        .primary()
                        .small()
                        .label(s::PREVIEW_OPEN_PR),
                )
                .child(
                    Button::new("preview-review")
                        .outline()
                        .small()
                        .label(s::PREVIEW_REVIEW),
                ),
        )
        .child(
            h_flex()
                .gap(space::S8)
                .items_start()
                .child(column(
                    s::preview_editor(editor_size),
                    editor.into_any_element(),
                    None,
                ))
                .child(column(
                    s::preview_terminal(code_size),
                    lines(text::CODE_TEXT, &TERMINAL, r),
                    Some(r.container_terminal_bg),
                )),
        )
        .into_any_element()
}

use gpui_kit::prelude::FluentBuilder as _;
