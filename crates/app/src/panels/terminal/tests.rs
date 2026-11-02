//! vt100 grid behavior, theme palette mapping and the panel's tab lifecycle.

use std::sync::Arc;

use gpui_kit::{AppContext as _, ElementId, Rgba, TestAppContext, test::TestWindowExt as _};
use thinkrail_core::TerminalTab;

use super::{Link, TerminalPanel, nonzero, screen::color};
use crate::testing;

#[test]
fn vt_grid_keeps_wide_cells_colors_and_alternate_screen() {
    let mut p = vt100::Parser::new(nonzero(2), nonzero(8), 20);
    p.process("\x1b[31m中文\x1b[0m\r\nsecond\r\nthird".as_bytes());
    p.screen_mut().set_scrollback(1);
    let cell = p.screen().cell(0, 0).cloned();
    assert_eq!(
        cell.as_ref().map(vt100::Cell::fgcolor),
        Some(vt100::Color::Idx(1))
    );
    assert!(cell.is_some_and(|c| c.is_wide()));
    p.screen_mut().set_scrollback(0);
    p.process(b"\x1b[?1049hALT");
    assert!(p.screen().alternate_screen());
    p.process(b"\x1b[?1049l");
    assert!(p.screen().contents().contains("third"));
}

#[test]
fn palette_maps_ansi_cube_and_gray() {
    let ansi = [Rgba {
        r: 0.5,
        g: 0.,
        b: 0.,
        a: 1.,
    }; 16];
    let default = Rgba {
        r: 1.,
        g: 1.,
        b: 1.,
        a: 1.,
    };
    assert_eq!(color(vt100::Color::Default, default, &ansi), default);
    assert_eq!(color(vt100::Color::Idx(3), default, &ansi), ansi[3]);
    let cube = color(vt100::Color::Idx(196), default, &ansi);
    assert!((cube.r - 1.).abs() < 1e-6 && cube.g == 0. && cube.b == 0.);
    let gray = color(vt100::Color::Idx(232), default, &ansi);
    assert!((gray.r - 8. / 255.).abs() < 1e-6);
}

#[gpui_kit::test]
fn closing_the_shown_tab_clears_its_screen(cx: &mut TestAppContext) {
    let (fixture, panel) = testing::open(cx, TerminalPanel::new);
    cx.update(|cx| {
        panel.update(cx, |panel, cx| {
            let workspace = thinkrail_core::WorkspaceId::from("ws");
            panel.workspace = Some(workspace.clone());
            panel.tabs = vec![TerminalTab {
                workspace,
                tab_key: "t1".into(),
                title: "终端 1".into(),
                runtime: None,
            }];
            panel.selected = Some("t1".into());
            panel.link = Link::Attached {
                runtime: "rt".into(),
                offset: 0,
            };
            panel
                .parser
                .process(b"PS C:\\> echo LEFTOVER\r\nLEFTOVER\r\n");
            panel.truncated = true;
            cx.notify();
        });
    });

    let close = ElementId::NamedChild(Arc::new("term-tab-t1".into()), "close".into());
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(close, cx);
    })
    .unwrap();

    cx.update(|cx| {
        let panel = panel.read(cx);
        assert!(panel.tabs.is_empty());
        assert_eq!(panel.selected, None);
        assert!(matches!(panel.link, Link::Detached));
        assert_eq!(panel.parser.screen().contents().trim(), "");
        assert!(!panel.truncated);
    });
}
