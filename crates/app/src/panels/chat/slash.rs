//! The composer's `/` menu (native-multi-agent 07 §1): rows anchored above the composer, the
//! ↑↓ / Enter / Tab / Esc keys captured before the textarea sees them while the menu shows, and
//! completion of the draft to `/name `.

use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window,
    base::{h_flex, v_flex},
    component::input::{Enter, Escape, IndentInline, MoveDown, MoveUp},
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::BackendKind;
use thinkrail_ui::{
    ThinkrailTheme,
    components::label,
    metrics::{radius, size, space, text},
};

use super::{
    Chat, ChatPanel,
    commands::{self, HostCommand, Row},
};

#[cfg(test)]
mod tests;

/// Rows visible at once; more scroll (07 C11).
const VISIBLE_ROWS: f32 = 8.;

/// Menu state; its rows are derived from the draft and the active chat whenever they are read.
#[derive(Debug, Default)]
pub(super) struct SlashMenu {
    selected: usize,
    /// Esc hides the menu until the draft changes.
    dismissed: bool,
    /// Reset with the rest of the state on every edit, so the list starts at the top.
    scroll: ScrollHandle,
}

impl ChatPanel {
    /// The chat's engine and the host built-ins it offers: capabilities come from the snapshot,
    /// or from the runtime before the engine has run (07 C8).
    pub(super) fn host_offer(
        &self,
        chat: &Chat,
        cx: &Context<Self>,
    ) -> Option<(BackendKind, Vec<HostCommand>)> {
        let backend = chat
            .binding
            .as_ref()
            .map(|b| b.backend)
            .or(chat.snapshot.as_ref().map(|s| s.backend))?;
        let caps = match &chat.snapshot {
            Some(snapshot) => snapshot.capabilities,
            None => self.state.read(cx).services.runtime(backend).capabilities(),
        };
        Some((backend, commands::host_commands(&caps)))
    }

    /// The rows the menu shows now; empty while it is hidden.
    fn slash_rows(&self, cx: &Context<Self>) -> Vec<Row> {
        if self.slash.dismissed {
            return Vec::new();
        }
        let Some(chat) = self.active.as_ref().and_then(|id| self.chats.get(id)) else {
            return Vec::new();
        };
        if chat.blocked.is_some() {
            return Vec::new();
        }
        let draft = self.composer.read(cx).value();
        let Some(query) = commands::slash_query(&draft) else {
            return Vec::new();
        };
        let Some((backend, host)) = self.host_offer(chat, cx) else {
            return Vec::new();
        };
        let engine = chat.snapshot.as_ref().map_or(&[][..], |s| &s.commands);
        commands::filter(commands::catalog(backend, &host, engine), query)
    }

    /// Any edit shows the menu again with the first row selected.
    pub(super) fn on_draft_changed(&mut self, cx: &mut Context<Self>) {
        self.slash = SlashMenu::default();
        cx.notify();
    }

    /// Moves the selection with wrap-around; `false` when the menu is hidden.
    fn slash_move(&mut self, down: bool, cx: &mut Context<Self>) -> bool {
        let len = self.slash_rows(cx).len();
        if len == 0 {
            return false;
        }
        let current = self.slash.selected.min(len - 1);
        self.slash.selected = if down {
            (current + 1) % len
        } else {
            (current + len - 1) % len
        };
        self.slash.scroll.scroll_to_item(self.slash.selected);
        cx.notify();
        true
    }

    /// Completes the draft to `/name ` (the trailing space closes the menu) with the caret at the
    /// end; `index` defaults to the selected row. `false` when the menu is hidden.
    fn slash_pick(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let rows = self.slash_rows(cx);
        let index = index.unwrap_or(self.slash.selected.min(rows.len().saturating_sub(1)));
        let Some(row) = rows.get(index) else {
            return false;
        };
        let value = format!("/{} ", row.name);
        let end = value.len();
        self.composer.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.set_selected_range(end..end, cx);
            input.focus(window, cx);
        });
        self.slash = SlashMenu::default();
        cx.notify();
        true
    }

    /// Hides the menu until the draft changes; `false` when it is already hidden.
    fn slash_dismiss(&mut self, cx: &mut Context<Self>) -> bool {
        if self.slash_rows(cx).is_empty() {
            return false;
        }
        self.slash.dismissed = true;
        cx.notify();
        true
    }

    /// Wraps the composer textarea: while the menu shows, its keys are handled here in the
    /// capture phase and never reach the textarea (Enter must not send).
    pub(super) fn slash_keys(
        &self,
        textarea: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.slash_move(false, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.slash_move(true, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                // Shift+Enter still inserts a newline, which closes the menu.
                if !action.shift && !action.secondary && this.slash_pick(None, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if this.slash_pick(None, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                if this.slash_dismiss(cx) {
                    cx.stop_propagation();
                }
            }))
            .child(textarea)
    }

    /// The menu, anchored with its bottom at the top of the (relative) composer container so it
    /// overlays the transcript; `None` while hidden.
    pub(super) fn slash_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rows = self.slash_rows(cx);
        if rows.is_empty() {
            return None;
        }
        let r = ThinkrailTheme::global(cx).roles;
        let selected = self.slash.selected.min(rows.len() - 1);
        let row_height = f32::from(text::CODE_TEXT.size) * text::CODE_TEXT.line_height
            + 2. * f32::from(space::S4);
        let list_height = VISIBLE_ROWS * row_height + (VISIBLE_ROWS - 1.) * f32::from(space::S2);
        let items = rows.into_iter().enumerate().map(|(index, row)| {
            h_flex()
                .id(format!("slash-row-{index}"))
                .test_support()
                .w_full()
                .gap(space::S8)
                .px(space::S8)
                .py(space::S4)
                .rounded(radius::SM)
                .cursor_pointer()
                .when(index == selected, |d| d.bg(r.control_bg_selected))
                .when(index != selected, |d| {
                    d.hover(|s| s.bg(r.control_bg_hovered))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.slash_pick(Some(index), window, cx);
                }))
                .child(
                    label(text::CODE_TEXT, format!("/{}", row.name))
                        .flex_none()
                        .text_color(r.text_default),
                )
                .children(row.description.map(|description| {
                    label(text::METADATA, description)
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_color(r.text_muted)
                }))
                .child(
                    label(text::METADATA, row.source)
                        .flex_none()
                        .ml_auto()
                        .text_color(r.text_subtle),
                )
        });
        Some(
            v_flex()
                .id("slash-menu")
                .absolute()
                .bottom_full()
                .left(size::CHAT_GUTTER)
                .right(size::CHAT_GUTTER)
                .mb(space::S4)
                .p(space::S4)
                .rounded(radius::MD)
                .border_1()
                .border_color(r.border_default)
                .bg(r.container_elevated_bg)
                .occlude()
                .child(
                    v_flex()
                        .id("slash-menu-rows")
                        .max_h(px(list_height))
                        .overflow_y_scroll()
                        .track_scroll(&self.slash.scroll)
                        .gap(space::S2)
                        .children(items),
                )
                .into_any_element(),
        )
    }
}
