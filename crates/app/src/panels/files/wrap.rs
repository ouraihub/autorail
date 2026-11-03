//! Line wrapping for the file and diff viewer (08 A14, A19): the wrap column, then each line cut
//! into pieces no wider than it, counted in terminal-style columns (CJK 2, tab to the next stop).

use std::ops::Range;

use unicode_width::UnicodeWidthChar as _;

pub const TAB_STOP: usize = 4;

/// Monaco's `wordWrap`: `bounded` wraps at the narrower of the setting and what the pane shows,
/// otherwise always at the setting (a narrower pane scrolls). `fits`: whole columns on screen.
pub fn column(line_width: u16, bounded: bool, fits: usize) -> usize {
    let width = usize::from(line_width);
    if bounded {
        width.min(fits.max(1))
    } else {
        width
    }
}

/// Byte ranges of `text`'s pieces, each at most `column` columns wide; an empty line is one empty
/// piece. A character wider than the column still gets a piece of its own.
pub fn pieces(text: &str, column: usize) -> Vec<Range<usize>> {
    let column = column.max(1);
    let mut out = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (index, ch) in text.char_indices() {
        let width = char_width(ch, used);
        if used + width > column && used > 0 {
            out.push(start..index);
            start = index;
            used = 0;
        }
        used += char_width(ch, used);
    }
    out.push(start..text.len());
    out
}

/// A piece with its tabs expanded to spaces, so it draws as wide as it was counted.
pub fn expand(piece: &str) -> String {
    if !piece.contains('\t') {
        return piece.to_owned();
    }
    let mut out = String::with_capacity(piece.len() + TAB_STOP);
    let mut used = 0;
    for ch in piece.chars() {
        let width = char_width(ch, used);
        if ch == '\t' {
            out.extend(std::iter::repeat_n(' ', width));
        } else {
            out.push(ch);
        }
        used += width;
    }
    out
}

fn char_width(ch: char, used: usize) -> usize {
    if ch == '\t' {
        TAB_STOP - used % TAB_STOP
    } else {
        ch.width().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(text: &str, column: usize) -> Vec<&str> {
        pieces(text, column).into_iter().map(|r| &text[r]).collect()
    }

    fn width(text: &str) -> usize {
        text.chars().fold(0, |used, ch| used + char_width(ch, used))
    }

    #[test]
    fn the_column_follows_monaco_bounded_and_column_modes() {
        assert_eq!(column(120, true, 80), 80, "a narrow pane wraps sooner");
        assert_eq!(column(120, true, 300), 120, "a wide pane keeps the setting");
        assert_eq!(
            column(120, false, 80),
            120,
            "unbounded never follows the pane"
        );
        assert_eq!(
            column(120, true, 0),
            1,
            "a collapsed pane still has a column"
        );
    }

    #[test]
    fn lines_are_cut_at_the_column() {
        assert_eq!(cut("", 4), vec![""]);
        assert_eq!(cut("abcd", 4), vec!["abcd"]);
        assert_eq!(cut("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wide_characters_and_tabs_count_by_columns() {
        assert_eq!(
            cut("中文字符", 5),
            vec!["中文", "字符"],
            "a CJK char never splits"
        );
        assert_eq!(cut("a中", 2), vec!["a", "中"]);
        assert_eq!(cut("中", 1), vec!["中"], "wider than the column: alone");
        assert_eq!(
            cut("\tab", 4),
            vec!["\t", "ab"],
            "a tab runs to the next stop"
        );
        assert_eq!(cut("ab\tc", 4), vec!["ab\t", "c"]);
        assert_eq!(width("ab\tc中"), 7);
    }

    #[test]
    fn tabs_expand_to_their_counted_width() {
        assert_eq!(expand("ab\tc"), "ab  c");
        assert_eq!(expand("\t\tx"), "        x");
        assert_eq!(width(&expand("ab\tc")), width("ab\tc"));
    }
}
