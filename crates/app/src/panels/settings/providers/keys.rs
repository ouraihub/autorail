//! API key management dialog and presentation.

use gpui_kit::{AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window, base::{h_flex, v_flex}, component::{Disableable as _, WindowExt as _, button::{Button, ButtonVariants as _}, dialog::{DialogClose, DialogFooter}, input::{Input, InputState}, switch::Switch}, px};
use thinkrail_core::{Provider, ProviderKeyId, ProviderKeyInput, ProviderKeySummary, ProviderKeyUpdate, Secret};
use thinkrail_ui::{ThinkrailTheme, components::{IconButton, label}, metrics::{radius, space, text}};
use crate::{state::run, strings::settings_providers as s};

use super::ProvidersPage;

pub(crate) struct KeysDialog {
    page: Entity<ProvidersPage>,
    provider_id: thinkrail_core::ProviderId,
    label: Entity<InputState>,
    value: Entity<InputState>,
    editing_key: Option<ProviderKeyId>,
    _subscription: Subscription,
}

impl KeysDialog {
    pub(crate) fn new(page: Entity<ProvidersPage>, provider_id: thinkrail_core::ProviderId, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&page, |_, _, cx| cx.notify());
        Self { page, provider_id, label: cx.new(|cx| InputState::new(window, cx).placeholder(s::KEY_LABEL)), value: cx.new(|cx| InputState::new(window, cx).placeholder(s::KEY_VALUE)), editing_key: None, _subscription: subscription }
    }
    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_key = None;
        self.label.update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.value.update(cx, |input, cx| input.set_value(String::new(), window, cx));
    }
}

impl Render for KeysDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let roles = ThinkrailTheme::global(cx).roles;
        let Some(provider) = self.page.read(cx).providers.iter().find(|provider| provider.id == self.provider_id).cloned() else { return v_flex().child(label(text::METADATA, s::KEYS_EMPTY)).into_any_element(); };
        let page = self.page.clone();
        let provider_for_add = provider.clone();
        let editing_key = self.editing_key.clone();
        v_flex().w(px(460.)).gap(space::S8)
            .child(label(text::METADATA, s::KEYS_DESCRIPTION).text_color(roles.text_subtle))
            .children(provider.keys.iter().map(|key| {
                let key_id = key.id.clone();
                let key_label = key.label.clone();
                let key_for_toggle = key.clone();
                let key_for_delete = key.clone();
                let provider_for_toggle = provider.clone();
                let provider_for_delete = provider.clone();
                let page_for_toggle = page.clone();
                let page_for_delete = page.clone();
                h_flex().gap(space::S4).p(space::S6).rounded(radius::SM).border_1().border_color(roles.border_muted)
                    .child(label(text::UI, key.label.clone().unwrap_or_else(|| s::API_KEY_DEFAULT.to_owned())).flex_1())
                    .child(label(text::METADATA, key.masked.clone()).text_color(roles.text_subtle))
                    .child(IconButton::new(format!("keys-copy-{}", key.id), gpui_kit::assets::IconName::Copy).tooltip(s::COPY).on_click(cx.listener({ let key_id = key_id.clone(); move |this, _, _, cx| this.page.update(cx, |page, cx| page.key_copy(key_id.clone(), cx)) })))
                    .child(IconButton::new(format!("keys-edit-{}", key.id), gpui_kit::assets::IconName::Pencil).tooltip(s::EDIT).on_click(cx.listener(move |this, _, window, cx| { this.editing_key = Some(key_id.clone()); this.label.update(cx, |input, cx| input.set_value(key_label.clone().unwrap_or_default(), window, cx)); this.value.update(cx, |input, cx| input.set_value(String::new(), window, cx)); cx.notify(); })))
                    .child(Switch::new(format!("keys-enabled-{}", key.id)).checked(key.enabled).color(roles.feedback_success).on_click(cx.listener(move |_, enabled: &bool, _, cx| { page_for_toggle.update(cx, |page, cx| page.key_toggle(provider_for_toggle.clone(), key_for_toggle.clone(), *enabled, cx)); })))
                    .child(IconButton::new(format!("keys-delete-{}", key.id), gpui_kit::assets::IconName::Trash).tooltip(s::DELETE).on_click(cx.listener(move |_, _, _, cx| { page_for_delete.update(cx, |page, cx| page.key_delete(provider_for_delete.clone(), key_for_delete.clone(), cx)); })))
            }))
            .child(h_flex().gap(space::S4).child(Input::new(&self.label)).child(Input::new(&self.value).mask_toggle()).child(Button::new("keys-save").primary().label(if editing_key.is_some() { s::SAVE } else { s::ADD_KEY }).on_click(cx.listener(move |this, _, window, cx| { let label = this.label.read(cx).value().trim().to_owned(); let value = this.value.read(cx).value().trim().to_owned(); if let Some(key_id) = this.editing_key.clone() { this.page.update(cx, |page, cx| page.update_key_value(provider_for_add.clone(), key_id, label, value, cx)); } else { this.page.update(cx, |page, cx| page.add_key_value(provider_for_add.clone(), label, value, cx)); } this.clear(window, cx); }))))
            .into_any_element()
    }
}

impl ProvidersPage {
    pub(super) fn key_copy(&mut self, key_id: ProviderKeyId, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.providers.clone();
        run(cx, move || service.reveal_key(&key_id).map(|secret| secret.expose().to_owned()), |this, result, cx| match result {
            Ok(value) => cx.write_to_clipboard(ClipboardItem::new_string(value)),
            Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
        });
    }

    pub(super) fn key_delete(&mut self, provider: Provider, key: ProviderKeySummary, cx: &mut Context<Self>) {
        let target = super::DeleteTarget::Key(key.id.clone());
        if self.confirm_delete.as_ref() != Some(&target) { self.confirm_delete = Some(target); cx.notify(); return; }
        self.confirm_delete = None;
        let service = self.state.read(cx).services.providers.clone();
        run(cx, move || service.delete_key(&provider.id, &key.id), |this, result, cx| match result {
            Ok(updated) => { this.replace_provider(updated); this.emit_changed(cx); }
            Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
        });
    }

    pub(super) fn key_toggle(&mut self, provider: Provider, key: ProviderKeySummary, enabled: bool, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.providers.clone();
        let update = ProviderKeyUpdate { label: key.label.clone(), key: None, enabled };
        run(cx, move || service.update_key(&provider.id, &key.id, update), |this, result, cx| match result {
            Ok(updated) => { this.replace_provider(updated); this.emit_changed(cx); }
            Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
        });
    }

    pub(super) fn add_key_value(&mut self, provider: Provider, label: String, value: String, cx: &mut Context<Self>) {
        let value = value.trim().to_owned();
        if value.is_empty() { self.error = Some(s::KEY_EMPTY.to_owned()); cx.notify(); return; }
        let service = self.state.read(cx).services.providers.clone();
        run(cx, move || service.add_key(&provider.id, ProviderKeyInput { label: (!label.trim().is_empty()).then(|| label.trim().to_owned()), key: Secret::new(value), enabled: true }), |this, result, cx| match result {
            Ok(updated) => { this.replace_provider(updated); this.emit_changed(cx); }
            Err(error) => { this.error = Some(error.to_string()); this.state.update(cx, |state, cx| state.report(&error, cx)); cx.notify(); }
        });
    }

    pub(super) fn update_key_value(&mut self, provider: Provider, key_id: ProviderKeyId, label: String, value: String, cx: &mut Context<Self>) {
        let enabled = provider.keys.iter().find(|key| key.id == key_id).is_some_and(|key| key.enabled);
        let update = ProviderKeyUpdate { label: (!label.trim().is_empty()).then(|| label.trim().to_owned()), key: (!value.trim().is_empty()).then(|| Secret::new(value.trim())), enabled };
        let service = self.state.read(cx).services.providers.clone();
        run(cx, move || service.update_key(&provider.id, &key_id, update), |this, result, cx| match result {
            Ok(updated) => { this.replace_provider(updated); this.emit_changed(cx); }
            Err(error) => { this.error = Some(error.to_string()); this.state.update(cx, |state, cx| state.report(&error, cx)); cx.notify(); }
        });
    }

    pub(super) fn open_keys_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.editing.as_ref() else { return; };
        let Some(provider_id) = editor.id.clone() else { return; };
        let page = cx.entity();
        let keys = cx.new(|cx| KeysDialog::new(page, provider_id, window, cx));
        window.open_dialog(cx, move |dialog, _, _| dialog.title(s::KEYS_MANAGE).child(keys.clone()).footer(DialogFooter::new().child(DialogClose::new().trigger(|button| button.label(s::CLOSE)))));
    }

    pub(super) fn check_keys(&mut self, provider: Provider, model_id: String, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editing else { return; };
        if editor.check_all { return; }
        editor.check_all = true;
        let probe = self.state.read(cx).services.probe.clone();
        run(cx, move || probe.check_keys(&provider.id, &model_id), |this, result, cx| {
            if let Some(editor) = &mut this.editing { editor.check_all = false; }
            if let Ok(checks) = result && let Some(editor) = &mut this.editing { editor.key_checks = checks; }
            cx.notify();
        });
    }
}

impl ProvidersPage {
    pub(super) fn render_keys(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let roles = ThinkrailTheme::global(cx).roles;
        let Some(editor) = self.editing.as_ref() else { return v_flex().into_any_element(); };
        if editor.id.is_some() {
            let primary = editor.keys.iter().find(|key| key.enabled).or_else(|| editor.keys.first());
            let summary = primary.map(|key| format!("{} · {}", key.label.clone().unwrap_or_else(|| s::API_KEY_DEFAULT.to_owned()), key.masked)).unwrap_or_else(|| s::KEYS_EMPTY.to_owned());
            return v_flex().gap(space::S4).child(label(text::TITLE_COMPACT, s::KEYS).text_color(roles.text_muted)).child(h_flex().gap(space::S4).child(label(text::UI, summary).flex_1().truncate()).child(label(text::METADATA, s::key_summary(editor.keys.iter().filter(|key| key.enabled).count(), editor.keys.len())).text_color(roles.text_subtle)).child(Button::new("provider-keys-manage").ghost().label(s::KEYS_MANAGE).on_click(cx.listener(|this, _, window, cx| this.open_keys_dialog(window, cx)))).child(Button::new("provider-keys-check-all").ghost().label(if editor.check_all { s::PROBING } else { s::CHECK_ALL }).disabled(editor.check_all || editor.models.iter().all(|model| !model.enabled)).on_click(cx.listener(|this, _, _, cx| { let Some(provider) = this.providers.iter().find(|provider| this.editing.as_ref().and_then(|editor| editor.id.as_ref()) == Some(&provider.id)).cloned() else { return; }; let model_id = this.editing.as_ref().and_then(|editor| editor.models.iter().find(|model| model.enabled).map(|model| model.id.clone())); if let Some(model_id) = model_id { this.check_keys(provider, model_id, cx); } })))).into_any_element();
        }
        v_flex().gap(space::S4).child(label(text::TITLE_COMPACT, s::KEYS).text_color(roles.text_muted)).child(label(text::METADATA, s::KEYS_EMPTY).text_color(roles.text_subtle)).child(h_flex().gap(space::S4).child(Input::new(&editor.key_label)).child(Input::new(&editor.key_value).mask_toggle()).child(Button::new("provider-key-save").primary().label(s::ADD_KEY).disabled(editor.adding_key).on_click(cx.listener(|this, _, _, cx| this.save(cx))))).into_any_element()
    }
}
