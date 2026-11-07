//! 编辑器 group (original `LineWidthSettings.tsx`, kind `file`; 08 A14, A19, A20): the file and
//! diff line width and 适应面板.

use gpui_kit::{
    AnyElement, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, TestSupportExt as _, Window,
    base::v_flex,
    component::{
        Sizable as _,
        input::{Escape, Input, InputEvent, InputState},
        switch::Switch,
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::LineWidth;
use thinkrail_ui::{
    ThinkrailTheme,
    components::label,
    metrics::{space, text},
};

use super::{
    super::rows::{group, row, with_reset},
    AppearancePage,
};
use crate::{appearance::AppearanceState, strings::settings_appearance as s};

/// A draft that is a whole number in range (the original `parsedWidth`).
fn parsed(draft: &str) -> Option<u16> {
    let draft = draft.trim();
    if draft.is_empty() || !draft.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    draft
        .parse()
        .ok()
        .filter(|width| LineWidth::is_valid(*width))
}

fn stored_width(cx: &gpui_kit::App) -> u16 {
    cx.try_global::<AppearanceState>()
        .map_or(LineWidth::DEFAULT, |state| state.prefs.file_line_width)
}

impl AppearancePage {
    pub(super) fn on_line_width_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } | InputEvent::Blur => self.commit_line_width(window, cx),
            InputEvent::Focus => {}
        }
    }

    /// Saves a valid draft that differs from the stored width; an invalid one stays on screen
    /// with its message and is not saved (the original `useSettingsDraft`).
    fn commit_line_width(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(width) = parsed(&self.line_width.read(cx).value()) else {
            return;
        };
        if width != stored_width(cx) {
            AppearanceState::edit(s::LINE_WIDTH_FAILED, cx, |prefs| {
                prefs.file_line_width = width;
            });
        }
        self.show_line_width(stored_width(cx), window, cx);
    }

    /// Puts `width` in the field unless that is already its text.
    fn show_line_width(&mut self, width: u16, window: &mut Window, cx: &mut Context<Self>) {
        self.line_width_shown = width;
        let text = width.to_string();
        if self.line_width.read(cx).value() != text {
            self.line_width
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    /// Follows the stored width (a reset, a failed write) while the field is not being edited.
    pub(super) fn sync_line_width(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stored = stored_width(cx);
        let editing = self.line_width.read(cx).focus_handle(cx).is_focused(window);
        if stored != self.line_width_shown && !editing {
            self.show_line_width(stored, window, cx);
        }
    }

    pub(super) fn editor_group(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let prefs = &cx.global::<AppearanceState>().prefs;
        let (width, bounded) = (prefs.file_line_width, prefs.file_line_width_bounded);
        let invalid = parsed(&self.line_width.read(cx).value()).is_none();
        let input = v_flex()
            .id("line-width-field")
            .test_support()
            .w(px(80.))
            // Esc puts the stored width back instead of closing the settings dialog.
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                let stored = stored_width(cx);
                this.show_line_width(stored, window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                Input::new(&self.line_width)
                    .small()
                    .aria_label(s::LINE_WIDTH_INPUT),
            );
        let switch = Switch::new("line-width-bounded")
            .checked(bounded)
            .accessibility_label(s::FIT_TO_PANE_SWITCH)
            .on_click(cx.listener(|_, checked: &bool, _, cx| {
                let checked = *checked;
                AppearanceState::edit(s::LINE_WIDTH_FAILED, cx, |prefs| {
                    prefs.file_line_width_bounded = checked;
                });
            }));
        group("appearance-editor", s::EDITOR_TITLE, None, &r)
            .child(row(
                s::LINE_WIDTH,
                Some(s::LINE_WIDTH_DESCRIPTION.to_owned()),
                with_reset(
                    "line-width-reset",
                    width != LineWidth::DEFAULT,
                    s::RESET_LINE_WIDTH,
                    input,
                    |_, cx| {
                        AppearanceState::edit(s::LINE_WIDTH_FAILED, cx, |prefs| {
                            prefs.file_line_width = LineWidth::DEFAULT;
                        });
                    },
                ),
                &r,
            ))
            .when(invalid, |d| {
                d.child(
                    label(
                        text::METADATA,
                        s::line_width_invalid(LineWidth::MIN, LineWidth::MAX),
                    )
                    .id("line-width-invalid")
                    .test_support()
                    .text_color(r.feedback_error)
                    .pt(space::S4),
                )
            })
            .child(row(
                s::FIT_TO_PANE,
                Some(s::FIT_TO_PANE_DESCRIPTION.to_owned()),
                with_reset(
                    "line-width-bounded-reset",
                    !bounded,
                    s::RESET_FIT_TO_PANE,
                    switch,
                    |_, cx| {
                        AppearanceState::edit(s::LINE_WIDTH_FAILED, cx, |prefs| {
                            prefs.file_line_width_bounded = true;
                        });
                    },
                ),
                &r,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::parsed;

    #[test]
    fn only_whole_numbers_in_range_parse() {
        assert_eq!(parsed("40"), Some(40));
        assert_eq!(parsed(" 240 "), Some(240));
        for bad in ["", "39", "241", "12.5", "-80", "1e2", "八十", "+80"] {
            assert_eq!(parsed(bad), None, "{bad:?}");
        }
    }
}
