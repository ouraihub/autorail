//! Headless-window fixture for view tests: the real services over a throwaway data directory.

use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, Render, TestAppContext, Window,
    WindowBounds, WindowOptions, point, px, size,
};
use thinkrail_observability::LogHandle;

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

/// `AppearanceState::init` writes the process-wide `Typography`, which tests run in parallel
/// would overwrite under each other: one window test at a time holds this.
static WINDOW: Mutex<()> = Mutex::new(());

pub struct Fixture {
    pub window: AnyWindowHandle,
    _channels: Channels,
    _data: tempfile::TempDir,
    _exclusive: MutexGuard<'static, ()>,
}

/// Opens `build`'s view in a 1280×800 window over fresh services.
pub fn open<V: Render>(
    cx: &mut TestAppContext,
    build: impl FnOnce(Entity<AppState>, &mut Window, &mut Context<V>) -> V + 'static,
) -> (Fixture, Entity<V>) {
    open_sized(cx, px(800.), build)
}

/// [`open`] with a window of this height (a long page needs its lower controls on screen).
pub fn open_sized<V: Render>(
    cx: &mut TestAppContext,
    height: gpui_kit::Pixels,
    build: impl FnOnce(Entity<AppState>, &mut Window, &mut Context<V>) -> V + 'static,
) -> (Fixture, Entity<V>) {
    let exclusive = WINDOW.lock().unwrap_or_else(PoisonError::into_inner);
    let data = tempfile::tempdir().unwrap();
    let (services, channels) = services::build(
        data.path(),
        &data.path().join("thinkrail.db"),
        LOG.1.clone(),
    )
    .unwrap();
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        // The app's startup path: stored appearance (defaults in a fresh db) and both folders.
        crate::appearance::AppearanceState::init(services.app_prefs.clone(), data.path(), cx);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1280.), height));
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
        _exclusive: exclusive,
    };
    (fixture, view)
}
