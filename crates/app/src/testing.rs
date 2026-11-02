//! Headless-window fixture for view tests: the real services over a throwaway data directory.

use std::sync::{Arc, LazyLock};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, Render, TestAppContext, Window,
    WindowBounds, WindowOptions, point, px, size,
};
use thinkrail_observability::LogHandle;
use thinkrail_ui::{Appearance, ThinkrailTheme};

use crate::{
    services::{self, Channels},
    state::AppState,
};

/// The logger is process-global, so every test in this binary shares one.
static LOG: LazyLock<(tempfile::TempDir, Arc<LogHandle>)> = LazyLock::new(|| {
    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(thinkrail_observability::init(dir.path()).unwrap());
    (dir, log)
});

pub struct Fixture {
    pub window: AnyWindowHandle,
    _channels: Channels,
    _data: tempfile::TempDir,
}

/// Opens `build`'s view in a 1280×800 window over fresh services.
pub fn open<V: Render>(
    cx: &mut TestAppContext,
    build: impl FnOnce(Entity<AppState>, &mut Window, &mut Context<V>) -> V + 'static,
) -> (Fixture, Entity<V>) {
    let data = tempfile::tempdir().unwrap();
    let (services, channels) = services::build(
        data.path(),
        &data.path().join("thinkrail.db"),
        LOG.1.clone(),
    )
    .unwrap();
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        ThinkrailTheme::install(Appearance::Dark, None, cx).unwrap();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(800.)));
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            move |window, cx| {
                let state = cx.new(|cx| AppState::new(services, cx));
                cx.new(|cx| build(state, window, cx))
            },
        )
        .unwrap()
    });
    let fixture = Fixture {
        window,
        _channels: channels,
        _data: data,
    };
    (fixture, view)
}
