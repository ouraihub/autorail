//! vt100 grid behavior and theme palette mapping.

use gpui_kit::Rgba;

use super::{nonzero, screen::color};

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
