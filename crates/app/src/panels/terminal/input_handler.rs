//! Keyboard, clipboard paste and platform IME (`EntityInputHandler`) input routed into the PTY.

use std::ops::Range;

use gpui_kit::{
    Bounds, ClipboardItem, Context, EntityInputHandler, KeyDownEvent, Pixels, Point,
    UTF16Selection, Window,
};

use super::{
    TerminalPanel,
    input::{encode_key, replace_composition, utf16_byte},
};

impl TerminalPanel {
    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let text = if self.parser.screen().bracketed_paste() {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        self.write(&text, cx);
    }

    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        let m = key.modifiers;
        if event.prefer_character_input || !self.composition.is_empty() {
            return;
        }
        if ((m.control && m.shift) || m.platform) && key.key == "v"
            || (m.shift && key.key == "insert")
        {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.paste_text(&text, cx);
            }
            cx.stop_propagation();
            return;
        }
        if ((m.control && m.shift) || m.platform) && key.key == "c" {
            cx.write_to_clipboard(ClipboardItem::new_string(self.parser.screen().contents()));
            cx.stop_propagation();
            return;
        }
        if m.shift && matches!(key.key.as_str(), "pageup" | "pagedown") {
            let half = isize::try_from(self.size.rows / 2).unwrap_or(1);
            self.scroll(if key.key == "pageup" { half } else { -half }, cx);
            cx.stop_propagation();
            return;
        }
        if let Some(data) = encode_key(key, self.parser.screen().application_cursor()) {
            self.write(&data, cx);
            cx.stop_propagation();
            window.prevent_default();
        }
    }
}

impl EntityInputHandler for TerminalPanel {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let start = utf16_byte(&self.composition, range.start);
        let end = utf16_byte(&self.composition, range.end).max(start);
        *actual = Some(
            self.composition[..start].encode_utf16().count()
                ..self.composition[..end].encode_utf16().count(),
        );
        Some(self.composition[start..end].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.composition_selection.clone(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.composition.is_empty()).then(|| 0..self.composition.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let text = std::mem::take(&mut self.composition);
        self.composition_selection = 0..0;
        self.write(&text, cx);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = replace_composition(&self.composition, range, text);
        self.composition.clear();
        self.composition_selection = 0..0;
        self.write(&text, cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.runtime().is_none() {
            return;
        }
        let start = range
            .as_ref()
            .map_or(0, |r| r.start.min(self.composition.encode_utf16().count()));
        self.composition = replace_composition(&self.composition, range, text);
        let length = self.composition.encode_utf16().count();
        self.composition_selection = selection.map_or(length..length, |r| {
            (start + r.start).min(length)..(start + r.end).min(length)
        });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds()
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        self.runtime().is_some()
    }
}
