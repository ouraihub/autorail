//! API 密钥管理对话框：列表（标签、掩码、复制、编辑、删除、启停）、添加 / 修改、页脚“N / M 已启用”。

use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window,
    base::{h_flex, v_flex},
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        input::{Input, InputState},
        switch::Switch,
    },
    px,
};
use thinkrail_core::{
    ProviderId, ProviderKeyId, ProviderKeyInput, ProviderKeySummary, ProviderKeyUpdate, Secret,
};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, label},
    metrics::{radius, space, text},
};

use super::{ProvidersPage, error_text, write};
use crate::{state::run, strings::settings_providers as s};

struct KeysDialog {
    page: Entity<ProvidersPage>,
    provider: ProviderId,
    label: Entity<InputState>,
    value: Entity<InputState>,
    editing: Option<ProviderKeyId>,
    error: Option<&'static str>,
    _subscription: Subscription,
}

impl ProvidersPage {
    /// Opens key management; `edit` preloads that key in the form (the pane's ✎ button).
    pub(super) fn open_keys_dialog(
        &mut self,
        edit: Option<ProviderKeySummary>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self.selected.clone() else {
            return;
        };
        let page = cx.entity();
        let label = edit
            .as_ref()
            .and_then(|k| k.label.clone())
            .unwrap_or_default();
        let dialog = cx.new(|cx| KeysDialog {
            _subscription: cx.observe(&page, |_, _, cx| cx.notify()),
            page,
            provider,
            label: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(s::KEYS_LABEL)
                    .default_value(label)
            }),
            value: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(s::KEY_PLACEHOLDER)
                    .masked(true)
            }),
            editing: edit.map(|k| k.id),
            error: None,
        });
        window.open_dialog(cx, move |d, _, _| {
            d.title(s::KEYS_MANAGE)
                .w(px(520.))
                .child(dialog.clone())
                .on_ok(|_, _, _| false)
                .footer(
                    DialogFooter::new().child(
                        Button::new("keys-close")
                            .outline()
                            .label(s::CLOSE)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
                )
        });
    }

    /// Copies one key's plaintext; it goes only to the clipboard.
    pub(super) fn copy_key(&mut self, key: ProviderKeyId, cx: &mut Context<Self>) {
        let service = self.service(cx);
        run(
            cx,
            move || service.reveal_key(&key),
            |this, result, cx| match result {
                Ok(secret) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(secret.expose().to_owned()))
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }
}

impl KeysDialog {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let label = Some(self.label.read(cx).value().trim().to_owned()).filter(|l| !l.is_empty());
        let value = self.value.read(cx).value().trim().to_owned();
        let id = self.provider.clone();
        let enabled = self
            .editing
            .as_ref()
            .and_then(|key| self.keys(cx).into_iter().find(|k| &k.id == key))
            .is_none_or(|k| k.enabled);
        match self.editing.take() {
            Some(key_id) => {
                let key = (!value.is_empty()).then(|| Secret::new(value));
                self.page.update(cx, |page, cx| {
                    write(page, cx, move |svc| {
                        svc.update_key(
                            &id,
                            &key_id,
                            ProviderKeyUpdate {
                                label,
                                key,
                                enabled,
                            },
                        )
                    })
                });
            }
            None if value.is_empty() => {
                self.error = Some(s::KEY_EMPTY);
                cx.notify();
                return;
            }
            None => {
                let key = ProviderKeyInput {
                    label,
                    key: Secret::new(value),
                    enabled: true,
                };
                self.page.update(cx, |page, cx| {
                    write(page, cx, move |svc| svc.add_key(&id, key))
                });
            }
        }
        self.error = None;
        self.label.update(cx, |i, cx| i.set_value("", window, cx));
        self.value.update(cx, |i, cx| i.set_value("", window, cx));
        cx.notify();
    }

    fn keys(&self, cx: &gpui_kit::App) -> Vec<thinkrail_core::ProviderKeySummary> {
        self.page
            .read(cx)
            .providers
            .iter()
            .find(|p| p.id == self.provider)
            .map(|p| p.keys.clone())
            .unwrap_or_default()
    }
}

impl Render for KeysDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let keys = self.keys(cx);
        let enabled = keys.iter().filter(|k| k.enabled).count();
        let rows = keys.iter().map(|key| {
            let (copy, edit, del, toggle) = (key.id.clone(), key.clone(), key.clone(), key.clone());
            let provider = self.provider.clone();
            let provider_toggle = self.provider.clone();
            h_flex()
                .gap(space::S6)
                .px(space::S8)
                .h(px(36.))
                .rounded(radius::MD)
                .border_1()
                .border_color(r.border_muted)
                .child(
                    label(
                        text::UI,
                        key.label
                            .clone()
                            .unwrap_or_else(|| s::KEY_DEFAULT_LABEL.into()),
                    )
                    .w(px(120.))
                    .truncate(),
                )
                .child(
                    label(text::CODE_TEXT, key.masked.clone())
                        .flex_1()
                        .text_color(r.text_muted),
                )
                .child(
                    IconButton::new(
                        format!("key-copy-{}", key.id),
                        gpui_kit::assets::IconName::Copy,
                    )
                    .tooltip(s::KEY_COPY)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let key = copy.clone();
                        this.page.update(cx, |page, cx| page.copy_key(key, cx))
                    })),
                )
                .child(
                    IconButton::new(
                        format!("key-edit-{}", key.id),
                        gpui_kit::assets::IconName::Pencil,
                    )
                    .tooltip(s::KEY_EDIT)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.editing = Some(edit.id.clone());
                        this.error = None;
                        this.label.update(cx, |i, cx| {
                            i.set_value(edit.label.clone().unwrap_or_default(), window, cx)
                        });
                        this.value.update(cx, |i, cx| {
                            i.set_value("", window, cx);
                            i.focus(window, cx);
                        });
                        cx.notify();
                    })),
                )
                .child(
                    IconButton::new(
                        format!("key-delete-{}", key.id),
                        gpui_kit::assets::IconName::Trash,
                    )
                    .tooltip(s::KEY_DELETE)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let (id, key) = (provider.clone(), del.id.clone());
                        this.page.update(cx, |page, cx| {
                            write(page, cx, move |svc| svc.delete_key(&id, &key))
                        });
                    })),
                )
                .child(
                    Switch::new(format!("key-enabled-{}", key.id))
                        .checked(key.enabled)
                        .color(r.feedback_success)
                        .on_click(cx.listener(move |this, value: &bool, _, cx| {
                            let (id, key, enabled) =
                                (provider_toggle.clone(), toggle.clone(), *value);
                            this.page.update(cx, |page, cx| {
                                write(page, cx, move |svc| {
                                    svc.update_key(
                                        &id,
                                        &key.id,
                                        ProviderKeyUpdate {
                                            label: key.label,
                                            key: None,
                                            enabled,
                                        },
                                    )
                                })
                            });
                        })),
                )
        });
        let editing = self.editing.is_some();
        v_flex()
            .gap(space::S8)
            .child(label(text::METADATA, s::KEYS_DESC).text_color(r.text_subtle))
            .children(
                keys.is_empty()
                    .then(|| label(text::METADATA, s::KEYS_EMPTY).text_color(r.text_subtle)),
            )
            .children(rows)
            .child(
                h_flex()
                    .gap(space::S6)
                    .child(Input::new(&self.label).w(px(120.)))
                    .child(Input::new(&self.value).mask_toggle().flex_1())
                    .child(
                        Button::new("keys-submit")
                            .primary()
                            .label(if editing { s::SAVE } else { s::KEYS_ADD })
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
            .children(
                editing.then(|| label(text::METADATA, s::KEYS_KEEP).text_color(r.text_subtle)),
            )
            .children(error_text(self.error.map(str::to_owned), cx))
            .child(
                label(text::METADATA, s::keys_enabled(enabled, keys.len()))
                    .text_color(r.text_muted),
            )
    }
}
