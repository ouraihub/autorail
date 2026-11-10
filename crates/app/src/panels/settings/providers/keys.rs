//! Native key labels/enabled state and async writes; shared key rows and manager composition.

use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window,
    component::{
        Disableable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState},
        switch::Switch,
    },
};
use lumakit_design::{components::settings::KeyRow, metrics::size};
use lumakit_gpui::providers as ui;
use thinkrail_core::{
    ProviderId, ProviderKeyId, ProviderKeyInput, ProviderKeySummary, ProviderKeyUpdate, Secret,
};
use thinkrail_ui::{components::label, metrics::text};

use super::{ProvidersPage, error_text, theme, write};
use crate::{state::run, strings::settings_providers as s};

#[cfg(test)]
mod tests;

struct KeysDialog {
    page: Entity<ProvidersPage>,
    provider: ProviderId,
    label: Entity<InputState>,
    value: Entity<InputState>,
    editing: Option<ProviderKeyId>,
    error: Option<String>,
    saving: bool,
    generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl ProvidersPage {
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
        let name = edit
            .as_ref()
            .and_then(|k| k.label.clone())
            .unwrap_or_default();
        let dialog = cx.new(|cx| {
            let label = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(s::KEYS_LABEL)
                    .default_value(name)
            });
            let value = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(s::KEY_PLACEHOLDER)
                    .masked(true)
            });
            let subscriptions = vec![
                cx.observe(&page, |_, _, cx| cx.notify()),
                cx.subscribe(&label, |this: &mut KeysDialog, _, event: &InputEvent, _| {
                    if matches!(event, InputEvent::Change) {
                        this.generation = this.generation.wrapping_add(1);
                    }
                }),
                cx.subscribe(&value, |this: &mut KeysDialog, _, event: &InputEvent, _| {
                    if matches!(event, InputEvent::Change) {
                        this.generation = this.generation.wrapping_add(1);
                    }
                }),
            ];
            KeysDialog {
                page,
                provider,
                label,
                value,
                editing: edit.map(|k| k.id),
                error: None,
                saving: false,
                generation: 0,
                _subscriptions: subscriptions,
            }
        });
        window.open_dialog(cx, move |d, _, cx| {
            let footer = ui::form_footer(
                None,
                Button::new("keys-close")
                    .outline()
                    .label(s::CLOSE)
                    .on_click(|_, window, cx| window.close_dialog(cx))
                    .into_any_element(),
                None,
            );
            d.w(size::SETTINGS_DIALOG)
                .child(ui::compact_form_body(
                    s::KEYS_MANAGE,
                    dialog.clone().into_any_element(),
                    footer.into_any_element(),
                    None,
                    &theme::roles(cx),
                ))
                .on_ok(|_, _, _| false)
        });
    }

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
        if self.saving {
            return;
        }
        let label = Some(self.label.read(cx).value().trim().to_owned()).filter(|l| !l.is_empty());
        let value = self.value.read(cx).value().trim().to_owned();
        if self.editing.is_none() && value.is_empty() {
            self.error = Some(s::KEY_EMPTY.into());
            cx.notify();
            return;
        }
        let enabled = self
            .editing
            .as_ref()
            .and_then(|key| self.keys(cx).into_iter().find(|k| &k.id == key))
            .is_none_or(|k| k.enabled);
        let editing = self.editing.clone();
        let id = self.provider.clone();
        let generation = self.generation;
        let service = self.page.read(cx).service(cx);
        let handle = window.window_handle();
        self.saving = true;
        self.error = None;
        run(
            cx,
            move || match editing {
                Some(key_id) => service.update_key(
                    &id,
                    &key_id,
                    ProviderKeyUpdate {
                        label,
                        key: (!value.is_empty()).then(|| Secret::new(value)),
                        enabled,
                    },
                ),
                None => service.add_key(
                    &id,
                    ProviderKeyInput {
                        label,
                        key: Secret::new(value),
                        enabled: true,
                    },
                ),
            },
            move |this, result, cx| {
                this.saving = false;
                match result {
                    Ok(provider) => {
                        this.page.update(cx, |page, cx| page.stored(provider, cx));
                        if this.generation == generation {
                            this.editing = None;
                            let (label, value) = (this.label.clone(), this.value.clone());
                            if handle
                                .update(cx, |_, window, cx| {
                                    label.update(cx, |i, cx| i.set_value("", window, cx));
                                    value.update(cx, |i, cx| i.set_value("", window, cx));
                                })
                                .is_err()
                            {
                                tracing::debug!("providers.keys.window_closed");
                            }
                        }
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.page.update(cx, |page, cx| page.report(&error, cx));
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn keys(&self, cx: &gpui_kit::App) -> Vec<ProviderKeySummary> {
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
        let r = theme::roles(cx);
        let keys = self.keys(cx);
        let enabled = keys.iter().filter(|k| k.enabled).count();
        let rows: Vec<_> = keys
            .iter()
            .map(|key| {
                let (copy, edit, del, toggle) =
                    (key.id.clone(), key.clone(), key.clone(), key.clone());
                let (provider, provider_toggle) = (self.provider.clone(), self.provider.clone());
                KeyRow::new(format!("managed-key-{}", key.id), key.masked.clone(), false)
                    .roles(r)
                    .label(
                        key.label
                            .clone()
                            .unwrap_or_else(|| s::KEY_DEFAULT_LABEL.into()),
                    )
                    .all_actions(true)
                    .disabled(self.saving)
                    .on_copy(cx.listener(move |this, _, _, cx| {
                        let key = copy.clone();
                        this.page.update(cx, |page, cx| page.copy_key(key, cx));
                    }))
                    .on_edit(cx.listener(move |this, _, window, cx| {
                        this.generation = this.generation.wrapping_add(1);
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
                    }))
                    .on_delete(cx.listener(move |this, _, _, cx| {
                        let (id, key) = (provider.clone(), del.id.clone());
                        this.page.update(cx, |page, cx| {
                            write(page, cx, move |svc| svc.delete_key(&id, &key))
                        });
                    }))
                    .trailing(
                        Switch::new(format!("key-enabled-{}", key.id))
                            .checked(key.enabled)
                            .color(r.feedback_success)
                            .disabled(self.saving)
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
                    .into_any_element()
            })
            .collect();
        let mut fields = vec![
            ui::key_entry(
                Input::new(&self.label)
                    .aria_label(s::KEYS_LABEL)
                    .into_any_element(),
                Input::new(&self.value)
                    .aria_label(s::API_KEY)
                    .mask_toggle()
                    .into_any_element(),
                Button::new("keys-submit")
                    .primary()
                    .label(if self.editing.is_some() {
                        s::SAVE
                    } else {
                        s::KEYS_ADD
                    })
                    .disabled(self.saving)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
                    .into_any_element(),
            )
            .into_any_element(),
        ];
        if self.editing.is_some() {
            fields.push(
                label(text::METADATA, s::KEYS_KEEP)
                    .text_color(r.text_subtle)
                    .into_any_element(),
            );
        }
        if let Some(error) = error_text(self.error.clone(), cx) {
            fields.push(error.into_any_element());
        }
        if keys.is_empty() {
            fields.push(ui::empty(s::KEYS_EMPTY, &r));
        }
        ui::key_manager(
            rows,
            fields,
            label(text::METADATA, s::keys_enabled(enabled, keys.len()))
                .text_color(r.text_muted)
                .into_any_element(),
            label(text::METADATA, s::KEYS_DESC)
                .text_color(r.text_subtle)
                .into_any_element(),
        )
    }
}
