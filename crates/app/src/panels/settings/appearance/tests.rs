//! The Appearance page in a headless window: real services, a fresh database and data dir.

use std::sync::Arc;

use gpui_kit::{AppContext as _, ElementId, TestAppContext, px, test::TestWindowExt as _};
use thinkrail_ui::{ThinkrailTheme, Typography};

use super::AppearancePage;
use crate::{appearance::AppearanceState, testing};

fn segment(base: &'static str, index: usize) -> ElementId {
    ElementId::NamedChild(Arc::new(base.into()), index.to_string().into())
}

#[gpui_kit::test]
fn mode_contrast_and_code_size_apply_at_once_and_are_stored(cx: &mut TestAppContext) {
    let (fixture, _page) = testing::open_sized(cx, px(2400.), |_, window, cx| {
        AppearancePage::new(window, cx)
    });
    let click = |cx: &mut TestAppContext, id: ElementId| {
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    cx.update(|cx| assert_eq!(ThinkrailTheme::global(cx).id, "dark"));

    click(cx, segment("theme-mode", 1));
    cx.update(|cx| assert_eq!(ThinkrailTheme::global(cx).id, "light"));

    click(cx, segment("theme-contrast", 1));
    cx.update(|cx| assert_eq!(ThinkrailTheme::global(cx).id, "high-contrast-light"));

    click(cx, ElementId::from("font-size-larger"));
    assert_eq!(Typography::current().code_size, px(14.));

    let (on_screen, stored) = cx.update(|cx| {
        let state = cx.global::<AppearanceState>();
        (
            state.prefs.clone(),
            state.service.app_prefs().unwrap().appearance,
        )
    });
    assert_eq!(on_screen.theme, "high-contrast-light");
    assert_eq!(on_screen.code_font_size, 14);
    assert_eq!(
        stored, on_screen,
        "every change was written to the database"
    );
}

#[gpui_kit::test]
fn escape_closes_the_font_picker_not_the_page(cx: &mut TestAppContext) {
    let (fixture, page) = testing::open_sized(cx, px(2400.), |_, window, cx| {
        AppearancePage::new(window, cx)
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(ElementId::from("font-interface"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(page.read(cx).picker.is_some(), "the picker opened"));

    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert!(page.read(cx).picker.is_none(), "Escape closed the picker"));
}

#[gpui_kit::test]
fn line_width_saves_valid_entries_and_esc_restores(cx: &mut TestAppContext) {
    let (fixture, page) = testing::open_sized(cx, px(2400.), |_, window, cx| {
        AppearancePage::new(window, cx)
    });
    let stored = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let prefs = cx.global::<AppearanceState>().service.app_prefs().unwrap();
            (
                prefs.appearance.file_line_width,
                prefs.appearance.file_line_width_bounded,
            )
        })
    };
    let act = |cx: &mut TestAppContext, f: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let shown = |cx: &mut TestAppContext| {
        cx.update(|cx| page.read(cx).line_width.read(cx).value().to_string())
    };
    let replace = |text: &'static str| {
        move |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            window.click(ElementId::from("line-width-field"), cx);
            window.press("ctrl-a", cx);
            window.input(text, cx);
        }
    };
    assert_eq!(stored(cx), (120, true));

    act(cx, &replace("300"));
    act(cx, &|window, cx| window.press("enter", cx));
    assert_eq!(stored(cx).0, 120, "out of range is not saved");
    act(cx, &|window, _| {
        assert!(
            window
                .try_find(ElementId::from("line-width-invalid"))
                .is_some()
        );
    });

    act(cx, &|window, cx| window.press("escape", cx));
    assert_eq!(shown(cx), "120", "Esc puts the stored width back");
    act(cx, &|window, _| {
        assert!(
            window
                .try_find(ElementId::from("line-width-invalid"))
                .is_none()
        );
    });

    act(cx, &replace("80"));
    act(cx, &|window, cx| window.press("enter", cx));
    assert_eq!(stored(cx).0, 80, "Enter saves a valid width");

    act(cx, &|window, cx| {
        window.click(ElementId::from("line-width-bounded"), cx)
    });
    assert_eq!(stored(cx), (80, false));

    act(cx, &|window, cx| {
        window.click(ElementId::from("line-width-reset"), cx)
    });
    assert_eq!(stored(cx).0, 120);
    assert_eq!(shown(cx), "120", "a reset shows in the field");
}
