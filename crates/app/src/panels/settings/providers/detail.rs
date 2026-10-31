//! 选中服务商的详情：标题行、API 密钥与检测、密钥使用方式、API 地址与线路、模型。

use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
        switch::Switch,
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{KeyCheck, ProbeTarget, Provider, provider_preset};
use thinkrail_routing::models_url;
use thinkrail_ui::{ThinkrailTheme, components::{IconButton, label}, metrics::{radius, space, text}};

use super::{ProvidersPage, error_text, write};
use crate::{state::run, strings::settings_providers as s};

impl ProvidersPage {
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let (Some(provider), Some(draft)) = (self.provider().cloned(), self.draft.as_ref()) else {
            let message = if self.loaded { (s::EMPTY, s::EMPTY_HINT) } else { (s::LOADING, "") };
            return v_flex().flex_1().items_center().py(space::S40).gap(space::S4)
                .child(label(text::TITLE_SECTION, message.0))
                .child(label(text::METADATA, message.1).text_color(r.text_subtle))
                .into_any_element();
        };
        let enabled = draft.enabled;
        let failed = self.failed.clone().map(|error| {
            h_flex().gap(space::S8).child(label(text::METADATA, format!("{} {error}", s::SAVE_FAILED)).text_color(r.feedback_error))
                .child(Button::new("provider-retry").link().label(s::RETRY).on_click(cx.listener(|this, _, _, cx| this.commit(cx))))
        });
        v_flex().flex_1().min_w(px(0.)).gap(space::S16)
            .child(h_flex().gap(space::S8)
                .child(label(text::HEADING_SM, draft.name.clone()).truncate())
                .child(IconButton::new("provider-settings", IconName::Settings2).tooltip(s::SETTINGS)
                    .on_click(cx.listener({ let p = provider.clone(); move |this, _, window, cx| this.open_name_dialog(Some(p.clone()), window, cx) })))
                .child(gpui_kit::div().flex_1())
                .child(Switch::new("provider-enabled").checked(enabled).color(r.feedback_success)
                    .on_click(cx.listener(|this, value: &bool, _, cx| { let value = *value; this.edit(cx, |d| d.enabled = value) }))))
            .children(failed)
            .child(self.render_key(&provider, cx))
            .child(v_flex().gap(space::S4)
                .child(label(text::TITLE_COMPACT, s::KEY_STRATEGY).text_color(r.text_muted))
                .child(label(text::UI, s::KEY_STRATEGY_ROUND_ROBIN))
                .child(label(text::METADATA, s::KEY_STRATEGY_ROUND_ROBIN_DESC).text_color(r.text_subtle)))
            .child(self.render_url(cx))
            .child(self.render_models(cx))
            .into_any_element()
    }

    fn render_key(&self, provider: &Provider, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let key_url = provider.preset_id.as_deref().and_then(provider_preset).map(|p| p.api_key_url);
        let enabled: Vec<_> = provider.keys.iter().filter(|k| k.enabled).collect();
        let shown = enabled.first().copied().or(provider.keys.first());
        let value = match shown {
            None => Input::new(&self.key_input).flex_1().into_any_element(),
            Some(key) => {
                let (id, copy, edit) = (key.id.clone(), key.id.clone(), key.clone());
                let text_value = self.revealed.clone().unwrap_or_else(|| key.masked.clone());
                h_flex().flex_1().min_w(px(0.)).h(px(32.)).px(space::S8).gap(space::S8).rounded(radius::MD).border_1()
                    .border_color(if self.key_hover { r.control_border_active } else { r.border_default })
                    .children(key.label.clone().map(|l| label(text::UI, l).text_color(r.text_muted)))
                    .child(label(text::CODE_TEXT, text_value).truncate())
                    .when(enabled.len() > 1, |d| d.child(label(text::METADATA, s::more_keys(enabled.len() - 1)).text_color(r.text_subtle)))
                    .child(gpui_kit::div().flex_1())
                    // Copy / edit appear while the pointer is on the eye; the cluster keeps them
                    // open as the pointer moves left onto them.
                    .child(h_flex().id("provider-key-actions").gap(space::S2)
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| { this.key_hover = *hovered; cx.notify(); }))
                        .when(self.key_hover, |d| d
                            .child(IconButton::new("provider-key-copy", IconName::Copy).tooltip(s::KEY_COPY)
                                .on_click(cx.listener(move |this, _, _, cx| this.copy_key(copy.clone(), cx))))
                            .child(IconButton::new("provider-key-edit", IconName::Pencil).tooltip(s::KEY_EDIT)
                                .on_click(cx.listener(move |this, _, window, cx| this.open_keys_dialog(Some(edit.clone()), window, cx)))))
                        .child(IconButton::new("provider-key-reveal", if self.revealed.is_some() { IconName::EyeOff } else { IconName::Eye })
                            .tooltip(if self.revealed.is_some() { s::KEY_HIDE } else { s::KEY_SHOW })
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_reveal(id.clone(), cx)))))
                    .into_any_element()
            }
        };
        let saved = !self.dirty && !self.saving && self.failed.is_none();
        v_flex().gap(space::S6)
            .child(h_flex().gap(space::S8).child(label(text::TITLE_COMPACT, s::API_KEY).text_color(r.text_muted))
                .children(key_url.map(|url| Button::new("provider-get-key").link().xsmall().label(s::GET_KEY)
                    .on_click(move |_, _, cx| cx.open_url(url)))))
            .child(h_flex().gap(space::S8).child(value)
                .child(Button::new("provider-keys").outline().icon(IconName::KeyRound).tooltip(s::KEYS_MANAGE)
                    .on_click(cx.listener(|this, _, window, cx| this.open_keys_dialog(None, window, cx))))
                .child(Button::new("provider-check").outline().label(if self.checking { s::CHECKING } else { s::CHECK })
                    .disabled(self.checking || !saved).on_click(cx.listener(|this, _, _, cx| this.check(cx)))))
            .children(self.render_checks(provider, cx))
            .into_any_element()
    }

    fn render_checks(&self, provider: &Provider, cx: &mut Context<Self>) -> Option<AnyElement> {
        let r = ThinkrailTheme::global(cx).roles;
        let checks = match self.checks.as_ref()? {
            Err(error) => return error_text(Some(error.clone()), cx).map(IntoElement::into_any_element),
            Ok(checks) => checks,
        };
        let line = |check: &KeyCheck| match (check.models, &check.error) {
            (Some(count), _) => (true, s::check_ok(count)),
            (None, error) => (false, error.clone().unwrap_or_default()),
        };
        if let [check] = checks.as_slice() {
            let (ok, message) = line(check);
            return Some(label(text::METADATA, message).text_color(if ok { r.feedback_success } else { r.feedback_error }).into_any_element());
        }
        Some(v_flex().gap(space::S4).children(checks.iter().filter_map(|check| {
            let key = provider.keys.iter().find(|k| k.id == check.key)?.clone();
            let (ok, message) = line(check);
            let name = key.label.clone().unwrap_or_else(|| key.masked.clone());
            let target = provider.id.clone();
            Some(h_flex().gap(space::S8)
                .child(label(text::METADATA, format!("{} {name} — {message}", if ok { "✓" } else { "✕" }))
                    .flex_1().text_color(if ok { r.feedback_success } else { r.feedback_error }))
                .child(Switch::new(format!("check-key-{}", key.id)).small().checked(key.enabled).color(r.feedback_success)
                    .on_click(cx.listener(move |this, value: &bool, _, cx| {
                        let (id, key, enabled) = (target.clone(), key.clone(), *value);
                        write(this, cx, move |service| service.update_key(&id, &key.id, thinkrail_core::ProviderKeyUpdate { label: key.label, key: None, enabled }));
                    }))))
        })).into_any_element())
    }
}

impl ProvidersPage {
    fn render_url(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let Some(draft) = self.draft.as_ref() else { return gpui_kit::div().into_any_element() };
        let can_add = !thinkrail_core::free_protocols(draft).is_empty();
        let lines = draft.endpoints.clone();
        let current = self.line;
        let page = cx.entity();
        let picker = (lines.len() > 1).then(|| {
            Button::new("provider-line").outline().w_full().label(format!("{} · {}", s::protocol_short(current), self.line_url()))
                .icon(IconName::ChevronDown).dropdown_menu(move |menu, _, _| {
                    lines.iter().fold(menu, |menu, line| {
                        let (protocol, page) = (line.protocol, page.clone());
                        menu.item(PopupMenuItem::new(format!("{} · {}", s::protocol_short(protocol), line.base_url))
                            .checked(protocol == current)
                            .on_click(move |_, _, cx| page.update(cx, |this, cx| { this.line = protocol; this.sync_inputs = true; this.models.reset(); cx.notify(); })))
                    })
                })
        });
        let url = self.line_url();
        let hint = if self.url_error {
            error_text(Some(s::URL_INVALID.into()), cx).map(IntoElement::into_any_element)
        } else {
            (!url.is_empty()).then(|| label(text::CODE_SMALL, s::probe_get(&models_url(&url, current))).text_color(r.text_subtle).truncate().into_any_element())
        };
        v_flex().gap(space::S6)
            .child(h_flex().gap(space::S8).child(label(text::TITLE_COMPACT, s::BASE_URL).text_color(r.text_muted))
                .when(can_add, |d| d.child(Button::new("provider-add-line").link().xsmall().label(s::ADD_LINE)
                    .on_click(cx.listener(|this, _, window, cx| this.open_line_dialog(None, window, cx))))))
            .children(picker)
            .child(h_flex().gap(space::S8).child(Input::new(&self.url).flex_1())
                .child(Button::new("provider-line-settings").outline().icon(IconName::Settings2).tooltip(s::LINE_SETTINGS_TIP)
                    .on_click(cx.listener(move |this, _, window, cx| this.open_line_dialog(Some(current), window, cx)))))
            .children(hint)
            .into_any_element()
    }

    fn toggle_reveal(&mut self, key: thinkrail_core::ProviderKeyId, cx: &mut Context<Self>) {
        if self.revealed.take().is_some() {
            cx.notify();
            return;
        }
        let service = self.service(cx);
        let selected = self.selected.clone();
        run(cx, move || service.reveal_key(&key).map(|secret| secret.expose().to_owned()), move |this, result, cx| match result {
            Ok(value) if this.selected == selected => { this.revealed = Some(value); cx.notify(); }
            Ok(_) => {}
            Err(error) => this.report(&error, cx),
        });
    }

    /// The pane's only key input: shown while the provider has no key; Enter / blur adds it.
    pub(super) fn add_first_key(&mut self, window: &mut gpui_kit::Window, cx: &mut Context<Self>) {
        let value = self.key_input.read(cx).value().trim().to_owned();
        let Some(id) = self.selected.clone() else { return };
        if value.is_empty() || self.provider().is_some_and(|p| !p.keys.is_empty()) {
            return;
        }
        self.key_input.update(cx, |input, cx| input.set_value("", window, cx));
        let key = thinkrail_core::ProviderKeyInput { label: None, key: thinkrail_core::Secret::new(value), enabled: true };
        write(self, cx, move |service| service.add_key(&id, key));
    }

    /// "检测": lists the current line's models once per enabled key; with no key, once without one.
    fn check(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.provider().cloned() else { return };
        let Some(endpoint) = provider.input.endpoints.iter().find(|e| e.protocol == self.line).cloned() else { return };
        self.checking = true;
        self.checks = None;
        let probe = self.state.read(cx).services.probe.clone();
        let line = self.line;
        let selected = self.selected.clone();
        run(cx, move || {
            let checks = probe.check_keys(&provider.id, line)?;
            if !checks.is_empty() {
                return Ok(checks);
            }
            let target = ProbeTarget { endpoint, headers: provider.input.headers.clone(), key: None };
            let started = std::time::Instant::now();
            let result = probe.list_models(&target);
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let key = thinkrail_core::ProviderKeyId::from("");
            Ok(vec![match result {
                Ok(models) => KeyCheck { key, models: Some(models.len()), elapsed_ms, error: None },
                Err(error) => KeyCheck { key, models: None, elapsed_ms, error: Some(error.to_string()) },
            }])
        }, move |this, result, cx| {
            if this.selected != selected {
                return;
            }
            this.checking = false;
            this.checks = Some(result.map_err(|error| error.to_string()));
            cx.notify();
        });
    }
}
