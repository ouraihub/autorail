//! Tab clicks through a headless window.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, ElementId, IntoElement, Render, TestAppContext, Window,
    WindowBounds, WindowOptions, point, px, size,
};

use super::{Tab, TabStrip};
use crate::{Appearance, ThinkrailTheme};

type Clicks = Rc<RefCell<Vec<&'static str>>>;

struct Strip {
    clicks: Clicks,
}

impl Render for Strip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let tab = |key: &'static str| {
            let (select, close) = (self.clicks.clone(), self.clicks.clone());
            Tab::new(key, key)
                .on_click(move |_, _, _| select.borrow_mut().push("select"))
                .on_close(move |_, _, _| close.borrow_mut().push("close"))
        };
        TabStrip::new().tab(tab("tab-a")).tab(tab("tab-b"))
    }
}

fn close_button(tab: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(tab.into()), "close".into())
}

#[gpui_kit::test]
fn clicking_close_closes_without_selecting_the_tab(cx: &mut TestAppContext) {
    let clicks = Clicks::default();
    let window = cx.update(|cx| {
        gpui_kit::init(cx);
        ThinkrailTheme::install(Appearance::Dark, None, cx).unwrap();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(480.), px(120.)));
        let clicks = clicks.clone();
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            move |_, cx| cx.new(|_| Strip { clicks }),
        )
        .unwrap()
        .0
    });

    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click(close_button("tab-a"), cx);
    })
    .unwrap();
    assert_eq!(*clicks.borrow(), ["close"]);

    cx.update_window(window, |_, window, cx| window.click("tab-b", cx))
        .unwrap();
    assert_eq!(*clicks.borrow(), ["close", "select"]);
}
