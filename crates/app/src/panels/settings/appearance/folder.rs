//! The appearance page's folder row (the original's `SettingsFolderRow`): a path with
//! 复制路径 and 打开文件夹.

use gpui_kit::{
    AnyElement, ClipboardItem, ParentElement as _, Styled as _,
    assets::IconName,
    base::h_flex,
    component::{Sizable as _, button::Button},
};
use thinkrail_ui::{Roles, metrics::space};

use super::{super::rows::row, AppearancePage};
use crate::strings::settings_appearance as s;

impl AppearancePage {
    pub(super) fn folder_row(
        &self,
        key: &'static str,
        title: &'static str,
        dir: std::path::PathBuf,
        r: &Roles,
        cx: &mut gpui_kit::Context<Self>,
    ) -> AnyElement {
        let copy_dir = dir.clone();
        let shown = dir.display().to_string();
        row(
            title,
            Some(shown.clone()),
            h_flex()
                .gap(space::S8)
                .child(
                    Button::new(format!("{key}-copy"))
                        .outline()
                        .small()
                        .icon(IconName::Copy)
                        .label(if self.copied == Some(key) {
                            s::PATH_COPIED
                        } else {
                            s::COPY_PATH
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                copy_dir.display().to_string(),
                            ));
                            this.copied = Some(key);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(format!("{key}-open"))
                        .outline()
                        .small()
                        .icon(IconName::FolderOpen)
                        .label(s::OPEN_FOLDER)
                        .on_click(move |_, _, cx| {
                            if std::fs::create_dir_all(&dir).is_ok() {
                                cx.reveal_path(&dir);
                            }
                        }),
                ),
            r,
        )
    }
}
