//! Provider, endpoint, model and API key settings (agent-settings 04 §7.6–§7.7).

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement,
    InteractiveElement as _, ParentElement as _, Render, Styled as _, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    div,
    component::{
        Disableable as _, IndexPath, button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState}, scroll::ScrollableElement as _,
        select::{Select, SelectState},
        switch::Switch,
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{
    Access, Endpoint, Protocol, Provider, ProviderInput,
    ProviderKeyInput, ProviderKeySummary, Secret,
};
use thinkrail_routing::preview;
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, Segment, label},
    metrics::{radius, size, space, text},
};
use crate::{state::{AppEvent, AppState, run}, strings::settings_providers as s};

mod draft;
mod models;
use models::ModelFields;
mod keys;
#[cfg(test)]
mod tests;
use draft::{parse_headers, parse_url, protocol_index, protocol_options, set_input, PROTOCOLS, ProtocolOptions};

// ProviderKeySummary already contains the store's canonical masked value; plaintext never
// enters this page except for the short-lived reveal/copy operation.

struct HeaderFields {
    name: Entity<InputState>,
    value: Entity<InputState>,
}


struct ProviderEditor {
    id: Option<thinkrail_core::ProviderId>,
    revision: u64,
    name: Entity<InputState>,
    openai: Entity<InputState>,
    anthropic: Entity<InputState>,
    responses: Entity<InputState>,
    default_protocol: Entity<SelectState<ProtocolOptions>>,
    headers: Vec<HeaderFields>,
    model_id: Entity<InputState>,
    key_label: Entity<InputState>,
    key_value: Entity<InputState>,
    editing_key: Option<ProviderKeyId>,
    keys: Vec<ProviderKeySummary>,
    models: Vec<ModelFields>,
    discovered: Vec<thinkrail_core::DiscoveredModel>,
    access: Access,
    enabled: bool,
    origin_locked: bool,
    saving: bool,
    probing: bool,
    adding_key: bool,
    key_checks: Vec<thinkrail_core::KeyCheck>,
    check_all: bool,
}

use thinkrail_core::ProviderKeyId;

struct SaveInput {
    provider: ProviderInput,
    key_label: Option<String>,
    key: Option<Secret>,
}

#[derive(PartialEq, Eq)]
enum DeleteTarget {
    Provider(thinkrail_core::ProviderId),
    Key(ProviderKeyId),
}


pub struct ProvidersPage {
    state: Entity<AppState>,
    providers: Vec<Provider>,
    search: Entity<InputState>,
    editing: Option<ProviderEditor>,
    confirm_delete: Option<DeleteTarget>,
    revealed_key: Option<(ProviderKeyId, String)>,
    sensitive_generation: u64,
    loading: bool,
    error: Option<String>,
    operation_generation: u64,
    _subscriptions: Vec<Subscription>,
}

impl ProvidersPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(s::SEARCH));
        let subscription = cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
            if matches!(event, AppEvent::Routing) && this.editing.is_none() { this.reload(cx); }
        });
        let search_subscription = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) { cx.notify(); }
        });
        let mut page = Self {
            state, providers: Vec::new(), search, editing: None, confirm_delete: None,
            revealed_key: None, sensitive_generation: 0, loading: false, error: None,
            operation_generation: 0, _subscriptions: vec![subscription, search_subscription],
        };
        page.reload(cx);
        page
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.editing.is_some() {
            return;
        }
        self.loading = true;
        let service = self.state.read(cx).services.providers.clone();
        run(cx, move || service.providers(), |this, result, cx| {
            this.loading = false;
            match result {
                Ok(providers) => { this.providers = providers; this.error = None; }
                Err(error) => {
                    this.error = Some(error.to_string());
                    this.state.update(cx, |state, cx| state.report(&error, cx));
                }
            }
            cx.notify();
        });
    }

    pub(crate) fn clear_sensitive(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.sensitive_generation = self.sensitive_generation.wrapping_add(1);
        self.revealed_key = None;
        if let Some(editor) = &mut self.editing {
            editor.editing_key = None;
            editor.key_checks.clear();
        }
    }

    fn begin_edit(&mut self, provider: Option<Provider>, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_edit(window, cx);
        self.confirm_delete = None;
        let (id, revision, name, urls, default_protocol, access, enabled, models, keys, locked) = provider.as_ref().map_or_else(
            || (None, 0, String::new(), Vec::new(), Protocol::OpenaiChat, Access::Direct, true, Vec::new(), Vec::new(), false),
            |provider| (
                Some(provider.id.clone()), provider.revision, provider.input.name.clone(), provider.input.endpoints.clone(),
                provider.input.default_protocol, provider.input.access, provider.input.enabled, provider.input.models.clone(),
                provider.keys.clone(), true,
            ),
        );
        let default_protocol_select = cx.new(|cx| SelectState::new(protocol_options(), Some(IndexPath::default().row(protocol_index(default_protocol))), window, cx));
        let model_fields = models.iter().map(|model| ModelFields::from_model(model, window, cx)).collect();
        let mut input = |value: String, placeholder: &'static str| {
            let field = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            set_input(&field, value, window, cx);
            field
        };
        let endpoint = |protocol: Protocol| urls.iter().find(|item| item.protocol == protocol).map(|item| item.base_url.clone()).unwrap_or_default();
        let headers = provider.as_ref().map_or_else(Vec::new, |provider| provider.input.headers.iter().map(|header| HeaderFields { name: input(header.name.clone(), s::HEADER_NAME), value: input(header.value.clone(), s::HEADER_VALUE) }).collect());
        self.editing = Some(ProviderEditor {
            id, revision, name: input(name, s::NAME), openai: input(endpoint(Protocol::OpenaiChat), ""),
            anthropic: input(endpoint(Protocol::AnthropicMessages), ""), responses: input(endpoint(Protocol::OpenaiResponses), ""),
            default_protocol: default_protocol_select,
            headers, model_id: input(String::new(), s::ADD_MODEL), key_label: input(String::new(), s::KEY_LABEL), key_value: input(String::new(), s::KEY_VALUE),
            editing_key: None, keys, models: model_fields, discovered: Vec::new(),
            access, enabled, origin_locked: locked, saving: false, probing: false, adding_key: false,
            key_checks: Vec::new(), check_all: false,
        });
        let _ = &self._subscriptions;
        self.error = None;
        cx.notify();
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.operation_generation = self.operation_generation.wrapping_add(1);
        self.editing = None;
        self.clear_sensitive(window, cx);
        cx.notify();
    }
    fn active_protocol(&self, cx: &Context<Self>) -> Protocol {
        self.editing.as_ref().and_then(|editor| editor.default_protocol.read(cx).selected_value().copied()).unwrap_or(Protocol::OpenaiChat)
    }
    fn input(&self, field: &Entity<InputState>, cx: &Context<Self>) -> String { field.read(cx).value().to_string() }

    fn endpoint(&self, protocol: Protocol, cx: &Context<Self>) -> Result<Endpoint, &'static str> {
        let editor = self.editing.as_ref().ok_or(s::URL_REQUIRED)?;
        let field = match protocol {
            Protocol::OpenaiChat => &editor.openai,
            Protocol::OpenaiResponses => &editor.responses,
            Protocol::AnthropicMessages => &editor.anthropic,
        };
        let raw = self.input(field, cx);
        let base_url = parse_url(&raw)?;
        if base_url.is_empty() { return Err(s::URL_REQUIRED); }
        Ok(Endpoint { protocol, base_url })
    }

    fn save_input(&self, cx: &Context<Self>) -> Result<SaveInput, &'static str> {
        let editor = self.editing.as_ref().ok_or(s::NAME_REQUIRED)?;
        let name = self.input(&editor.name, cx).trim().to_owned();
        if name.is_empty() { return Err(s::NAME_REQUIRED); }
        let mut endpoints = Vec::new();
        for protocol in PROTOCOLS {
            let field = match protocol {
                Protocol::OpenaiChat => &editor.openai,
                Protocol::OpenaiResponses => &editor.responses,
                Protocol::AnthropicMessages => &editor.anthropic,
            };
            let raw = self.input(field, cx);
            if !raw.trim().is_empty() { endpoints.push(Endpoint { protocol, base_url: parse_url(&raw)? }); }
        }
        if endpoints.is_empty() { return Err(s::URL_REQUIRED); }
        let headers = parse_headers(&editor.headers, cx)?;
        let mut models = Vec::new();
        for model in &editor.models { models.push(model.to_model(cx)?); }
        if !models.iter().any(|model| model.enabled) { return Err(s::ENABLED_MODELS_REQUIRED); }
        let key = self.input(&editor.key_value, cx).trim().to_owned();
        Ok(SaveInput {
            provider: ProviderInput { name, endpoints, default_protocol: editor.default_protocol.read(cx).selected_value().copied().unwrap_or(Protocol::OpenaiChat), headers, access: editor.access, enabled: editor.enabled, models },
            key_label: (!self.input(&editor.key_label, cx).trim().is_empty()).then(|| self.input(&editor.key_label, cx).trim().to_owned()),
            key: (!key.is_empty()).then(|| Secret::new(key)),
        })
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.editing else { return; };
        if editor.saving { return; }
        let input = match self.save_input(cx) {
            Ok(input) => input,
            Err(error) => { self.error = Some(error.to_owned()); cx.notify(); return; }
        };
        let id = editor.id.clone();
        let revision = editor.revision;
        let service = self.state.read(cx).services.providers.clone();
        let generation = self.operation_generation.wrapping_add(1);
        self.operation_generation = generation;
        if let Some(editor) = &mut self.editing { editor.saving = true; }
        run(cx, move || match id {
            Some(id) => service.update(&id, revision, input.provider).map(|provider| (provider, input.key_label, input.key)),
            None => service.create(input.provider).map(|provider| (provider, input.key_label, input.key)),
        }, move |this, result, cx| {
            if generation != this.operation_generation { return; }
            match result {
                Ok((provider, label, key)) => {
                    if key.is_some() {
                        this.finish_provider_save(provider, label, key, cx);
                    } else {
                        this.finish_editor(provider, cx);
                    }
                }
                Err(error) => {
                    if let Some(editor) = &mut this.editing { editor.saving = false; }
                    this.error = Some(error.to_string());
                    this.state.update(cx, |state, cx| state.report(&error, cx));
                    cx.notify();
                }
            }
        });
    }

    fn finish_provider_save(&mut self, provider: Provider, label: Option<String>, key: Option<Secret>, cx: &mut Context<Self>) {
        let Some(key) = key else { self.finish_editor(provider, cx); return; };
        let service = self.state.read(cx).services.providers.clone();
        let id = provider.id.clone();
        run(cx, move || service.add_key(&id, ProviderKeyInput { label, key, enabled: true }), move |this, result, cx| {
            match result {
                Ok(updated) => this.finish_editor(updated, cx),
                Err(error) => {
                    if let Some(editor) = &mut this.editing {
                        editor.id = Some(provider.id.clone());
                        editor.revision = provider.revision;
                        editor.keys = provider.keys.clone();
                        editor.saving = false;
                    }
                    this.error = Some(error.to_string());
                    this.state.update(cx, |state, cx| state.report(&error, cx));
                    cx.notify();
                }
            }
        });
    }

    fn finish_editor(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.providers.retain(|item| item.id != provider.id);
        self.providers.push(provider);
        self.providers.sort_by(|a, b| a.input.name.cmp(&b.input.name));
        self.editing = None;
        self.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
        self.revealed_key = None;
        cx.notify();
    }

    fn toggle_provider(&mut self, provider: Provider, enabled: bool, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.providers.clone();
        let mut input = provider.input.clone();
        input.enabled = enabled;
        run(cx, move || service.update(&provider.id, provider.revision, input), |this, result, cx| match result {
            Ok(updated) => { this.replace_provider(updated); this.emit_changed(cx); }
            Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
        });
    }

    fn delete_provider(&mut self, provider: Provider, cx: &mut Context<Self>) {
        let target = DeleteTarget::Provider(provider.id.clone());
        if self.confirm_delete.as_ref() != Some(&target) {
            self.confirm_delete = Some(target);
            cx.notify();
            return;
        }
        self.confirm_delete = None;
        let service = self.state.read(cx).services.providers.clone();
        let deleted_id = provider.id.clone();
        run(cx, move || service.delete(&provider.id, provider.revision), move |this, result, cx| match result {
            Ok(()) => { this.providers.retain(|item| item.id != deleted_id); this.emit_changed(cx); }
            Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
        });
    }

    fn replace_provider(&mut self, provider: Provider) {
        if let Some(item) = self.providers.iter_mut().find(|item| item.id == provider.id) { *item = provider.clone(); }
        if let Some(editor) = &mut self.editing && editor.id.as_ref() == Some(&provider.id) {
            editor.revision = provider.revision;
            editor.keys = provider.keys.clone();
        }
    }

    fn emit_changed(&self, cx: &mut Context<Self>) {
        self.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
        cx.notify();
    }

    fn add_header(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.editing {
            editor.headers.push(HeaderFields {
                name: cx.new(|cx| InputState::new(window, cx).placeholder(s::HEADER_NAME)),
                value: cx.new(|cx| InputState::new(window, cx).placeholder(s::HEADER_VALUE)),
            });
            cx.notify();
        }
    }



    fn render_header_fields(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let Some(editor) = self.editing.as_ref() else { return v_flex().into_any_element(); };
        v_flex().gap(space::S4)
            .child(label(text::TITLE_COMPACT, s::HEADERS).text_color(r.text_muted))
            .children(editor.headers.iter().enumerate().map(|(index, field)| {
                h_flex().gap(space::S4).child(Input::new(&field.name)).child(Input::new(&field.value))
                    .child(IconButton::new(format!("provider-header-delete-{index}"), IconName::Trash).tooltip(s::DELETE).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(editor) = &mut this.editing && index < editor.headers.len() { editor.headers.remove(index); cx.notify(); }
                    })))
            }))
            .child(Button::new("provider-add-header").ghost().label(s::ADD_HEADER).on_click(cx.listener(|this, _, window, cx| this.add_header(window, cx))))
            .into_any_element()
    }



    fn editor(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let Some(editor) = self.editing.as_ref() else { return v_flex().into_any_element(); };
        let field = |title: &'static str, input: &Entity<InputState>| v_flex().gap(space::S2).child(label(text::TITLE_COMPACT, title).text_color(r.text_muted)).child(Input::new(input));
        let protocol = self.active_protocol(cx);
        let url_input = match protocol { Protocol::OpenaiChat => &editor.openai, Protocol::OpenaiResponses => &editor.responses, Protocol::AnthropicMessages => &editor.anthropic };
        let raw_url = url_input.read(cx).value().to_string();
        let url_field = field(s::API_URL, url_input).when(!raw_url.trim().is_empty(), |view| view.child(label(text::METADATA, s::endpoint_preview(&preview(&raw_url, protocol))).text_color(r.text_subtle)));
        v_flex().id("provider-editor").gap(space::S8).p(space::S12).rounded(radius::MD).bg(r.container_content_bg)
            .child(label(text::TITLE_CARD, if editor.id.is_some() { s::EDIT } else { s::ADD }))
            .child(field(s::NAME, &editor.name))
            .child(url_field)
            .child(label(text::METADATA, s::URL_HINT).text_color(r.text_subtle))
            .when(editor.origin_locked, |view| view.child(label(text::METADATA, s::ORIGIN_LOCKED).text_color(r.feedback_warning)))
            .child(label(text::TITLE_COMPACT, s::DEFAULT_PROTOCOL).text_color(r.text_muted))
            .child(Select::new(&editor.default_protocol).id("provider-default-protocol").placeholder(s::CHAT).cleanable(false))
            .child(label(text::TITLE_COMPACT, s::ACCESS).text_color(r.text_muted))
            .child(Segment::new("provider-access", [s::DIRECT, s::GATEWAY], usize::from(editor.access == Access::Gateway)).on_select(cx.listener(|this, index: &usize, _, cx| { if let Some(editor) = &mut this.editing { editor.access = if *index == 0 { Access::Direct } else { Access::Gateway }; cx.notify(); } })))
            .child(label(text::METADATA, s::GATEWAY_HELP).text_color(r.text_subtle))
            .child(self.render_header_fields(cx))
            .children(editor.models.iter().enumerate().map(|(index, model)| self.render_model(index, model, cx)))
            .child(h_flex().gap(space::S4).child(Input::new(&editor.model_id)).child(Button::new("provider-add-model").ghost().label(s::ADD_BUTTON).on_click(cx.listener(|this, _, window, cx| this.add_model(window, cx)))).child(Button::new("provider-discover-models").ghost().label(if editor.probing { s::PROBING } else { s::FIND_MODELS }).disabled(editor.probing).on_click(cx.listener(|this, _, window, cx| this.discover(window, cx)))))
            .child(self.render_keys(cx))
            .child(Switch::new("provider-enabled").checked(editor.enabled).color(r.feedback_success).label(s::ENABLED).on_click(cx.listener(|this, value: &bool, _, cx| { if let Some(editor) = &mut this.editing { editor.enabled = *value; cx.notify(); } })))
            .child(h_flex().gap(space::S8).justify_end()
                .child(Button::new("provider-cancel").ghost().label(s::CANCEL).disabled(editor.saving).on_click(cx.listener(|this, _, window, cx| { this.cancel_edit(window, cx); })))
                .child(Button::new("provider-save").primary().label(if editor.saving { s::SAVING } else { s::SAVE }).disabled(editor.saving).on_click(cx.listener(|this, _, _, cx| this.save(cx)))))
            .into_any_element()
    }
}


impl Render for ProvidersPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let query = self.search.read(cx).value().trim().to_lowercase();
        let rows = self.providers.clone().into_iter().filter(|provider| query.is_empty() || provider.input.name.to_lowercase().contains(&query) || provider.input.endpoints.iter().any(|endpoint| endpoint.base_url.to_lowercase().contains(&query))).map(|provider| {
            let id = provider.id.clone();
            let edit = provider.clone();
            let delete = provider.clone();
            let provider_for_toggle = provider.clone();
            let confirming = self.confirm_delete == Some(DeleteTarget::Provider(id.clone()));
            let endpoint = provider.input.endpoints.iter().find(|endpoint| endpoint.protocol == provider.input.default_protocol).or_else(|| provider.input.endpoints.first()).map(|endpoint| endpoint.base_url.clone()).unwrap_or_default();
            h_flex().id(format!("provider-row-{id}")).h(size::SETTINGS_ROW).px(space::S4).gap(space::S4).rounded(radius::MD)
                .when(confirming, |view| view.bg(r.control_bg_selected)).when(!confirming, |view| view.hover(|s| s.bg(r.control_bg_hovered)))
                .child(div().size(size::STATUS_DOT).rounded_full().bg(if provider.input.enabled { r.feedback_success } else { r.control_disabled_bg }))
                .child(v_flex().flex_1().min_w(px(0.)).child(label(text::UI, provider.input.name.clone()).truncate()).child(label(text::METADATA, format!("{} · {}", endpoint, s::model_summary(provider.input.models.len()))).text_color(r.text_subtle).truncate()))
                .child(Switch::new(format!("provider-enabled-{id}")).checked(provider.input.enabled).color(r.feedback_success).on_click(cx.listener(move |this, enabled: &bool, _, cx| this.toggle_provider(provider_for_toggle.clone(), *enabled, cx))))
                .child(IconButton::new(format!("provider-edit-{id}"), IconName::Pencil).tooltip(s::EDIT).on_click(cx.listener(move |this, _, window, cx| this.begin_edit(Some(edit.clone()), window, cx))))
                .child(if confirming { Button::new(format!("provider-confirm-delete-{id}")).danger().label(s::CONFIRM_DELETE).on_click(cx.listener(move |this, _, _, cx| this.delete_provider(delete.clone(), cx))).into_any_element() } else { IconButton::new(format!("provider-delete-{id}"), IconName::Trash).tooltip(s::DELETE).on_click(cx.listener(move |this, _, _, cx| this.delete_provider(delete.clone(), cx))).into_any_element() })
        }).collect::<Vec<_>>();
        let list = v_flex().id("provider-list").flex_1().min_h(px(0.)).gap(space::S2).overflow_y_scrollbar().children(rows).when(self.loading, |view| view.child(label(text::METADATA, s::LOADING).text_color(r.text_subtle))).when(self.providers.is_empty() && !self.loading, |view| view.child(label(text::METADATA, s::EMPTY).text_color(r.text_subtle))).child(Button::new("provider-add").ghost().label(s::ADD).on_click(cx.listener(|this, _, window, cx| this.begin_edit(None, window, cx))));
        let editor = self.editing.as_ref().map(|_| self.editor(cx));
        h_flex().id("settings-providers").size_full().min_w(px(0.)).gap(space::S12)
            .child(v_flex().w(size::PROVIDER_LIST).flex_none().min_h(px(0.)).gap(space::S8).child(Input::new(&self.search)).child(list))
            .child(v_flex().flex_1().min_w(px(0.)).min_h(px(0.)).overflow_y_scrollbar().when_some(editor, |view, editor| view.child(editor)))
            .into_any_element()
    }
}
