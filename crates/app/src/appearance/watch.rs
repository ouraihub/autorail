//! Keeps the appearance current while the app runs (08 A5, A6, A13): the window's system
//! appearance, and a once-a-second signature check of the theme and font folders that rescans
//! only when a file was added, removed or rewritten.

use std::time::Duration;

use gpui_kit::{BorrowAppContext as _, Subscription, Window};

use super::{AppearanceState, files, scan, system_appearance};

const EVERY: Duration = Duration::from_secs(1);

/// Starts watching; the subscription lives as long as the window's root view keeps it.
pub fn start(window: &mut Window, cx: &mut gpui_kit::App) -> Option<Subscription> {
    let (themes_dir, fonts_dir) = {
        let state = cx.try_global::<AppearanceState>()?;
        (state.themes_dir.clone(), state.fonts_dir.clone())
    };
    AppearanceState::set_system(system_appearance(window.appearance()), cx);
    let appearance = window.observe_window_appearance(|window, cx| {
        AppearanceState::set_system(system_appearance(window.appearance()), cx);
    });
    cx.spawn(async move |cx| {
        let signatures = |themes: &std::path::Path, fonts: &std::path::Path| {
            (files::signature(themes), files::signature(fonts))
        };
        let mut last = signatures(&themes_dir, &fonts_dir);
        loop {
            cx.background_executor().timer(EVERY).await;
            let (themes, fonts) = (themes_dir.clone(), fonts_dir.clone());
            let read = cx.background_executor().spawn(async move {
                let now = signatures(&themes, &fonts);
                (now, (now != last).then(|| scan(&themes, &fonts)))
            });
            let (now, found) = read.await;
            last = now;
            if let Some(found) = found {
                cx.update(|cx| {
                    if cx.try_global::<AppearanceState>().is_some() {
                        cx.update_global::<AppearanceState, _>(|state, cx| {
                            state.absorb(found, cx);
                        });
                        AppearanceState::apply(cx);
                    }
                });
            }
        }
    })
    .detach();
    Some(appearance)
}
