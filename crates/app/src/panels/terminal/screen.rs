//! vt100 grid rendering: palette, cell/cursor/IME painting, viewport size, scrollback, status.

use gpui_kit::{
    App, Bounds, Context, FontStyle, FontWeight, Pixels, Rgba, SharedString, TextAlign, TextRun,
    UnderlineStyle, Window, fill, font, point, px, size,
};
use thinkrail_core::GridSize;
use thinkrail_ui::{ThinkrailTheme, fonts, metrics::text};
use tracing::debug;

use super::{Link, TerminalPanel, nonzero};
use crate::strings::terminal as s;

/// A vt100 color in the theme's palette (16 ANSI + the xterm 6×6×6 cube and gray ramp).
pub(super) fn color(value: vt100::Color, default: Rgba, ansi: &[Rgba; 16]) -> Rgba {
    let rgb = |r: u8, g: u8, b: u8| Rgba {
        r: f32::from(r) / 255.,
        g: f32::from(g) / 255.,
        b: f32::from(b) / 255.,
        a: 1.,
    };
    match value {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => rgb(r, g, b),
        vt100::Color::Idx(index) if index < 16 => ansi[usize::from(index)],
        vt100::Color::Idx(index) if index < 232 => {
            let n = index - 16;
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        vt100::Color::Idx(index) => {
            let n = 8 + (index - 232) * 10;
            rgb(n, n, n)
        }
    }
}

impl TerminalPanel {
    pub(super) fn layout(
        &mut self,
        bounds: Bounds<Pixels>,
        cell_width: f32,
        cx: &mut Context<Self>,
    ) {
        self.bounds = Some(bounds);
        self.cell_width = cell_width;
        let cols = ((f32::from(bounds.size.width) / cell_width).floor() as u16).clamp(2, 500);
        let rows =
            ((f32::from(bounds.size.height) / self.line_height).floor() as u16).clamp(1, 300);
        if self.size.cols != cols || self.size.rows != rows {
            self.size = GridSize { cols, rows };
            self.parser.set_size(nonzero(rows), nonzero(cols));
            if matches!(self.link, Link::Attached { .. }) {
                self.resize(cx);
            }
            cx.notify();
        }
    }

    pub(super) fn scroll(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.parser.screen().alternate_screen() {
            return;
        }
        let offset = self
            .parser
            .screen()
            .scrollback()
            .saturating_add_signed(delta);
        self.parser.screen_mut().set_scrollback(offset);
        cx.notify();
    }

    pub(super) fn cursor_bounds(&self) -> Option<Bounds<Pixels>> {
        let bounds = self.bounds?;
        let (row, col) = self.parser.screen().cursor_position();
        let origin = bounds.origin
            + point(
                px(f32::from(col) * self.cell_width),
                px(f32::from(row) * self.line_height),
            );
        Some(Bounds::new(
            origin,
            size(px(self.cell_width), px(self.line_height)),
        ))
    }

    pub(super) fn paint_screen(&self, window: &mut Window, cx: &mut App) {
        let Some(bounds) = self.bounds else { return };
        let theme = ThinkrailTheme::global(cx);
        let (ansi, background, foreground, cursor_color) = (
            theme.ansi.colors(),
            theme.roles.container_terminal_bg,
            theme.roles.text_default,
            theme.roles.primary,
        );
        let font_size = text::CODE_TEXT.size;
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        for row in 0..rows.get() {
            for col in 0..cols.get() {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut fg = color(cell.fgcolor(), foreground, &ansi);
                let mut bg = color(cell.bgcolor(), background, &ansi);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let origin = bounds.origin
                    + point(
                        px(f32::from(col) * self.cell_width),
                        px(f32::from(row) * self.line_height),
                    );
                if bg != background {
                    let width = self.cell_width * if cell.is_wide() { 2. } else { 1. };
                    window.paint_quad(fill(
                        Bounds::new(origin, size(px(width), px(self.line_height))),
                        bg,
                    ));
                }
                if !cell.has_contents() {
                    continue;
                }
                let mut cell_font = font(fonts::CODE);
                if cell.bold() {
                    cell_font.weight = FontWeight::BOLD;
                }
                if cell.italic() {
                    cell_font.style = FontStyle::Italic;
                }
                if cell.dim() {
                    fg.a *= 0.6;
                }
                let content = cell.contents();
                let run = TextRun {
                    len: content.len(),
                    font: cell_font,
                    color: fg.into(),
                    background_color: None,
                    underline: cell.underline().then_some(UnderlineStyle {
                        color: Some(fg.into()),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(
                    SharedString::from(content.to_owned()),
                    font_size,
                    &[run],
                    None,
                );
                if let Err(error) = line.paint(
                    origin,
                    px(self.line_height),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                ) {
                    debug!(%error, "terminal cell paint failed");
                }
            }
        }
        if screen.scrollback() == 0
            && !screen.hide_cursor()
            && self.runtime().is_some()
            && let Some(cursor) = self.cursor_bounds()
        {
            let thickness = if self.focus.is_focused(window) {
                2.
            } else {
                1.
            };
            let bar = Bounds::new(
                point(cursor.left(), cursor.bottom() - px(thickness)),
                size(cursor.size.width, px(thickness)),
            );
            window.paint_quad(fill(bar, cursor_color));
        }
        if !self.composition.is_empty()
            && let Some(cursor) = self.cursor_bounds()
        {
            let run = TextRun {
                len: self.composition.len(),
                font: font(fonts::CODE),
                color: foreground.into(),
                background_color: Some(background.into()),
                underline: Some(UnderlineStyle {
                    color: None,
                    thickness: px(1.),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                self.composition.clone().into(),
                font_size,
                &[run],
                None,
            );
            if let Err(error) = line.paint(
                cursor.origin,
                px(self.line_height),
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                debug!(%error, "terminal composition paint failed");
            }
        }
    }

    pub(super) fn status_line(&self) -> String {
        let state = match &self.link {
            Link::Detached if self.tabs.is_empty() => s::NO_TABS.to_owned(),
            Link::Detached => s::DETACHED.to_owned(),
            Link::Attaching => s::ATTACHING.to_owned(),
            Link::Attached { .. } => format!("{}×{}", self.size.cols, self.size.rows),
            Link::Exited(code) => {
                format!(
                    "{}（{} {}）",
                    s::EXITED,
                    s::EXIT_CODE,
                    code.map_or_else(|| "?".to_owned(), |c| c.to_string())
                )
            }
        };
        let truncated = if self.truncated {
            format!(" · {}", s::TRUNCATED)
        } else {
            String::new()
        };
        format!("{state}{truncated} · {}", s::COPY_HINT)
    }
}
