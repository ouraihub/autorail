//! 模型服务页（agent-settings 04 §7.6，对标 Aether `ProvidersSettings`）：左侧服务商栏，右侧选中的
//! 服务商。每次编辑立即保存（`commit`）；保存失败时修改留在 `draft` 里，可重试。

use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Pixels, Render, Styled as _, Subscription,
    Window,
    base::{h_flex, v_flex},
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        input::{InputEvent, InputState},
    },
    div, px,
};
use thinkrail_core::{Error, KeyCheck, Protocol, Provider, ProviderId, ProviderInput, ProviderService, Result};
use thinkrail_ui::{ThinkrailTheme, components::label, metrics::{radius, space, text}};

use crate::{state::{AppEvent, AppState, run}, strings::settings_providers as s};

mod detail;
mod dialogs;
mod keys;
mod models;
mod rail;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter { All, Enabled, Disabled }

pub struct ProvidersPage {
    state: Entity<AppState>,
    providers: Vec<Provider>,
    loaded: bool,
    selected: Option<ProviderId>,
    /// The selected provider's input as edited; `commit` saves it.
    draft: Option<ProviderInput>,
    /// The line (endpoint protocol) the detail pane shows.
    line: Protocol,
    search: Entity<InputState>,
    filter: Filter,
    url: Entity<InputState>,
    /// Set when the url input must be refilled from `draft` on the next render.
    sync_inputs: bool,
    url_error: bool,
    key_input: Entity<InputState>,
    models: models::ModelView,
    saving: bool,
    dirty: bool,
    failed: Option<String>,
    checking: bool,
    checks: Option<std::result::Result<Vec<KeyCheck>, String>>,
    /// Plaintext of the shown key; cleared on every selection change and when settings close.
    revealed: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProvidersPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(s::SEARCH));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder(s::URL_PLACEHOLDER));
        let key_input = cx.new(|cx| InputState::new(window, cx).placeholder(s::KEY_PLACEHOLDER).masked(true));
        let models = models::ModelView::new(window, cx);
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
                if matches!(event, AppEvent::Routing) && !this.saving && !this.dirty && this.failed.is_none() {
                    this.reload(cx);
                }
            }),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| if matches!(event, InputEvent::Change) { cx.notify() }),
            cx.subscribe(&models.query, |_, _, event: &InputEvent, cx| if matches!(event, InputEvent::Change) { cx.notify() }),
            cx.subscribe_in(&url, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.commit_url(window, cx);
                }
            }),
            cx.subscribe_in(&key_input, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.add_first_key(window, cx);
                }
            }),
        ];
        let mut page = Self {
            state, providers: Vec::new(), loaded: false, selected: None, draft: None, line: Protocol::OpenaiChat,
            search, filter: Filter::All, url, sync_inputs: true, url_error: false, key_input, models,
            saving: false, dirty: false, failed: None, checking: false, checks: None, revealed: None,
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        page
    }

    fn service(&self, cx: &App) -> std::sync::Arc<dyn ProviderService> {
        self.state.read(cx).services.providers.clone()
    }

    fn report(&mut self, error: &Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }

    /// Seeds the built-in presets (idempotent) and returns every provider.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let service = self.service(cx);
        run(cx, move || service.seed_presets(), |this, result, cx| {
            this.loaded = true;
            match result {
                Ok(providers) => this.providers = providers,
                Err(error) => this.report(&error, cx),
            }
            let keep = this.selected.as_ref().filter(|id| this.providers.iter().any(|p| &p.id == *id)).cloned();
            match keep.or_else(|| this.providers.first().map(|p| p.id.clone())) {
                Some(id) if this.draft.is_none() || this.selected.as_ref() != Some(&id) => this.select(id, cx),
                Some(id) => this.adopt(id, cx),
                None => { this.selected = None; this.draft = None; }
            }
            cx.notify();
        });
    }

    fn provider(&self) -> Option<&Provider> {
        self.selected.as_ref().and_then(|id| self.providers.iter().find(|p| &p.id == id))
    }

    /// Reloads `draft` from the stored snapshot (no pending edits exist).
    fn adopt(&mut self, id: ProviderId, cx: &mut Context<Self>) {
        if let Some(provider) = self.providers.iter().find(|p| p.id == id) {
            self.draft = Some(provider.input.clone());
            self.revealed = None;
            if !provider.input.endpoints.iter().any(|e| e.protocol == self.line) {
                self.line = provider.input.default_protocol;
            }
            self.sync_inputs = true;
            cx.notify();
        }
    }

    fn select(&mut self, id: ProviderId, cx: &mut Context<Self>) {
        self.selected = Some(id.clone());
        self.draft = None;
        self.failed = None;
        self.dirty = false;
        self.checks = None;
        self.revealed = None;
        self.models.reset();
        self.sync_inputs = true;
        if let Some(provider) = self.providers.iter().find(|p| p.id == id) {
            self.line = provider.input.default_protocol;
        }
        self.adopt(id, cx);
    }

    pub(crate) fn clear_sensitive(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.revealed = None;
        self.checks = None;
        self.sync_inputs = true;
        cx.notify();
    }

    /// Stores a fresh snapshot from a write and tells chats routing changed.
    fn stored(&mut self, provider: Provider, cx: &mut Context<Self>) {
        let id = provider.id.clone();
        match self.providers.iter_mut().find(|p| p.id == id) {
            Some(slot) => *slot = provider,
            None => self.providers.push(provider),
        }
        if self.selected.as_ref() == Some(&id) && !self.dirty {
            self.adopt(id, cx);
        }
        self.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
    }

    /// Saves `draft`. Edits made while a save runs are saved right after it.
    fn commit(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.failed = None;
        if !self.saving {
            self.drain(cx);
        }
        cx.notify();
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        self.dirty = false;
        let (Some(id), Some(input), Some(revision)) = (self.selected.clone(), self.draft.clone(), self.provider().map(|p| p.revision)) else {
            return;
        };
        self.saving = true;
        let service = self.service(cx);
        run(cx, move || service.update(&id, revision, input), |this, result, cx| {
            this.saving = false;
            match result {
                Ok(provider) => {
                    this.stored(provider, cx);
                    if this.dirty {
                        this.drain(cx);
                    }
                }
                // The edit stays in `draft`; "重试" saves it again.
                Err(error) => this.failed = Some(error.to_string()),
            }
            cx.notify();
        });
    }

    /// Applies `edit` to the draft and saves it.
    fn edit(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut ProviderInput)) {
        if let Some(draft) = &mut self.draft {
            edit(draft);
            self.commit(cx);
        }
    }

    fn line_url(&self) -> String {
        let draft = self.draft.as_ref();
        draft.and_then(|d| d.endpoints.iter().find(|e| e.protocol == self.line)).map(|e| e.base_url.clone()).unwrap_or_default()
    }

    fn commit_url(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let raw = self.url.read(cx).value().trim().to_owned();
        if raw == self.line_url() {
            self.url_error = false;
            return;
        }
        self.url_error = !valid_url(&raw);
        if self.url_error {
            cx.notify();
            return;
        }
        let line = self.line;
        self.edit(cx, |draft| thinkrail_core::set_line(draft, line, raw));
    }
}

/// UI draft check only; the store validates again.
fn valid_url(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

impl Render for ProvidersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.sync_inputs) {
            let url = self.line_url();
            self.url.update(cx, |input, cx| input.set_value(url, window, cx));
            self.key_input.update(cx, |input, cx| input.set_value("", window, cx));
            self.url_error = false;
        }
        h_flex().w_full().items_start().gap(space::S16).child(self.render_rail(cx)).child(self.render_detail(cx))
    }
}

/// Small bordered square with a provider's or model's initial (Aether avatar).
fn avatar(name: &str, cx: &App) -> impl IntoElement {
    let r = ThinkrailTheme::global(cx).roles;
    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    div().size(px(24.)).flex_none().rounded(radius::MD).bg(r.primary_subtle).flex().items_center().justify_center()
        .child(label(text::EMPHASIS, initial).text_color(r.primary))
}

/// A dialog with `body` and a 取消 / `ok` footer; `ok` returns true to close.
fn open_form<V: Render>(
    window: &mut Window,
    cx: &mut App,
    title: &'static str,
    width: Pixels,
    body: Entity<V>,
    ok: &'static str,
    on_ok: impl Fn(&mut V, &mut Window, &mut Context<V>) -> bool + 'static,
) {
    let on_ok = Rc::new(on_ok);
    window.open_dialog(cx, move |dialog, _, _| {
        let (enter_body, enter_ok, body, on_ok) = (body.clone(), on_ok.clone(), body.clone(), on_ok.clone());
        // Enter in a field runs the same check as the button; false keeps the dialog open.
        dialog.title(title).w(width).child(body.clone())
            .on_ok(move |_, window, cx| enter_body.update(cx, |view, cx| enter_ok(view, window, cx)))
            .footer(DialogFooter::new()
                .child(Button::new("form-cancel").outline().label(s::CANCEL).on_click(|_, window, cx| window.close_dialog(cx)))
                .child(Button::new("form-ok").primary().label(ok).on_click(move |_, window, cx| {
                    if body.update(cx, |view, cx| on_ok(view, window, cx)) {
                        window.close_dialog(cx);
                    }
                })))
    });
}

/// A labeled form field.
fn field(title: &'static str, control: impl IntoElement, cx: &App) -> impl IntoElement {
    let r = ThinkrailTheme::global(cx).roles;
    v_flex().gap(space::S4).child(label(text::TITLE_COMPACT, title).text_color(r.text_muted)).child(control)
}

fn error_text(message: Option<String>, cx: &App) -> Option<impl IntoElement> {
    let r = ThinkrailTheme::global(cx).roles;
    message.map(|message| label(text::METADATA, message).text_color(r.feedback_error))
}

/// Shared by every page action that runs a provider write and stores the snapshot it returns.
fn write(
    this: &mut ProvidersPage,
    cx: &mut Context<ProvidersPage>,
    job: impl FnOnce(&dyn ProviderService) -> Result<Provider> + Send + 'static,
) {
    let service = this.service(cx);
    run(cx, move || job(&*service), |this, result, cx| match result {
        Ok(provider) => this.stored(provider, cx),
        Err(error) => this.report(&error, cx),
    });
}
