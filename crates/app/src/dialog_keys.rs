//! 对话框里聚焦的按钮、链接、开关按回车时，触发该控件本身。
//!
//! gpui-kit 在 `Dialog` 按键上下文里绑定了 `enter → Confirm`，按键绑定又先于元素自己的
//! key-up 点击执行，所以回车总被对话框的「确定」吃掉（`on_ok(|..| false)` 时什么都不发生），
//! 到不了聚焦的控件。对话框没有绑定空格，所以空格一直可用。
//!
//! 修复只放在本应用里（gpui-kit 是第三方代码）。普通按钮不设按键上下文，绑定无法单独
//! 指向它；拦截器停掉回车又会连元素的 key-down/key-up 一起停掉。所以当焦点在对话框内、
//! 最内层上下文就是对话框（即聚焦的控件自己不处理按键）且该控件可 Tab 聚焦时，拦截器
//! 停掉这次回车，改为向同一焦点补发一次空格：按钮、链接、开关都把空格当作点击。
//! 输入框、下拉、列表、菜单会压入自己的上下文并保留自己的回车绑定；焦点停在对话框外壳
//! 自身（外壳句柄不是 Tab 停靠点）时，回车仍然触发「确定」。

use gpui_kit::{App, KeyContext, KeyDownEvent, KeyUpEvent, Keystroke, PlatformInput};

const DIALOG: &str = "Dialog";

/// 安装拦截器，存活到应用退出。
pub fn init(cx: &mut App) {
    cx.intercept_keystrokes(|event, window, cx| {
        let stroke = &event.keystroke;
        if stroke.key != "enter"
            || stroke.modifiers.modified()
            || !innermost_is_dialog(&event.context_stack)
            || !window.focused(cx).is_some_and(|focus| focus.tab_stop)
        {
            return;
        }
        cx.stop_propagation();
        window.defer(cx, |window, cx| {
            let space = Keystroke {
                key: "space".into(),
                key_char: Some(" ".into()),
                modifiers: Default::default(),
            };
            window.dispatch_event(
                PlatformInput::KeyDown(KeyDownEvent {
                    keystroke: space.clone(),
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
            window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: space }), cx);
        });
    })
    .detach();
}

fn innermost_is_dialog(stack: &[KeyContext]) -> bool {
    stack.last().is_some_and(|context| context.contains(DIALOG))
}

#[cfg(test)]
mod tests;
