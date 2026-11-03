//! Document rows (a file's lines, a diff's lines, hunk and file headers, notes), the pieces they
//! are cut into at the wrap column (08 A19), and drawing one piece.

use std::ops::Range;

use gpui_kit::{
    AnyElement, IntoElement, ParentElement as _, Pixels, Rgba, Styled as _, base::h_flex, div,
    prelude::FluentBuilder as _, px,
};
use thinkrail_core::FileContent;
use thinkrail_ui::{
    Roles,
    components::TextStyled as _,
    metrics::{size, space, text},
};

use super::{diff, wrap};
use crate::strings::files as s;

const LINE_NUMBER_WIDTH: Pixels = px(56.);
const DIFF_NUMBER_WIDTH: Pixels = px(40.);

/// One rendered row of a document.
#[derive(Debug, Clone)]
pub(super) enum Row {
    Code { number: u32, text: String },
    Diff(diff::Line),
    Hunk(String),
    FileHeader(String),
    Note(String),
}

/// One drawn line: `range` of its row's text (all of it unless the row wraps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Piece {
    pub(super) row: usize,
    pub(super) range: Range<usize>,
}

pub(super) fn file_rows(content: FileContent) -> Vec<Row> {
    match content {
        FileContent::Text(text) => text
            .lines()
            .enumerate()
            .map(|(i, line)| Row::Code {
                number: i as u32 + 1,
                text: line.to_owned(),
            })
            .collect(),
        FileContent::Binary { byte_len } => vec![Row::Note(format!(
            "{}（{byte_len} {}）",
            s::BINARY,
            s::BYTES
        ))],
    }
}

pub(super) fn diff_rows(text: &str) -> Vec<Row> {
    let files = diff::parse(text);
    if files.is_empty()
        || files
            .iter()
            .all(|f| f.hunks.is_empty() && !f.binary && f.old_path.is_none())
    {
        return vec![Row::Note(s::NO_DIFF.to_owned())];
    }
    let several = files.len() > 1;
    let mut rows = Vec::new();
    for file in files {
        if several {
            rows.push(Row::FileHeader(file.path.clone()));
        }
        if let Some(old) = file.old_path.as_ref().filter(|old| **old != file.path) {
            rows.push(Row::Note(format!("{} {old}", s::RENAMED_FROM)));
        }
        if file.binary {
            rows.push(Row::Note(s::BINARY_DIFF.to_owned()));
        }
        for hunk in file.hunks {
            rows.push(Row::Hunk(hunk.header));
            rows.extend(hunk.lines.into_iter().map(Row::Diff));
        }
    }
    rows
}

fn number(n: Option<u32>) -> String {
    n.map(|n| n.to_string()).unwrap_or_default()
}

impl Row {
    /// The text that wraps; headers and notes draw whole on one line.
    fn wrapped(&self) -> Option<&str> {
        match self {
            Row::Code { text, .. } => Some(text),
            Row::Diff(line) => Some(&line.text),
            Row::Hunk(_) | Row::FileHeader(_) | Row::Note(_) => None,
        }
    }
}

/// Every row cut at `column` (08 A19).
pub(super) fn cut(rows: &[Row], column: usize) -> Vec<Piece> {
    let mut pieces = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        match row.wrapped() {
            Some(text) => pieces.extend(
                wrap::pieces(text, column)
                    .into_iter()
                    .map(|range| Piece { row: index, range }),
            ),
            None => pieces.push(Piece {
                row: index,
                range: 0..0,
            }),
        }
    }
    pieces
}

/// Width left of the wrapped text: padding plus the line number, or the diff gutter and sign.
pub(super) fn gutter_width(diff: bool) -> Pixels {
    let numbers = if diff {
        DIFF_NUMBER_WIDTH * 2. + space::S4 + space::S8 + space::S16
    } else {
        LINE_NUMBER_WIDTH
    };
    space::S8 * 2. + numbers
}

/// `width`: the row's fixed width when the list scrolls sideways; else it fills the pane.
pub(super) fn render_piece(
    rows: &[Row],
    piece: &Piece,
    width: Option<Pixels>,
    r: &Roles,
) -> AnyElement {
    let row = &rows[piece.row];
    let first = piece.range.start == 0;
    let shown = |text: &str| wrap::expand(&text[piece.range.clone()]);
    let gutter = |old: String, new: String| {
        h_flex()
            .flex_none()
            .gap(space::S4)
            .pr(space::S8)
            .text_color(r.text_subtle)
            .child(div().w(DIFF_NUMBER_WIDTH).flex().justify_end().child(old))
            .child(div().w(DIFF_NUMBER_WIDTH).flex().justify_end().child(new))
    };
    let line = |content: String, color: Rgba| {
        div()
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_color(color)
            .child(content)
    };
    let base = h_flex()
        .map(|d| match width {
            Some(width) => d.w(width).flex_none(),
            None => d.w_full(),
        })
        .h(size::ROW)
        .px(space::S8)
        .text_style(text::CODE_DOCUMENT);
    match row {
        Row::Code { number: n, text: t } => base
            .child(
                div()
                    .flex_none()
                    .w(LINE_NUMBER_WIDTH)
                    .pr(space::S12)
                    .flex()
                    .justify_end()
                    .text_color(r.text_subtle)
                    .when(first, |d| d.child(n.to_string())),
            )
            .child(line(shown(t), r.text_default))
            .into_any_element(),
        Row::Diff(l) => {
            let (sign, bg, fg) = match l.kind {
                diff::Kind::Added => ("+", Some(r.feedback_success_subtle), r.feedback_success),
                diff::Kind::Removed => ("−", Some(r.feedback_error_subtle), r.feedback_error),
                diff::Kind::Context => (" ", None, r.text_muted),
                diff::Kind::Note => (" ", None, r.text_subtle),
            };
            let (old, new, sign) = if first {
                (number(l.old), number(l.new), sign)
            } else {
                (String::new(), String::new(), " ")
            };
            base.when_some(bg, |d, bg| d.bg(bg))
                .child(gutter(old, new))
                .child(div().flex_none().w(space::S16).text_color(fg).child(sign))
                .child(line(
                    shown(&l.text),
                    if l.kind == diff::Kind::Context {
                        r.text_default
                    } else {
                        fg
                    },
                ))
                .into_any_element()
        }
        Row::Hunk(header) => base
            .bg(r.container_content_bg)
            .child(line(header.clone(), r.text_subtle))
            .into_any_element(),
        Row::FileHeader(path) => base
            .bg(r.container_header_bg)
            .child(
                div()
                    .text_style(text::TITLE_COMPACT)
                    .text_color(r.text_default)
                    .child(path.clone()),
            )
            .into_any_element(),
        Row::Note(note) => base
            .child(line(note.clone(), r.text_subtle))
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests;
