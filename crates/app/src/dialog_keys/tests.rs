//! 对话框里聚焦的按钮按回车：触发按钮，不触发对话框的「确定」。

use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    Context, Entity, FocusHandle, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, base::Dialog,
    component::button::Button, px,
};

struct Harness {
    dialog: FocusHandle,
    button: FocusHandle,
    pressed: Rc<Cell<usize>>,
    confirmed: Rc<Cell<usize>>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (pressed, confirmed) = (self.pressed.clone(), self.confirmed.clone());
        Dialog::new(cx)
            .focus_handle(self.dialog.clone())
            .on_ok(move |_, _, _| {
                confirmed.set(confirmed.get() + 1);
                false
            })
            .popup(
                Button::new("in-dialog")
                    .label("Press")
                    .w(px(100.))
                    .track_focus(&self.button)
                    .on_click(move |_, _, _| pressed.set(pressed.get() + 1)),
            )
    }
}

struct Opened<'a> {
    cx: &'a mut VisualTestContext,
    view: Entity<Harness>,
    pressed: Rc<Cell<usize>>,
    confirmed: Rc<Cell<usize>>,
}

fn open(cx: &mut TestAppContext, fixed: bool) -> Opened<'_> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        if fixed {
            super::init(cx);
        }
    });
    let (pressed, confirmed) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (view, cx) = cx.add_window_view({
        let (pressed, confirmed) = (pressed.clone(), confirmed.clone());
        // 按钮渲染时自己把句柄标成 Tab 停靠点；对话框外壳不是停靠点。
        move |_, cx| Harness {
            dialog: cx.focus_handle(),
            button: cx.focus_handle(),
            pressed,
            confirmed,
        }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    Opened {
        cx,
        view,
        pressed,
        confirmed,
    }
}

impl Opened<'_> {
    fn focus(&mut self, pick: fn(&Harness) -> &FocusHandle) {
        let view = self.view.clone();
        self.cx.update(|window, cx| {
            pick(view.read(cx)).clone().focus(window, cx);
            window.draw(cx).clear(cx);
        });
    }

    fn press(&mut self, key: &'static str) {
        let keystroke = Keystroke::parse(key).unwrap();
        self.cx.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        self.cx.simulate_event(KeyUpEvent { keystroke });
        self.cx.run_until_parked();
    }

    fn counts(&self) -> (usize, usize) {
        (self.pressed.get(), self.confirmed.get())
    }
}

#[gpui_kit::test]
fn enter_presses_the_focused_button_and_skips_the_dialog_ok(cx: &mut TestAppContext) {
    let mut dialog = open(cx, true);
    dialog.focus(|h| &h.button);
    dialog.press("enter");
    assert_eq!(dialog.counts(), (1, 0));
    dialog.press("space");
    assert_eq!(dialog.counts(), (2, 0), "Space keeps working");
}

#[gpui_kit::test]
fn enter_on_the_dialog_surface_still_confirms(cx: &mut TestAppContext) {
    let mut dialog = open(cx, true);
    dialog.focus(|h| &h.dialog);
    dialog.press("enter");
    assert_eq!(dialog.counts(), (0, 1));
}

/// 本模块要修的问题：只有 gpui-kit 时，回车交给了对话框的「确定」。
#[gpui_kit::test]
fn without_the_fix_enter_goes_to_the_dialog_ok(cx: &mut TestAppContext) {
    let mut dialog = open(cx, false);
    dialog.focus(|h| &h.button);
    dialog.press("enter");
    assert_eq!(dialog.counts(), (0, 1));
}
