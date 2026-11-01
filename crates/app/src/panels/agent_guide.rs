//! The default-Agent guidance of agent-settings 07 §5.1: one line saying what is missing and a
//! button that opens 设置 → 编码搭档 at the relevant engine. Shared by 新建项目 and the chat column.

use gpui_kit::{
    Entity, IntoElement, ParentElement as _, Styled as _,
    base::{h_flex, v_flex},
    component::{Sizable as _, button::Button, WindowExt as _},
    div,
};
use thinkrail_core::{BackendKind, Readiness};
use thinkrail_ui::{ThinkrailTheme, components::label, metrics::{space, text}};

use super::settings::SettingsDialog;
use crate::{state::AppState, strings::{agent_guide as s, settings_coding}};

/// What 07 §5.1 says for a not-ready state, and the engine its button selects. `None` when ready.
pub fn message(readiness: Readiness) -> Option<(String, &'static str, BackendKind)> {
    match readiness {
        Readiness::Ready(_) => None,
        Readiness::NoneInstalled => Some((s::NONE_INSTALLED.to_owned(), s::INSTALL, BackendKind::Pi)),
        Readiness::NoDefault(first) => Some((s::no_default(settings_coding::name(first)), s::SET_DEFAULT, first)),
        Readiness::DefaultMissing(backend) => Some((s::missing(settings_coding::name(backend)), s::OPEN_CODING, backend)),
    }
}

/// The guidance row; the button closes the current dialog (if any) and opens 编码搭档.
pub fn row(id: &'static str, state: Entity<AppState>, readiness: Readiness, cx: &gpui_kit::App) -> Option<impl IntoElement + use<>> {
    let (text_line, action, backend) = message(readiness)?;
    let r = ThinkrailTheme::global(cx).roles;
    Some(
        h_flex().gap(space::S12).items_center()
            .child(v_flex().flex_1().min_w_0().child(label(text::METADATA, text_line).text_color(r.text_muted)))
            .child(div().flex_none().child(Button::new(id).outline().small().label(action).on_click(move |_, window, cx| {
                // Open on the next frame: in the same click a just-closed dialog swallows the new one.
                window.close_dialog(cx);
                let state = state.clone();
                window.defer(cx, move |window, cx| SettingsDialog::open_coding(state, backend, window, cx));
            }))),
    )
}
