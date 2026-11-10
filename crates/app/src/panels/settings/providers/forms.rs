//! Native dialog lifecycle; shared code owns the form body and action layout.

use std::rc::Rc;

use gpui_kit::{
    App, Context, Entity, IntoElement, ParentElement as _, Pixels, Render, Window,
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
    },
};
use lumakit_gpui::providers as ui;

use super::theme;
use crate::strings::settings_providers as s;

pub(super) fn open_form<V: Render>(
    window: &mut Window,
    cx: &mut App,
    title: &'static str,
    width: Pixels,
    body: Entity<V>,
    ok: &'static str,
    on_ok: impl Fn(&mut V, &mut Window, &mut Context<V>) -> bool + 'static,
) {
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, cx| {
        let (enter_body, enter_ok, body, on_ok) =
            (body.clone(), on_ok.clone(), body.clone(), on_ok.clone());
        let footer = ui::form_footer(
            None,
            Button::new("form-cancel")
                .outline()
                .label(s::CANCEL)
                .on_click(|_, window, cx| window.close_dialog(cx))
                .into_any_element(),
            Some(
                Button::new("form-ok")
                    .primary()
                    .label(ok)
                    .on_click(move |_, window, cx| {
                        if body.update(cx, |view, cx| on_ok(view, window, cx)) {
                            window.close_dialog(cx);
                        }
                    })
                    .into_any_element(),
            ),
        );
        dialog
            .w(width)
            .child(ui::form_body(
                title,
                enter_body.clone().into_any_element(),
                footer.into_any_element(),
                None,
                &theme::roles(cx),
            ))
            .on_ok(move |_, window, cx| {
                enter_body.update(cx, |view, cx| enter_ok(view, window, cx))
            })
    });
}

/// Keeps the editor alive on failure and closes only after the captured write succeeds.
pub(super) fn save<V: Render>(
    page: Entity<super::ProvidersPage>,
    window: &mut Window,
    cx: &mut Context<V>,
    job: impl FnOnce(
        &dyn thinkrail_core::ProviderService,
    ) -> thinkrail_core::Result<thinkrail_core::Provider>
    + Send
    + 'static,
    select: bool,
    failed: impl FnOnce(&mut V, String) + 'static,
) {
    let service = page.read(cx).service(cx);
    let handle = window.window_handle();
    crate::state::run(
        cx,
        move || job(&*service),
        move |view, result, cx| match result {
            Ok(provider) => {
                page.update(cx, |page, cx| {
                    let id = provider.id.clone();
                    page.stored(provider, cx);
                    if select {
                        page.select(id, cx);
                    }
                });
                let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
            }
            Err(error) => {
                failed(view, error.to_string());
                page.update(cx, |page, cx| page.report(&error, cx));
                cx.notify();
            }
        },
    );
}
