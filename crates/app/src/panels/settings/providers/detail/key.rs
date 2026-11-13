//! 详情页的 API 密钥区：密钥行、显示明文、首个密钥输入与按线路检测。

use super::super::theme;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        switch::Switch,
    },
    prelude::FluentBuilder as _,
};
use lumakit_design::components::settings::KeyRow;
use lumakit_gpui::providers as ui;
use thinkrail_core::{KeyCheck, ProbeTarget, Provider, provider_preset};
use thinkrail_ui::{
    components::label,
    metrics::{space, text},
};

use crate::{
    panels::settings::providers::{ProvidersPage, error_text, write},
    state::run,
    strings::settings_providers as s,
};

impl ProvidersPage {
    pub(super) fn render_key(&self, provider: &Provider, cx: &mut Context<Self>) -> AnyElement {
        let r = theme::roles(cx);
        let key_url = provider
            .preset_id
            .as_deref()
            .and_then(provider_preset)
            .map(|p| p.api_key_url);
        let enabled: Vec<_> = provider.keys.iter().filter(|k| k.enabled).collect();
        let shown = enabled.first().copied().or(provider.keys.first());
        let value = match shown {
            None => Input::new(&self.key_input).flex_1().into_any_element(),
            Some(key) => {
                let (id, copy, edit) = (key.id.clone(), key.id.clone(), key.clone());
                let text_value = self.revealed.clone().unwrap_or_else(|| key.masked.clone());
                KeyRow::new("provider-key", text_value, self.revealed.is_some())
                    .roles(r)
                    .hover_actions(true)
                    .more(enabled.len().saturating_sub(1))
                    .when_some(key.label.clone(), |row, label| row.label(label))
                    .on_copy(cx.listener(move |this, _, _, cx| this.copy_key(copy.clone(), cx)))
                    .on_edit(cx.listener(move |this, _, window, cx| {
                        this.open_keys_dialog(Some(edit.clone()), window, cx)
                    }))
                    .on_reveal(
                        cx.listener(move |this, _, _, cx| this.toggle_reveal(id.clone(), cx)),
                    )
                    .into_any_element()
            }
        };
        let saved = !self.dirty && !self.saving && self.failed.is_none();
        ui::key_section(
            key_url.map(|url| {
                Button::new("provider-get-key")
                    .link()
                    .xsmall()
                    .label(s::GET_KEY)
                    .on_click(move |_, _, cx| cx.open_url(url))
                    .into_any_element()
            }),
            value,
            Button::new("provider-keys")
                .outline()
                .icon(IconName::KeyRound)
                .tooltip(s::KEYS_MANAGE)
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_keys_dialog(None, window, cx)),
                )
                .into_any_element(),
            Button::new("provider-check")
                .outline()
                .label(if self.checking { s::CHECKING } else { s::CHECK })
                .disabled(self.checking || !saved)
                .on_click(cx.listener(|this, _, window, cx| this.check(window, cx)))
                .into_any_element(),
            self.render_checks(provider, cx),
            &r,
        )
        .into_any_element()
    }

    fn render_checks(&self, provider: &Provider, cx: &mut Context<Self>) -> Option<AnyElement> {
        let r = theme::roles(cx);
        let checks = match self.checks.as_ref()? {
            Err(error) => {
                return error_text(Some(error.clone()), cx).map(IntoElement::into_any_element);
            }
            Ok(checks) => checks,
        };
        let line = |check: &KeyCheck| match (check.models, &check.error) {
            (Some(count), _) => (true, s::check_ok(count)),
            (None, error) => (false, error.clone().unwrap_or_default()),
        };
        if let [check] = checks.as_slice() {
            let (ok, message) = line(check);
            return Some(
                label(text::METADATA, message)
                    .text_color(if ok {
                        r.feedback_success
                    } else {
                        r.feedback_error
                    })
                    .into_any_element(),
            );
        }
        Some(
            v_flex()
                .gap(space::S4)
                .children(checks.iter().filter_map(|check| {
                    let key = provider.keys.iter().find(|k| k.id == check.key)?.clone();
                    let (ok, message) = line(check);
                    let name = key.label.clone().unwrap_or_else(|| key.masked.clone());
                    let target = provider.id.clone();
                    Some(
                        h_flex()
                            .gap(space::S8)
                            .child(
                                label(
                                    text::METADATA,
                                    format!("{} {name} — {message}", if ok { "✓" } else { "✕" }),
                                )
                                .flex_1()
                                .text_color(if ok {
                                    r.feedback_success
                                } else {
                                    r.feedback_error
                                }),
                            )
                            .child(
                                Switch::new(format!("check-key-{}", key.id))
                                    .small()
                                    .checked(key.enabled)
                                    .color(r.feedback_success)
                                    .on_click(cx.listener(move |this, value: &bool, _, cx| {
                                        let (id, key, enabled) =
                                            (target.clone(), key.clone(), *value);
                                        write(this, cx, move |service| {
                                            service.update_key(
                                                &id,
                                                &key.id,
                                                thinkrail_core::ProviderKeyUpdate {
                                                    label: key.label,
                                                    key: None,
                                                    enabled,
                                                },
                                            )
                                        });
                                    })),
                            ),
                    )
                }))
                .into_any_element(),
        )
    }

    fn toggle_reveal(&mut self, key: thinkrail_core::ProviderKeyId, cx: &mut Context<Self>) {
        self.key_generation = self.key_generation.wrapping_add(1);
        let generation = self.key_generation;
        if self.revealed.take().is_some() {
            cx.notify();
            return;
        }
        let service = self.service(cx);
        let selected = self.selected.clone();
        run(
            cx,
            move || {
                service
                    .reveal_key(&key)
                    .map(|secret| secret.expose().to_owned())
            },
            move |this, result, cx| match result {
                Ok(value) if this.selected == selected && this.key_generation == generation => {
                    this.revealed = Some(value);
                    cx.notify();
                }
                Ok(_) => {}
                Err(error) => this.report(&error, cx),
            },
        );
    }

    /// The pane's only key input: shown while the provider has no key; Enter / blur adds it.
    pub(in crate::panels::settings::providers) fn add_first_key(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.key_input.read(cx).value().trim().to_owned();
        let Some(id) = self.selected.clone() else {
            return;
        };
        if self.key_saving
            || value.is_empty()
            || self.provider().is_some_and(|p| !p.keys.is_empty())
        {
            return;
        }
        self.key_saving = true;
        let generation = self.key_generation;
        let target = id.clone();
        let service = self.service(cx);
        let handle = window.window_handle();
        run(
            cx,
            move || {
                service.add_key(
                    &id,
                    thinkrail_core::ProviderKeyInput {
                        label: None,
                        key: thinkrail_core::Secret::new(value),
                        enabled: true,
                    },
                )
            },
            move |this, result, cx| {
                this.key_saving = false;
                let check_after_save = std::mem::take(&mut this.check_after_key_save);
                match result {
                    Ok(provider) => {
                        if this.selected.as_ref() == Some(&target)
                            && this.key_generation == generation
                        {
                            let input = this.key_input.clone();
                            if handle
                                .update(cx, |_, window, cx| {
                                    input.update(cx, |input, cx| input.set_value("", window, cx))
                                })
                                .is_err()
                            {
                                tracing::debug!("providers.key.window_closed");
                            }
                        }
                        this.stored(provider, cx);
                        if check_after_save && this.selected.as_ref() == Some(&target) {
                            this.check_saved(cx);
                        }
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    /// Persist pending input before probing, including a save already started by blur.
    pub(in crate::panels::settings::providers) fn check(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        if self.checking {
            return;
        }
        if self.key_saving {
            self.check_after_key_save = true;
            return;
        }
        if self.provider().is_some_and(|p| p.keys.is_empty())
            && !self.key_input.read(cx).value().trim().is_empty()
        {
            self.check_after_key_save = true;
            self.add_first_key(window, cx);
            return;
        }
        self.check_saved(cx);
    }

    fn check_saved(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.provider().cloned() else {
            return;
        };
        let Some(endpoint) = provider
            .input
            .endpoints
            .iter()
            .find(|e| e.protocol == self.line)
            .cloned()
        else {
            return;
        };
        self.checking = true;
        self.checks = None;
        let probe = self.state.read(cx).services.probe.clone();
        let line = self.line;
        let selected = self.selected.clone();
        let generation = self.check_generation;
        run(
            cx,
            move || {
                let checks = probe.check_keys(&provider.id, line)?;
                if !checks.is_empty() {
                    return Ok(checks);
                }
                let target = ProbeTarget {
                    endpoint,
                    headers: provider.input.headers.clone(),
                    key: None,
                };
                let started = std::time::Instant::now();
                let result = probe.list_models(&target);
                let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let key = thinkrail_core::ProviderKeyId::from("");
                Ok(vec![match result {
                    Ok(models) => KeyCheck {
                        key,
                        models: Some(models.len()),
                        elapsed_ms,
                        error: None,
                    },
                    Err(error) => KeyCheck {
                        key,
                        models: None,
                        elapsed_ms,
                        error: Some(error.to_string()),
                    },
                }])
            },
            move |this, result, cx| {
                if this.selected != selected
                    || this.line != line
                    || this.check_generation != generation
                {
                    return;
                }
                this.checking = false;
                this.checks = Some(result.map_err(|error| error.to_string()));
                cx.notify();
            },
        );
    }
}
