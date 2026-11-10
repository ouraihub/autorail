//! 模型服务页（agent-settings 04 §7.6，对标 Aether `ProvidersSettings`）：左侧服务商栏，右侧选中的
//! 服务商。每次编辑立即保存（`commit`）；保存失败时修改留在 `draft` 里，可重试。

use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, Render, Styled as _, Subscription, Window,
    component::input::{InputEvent, InputState},
    px,
};
use lumakit_gpui::providers as ui;
use thinkrail_core::{
    Error, KeyCheck, Protocol, Provider, ProviderId, ProviderInput, ProviderService, Result,
};
use thinkrail_ui::{ThinkrailTheme, components::label, metrics::text};

use crate::{
    state::{AppEvent, AppState, run},
    strings::settings_providers as s,
};

mod detail;
mod dialogs;
mod forms;
mod keys;
mod models;
mod rail;
#[cfg(test)]
mod tests;
mod theme;

use forms::open_form;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Enabled,
    Disabled,
}

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
    key_saving: bool,
    key_generation: u64,
    reset_key: bool,
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
        let key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(s::KEY_PLACEHOLDER)
                .masked(true)
        });
        let models = models::ModelView::new(window, cx);
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
                if matches!(event, AppEvent::Routing)
                    && !this.saving
                    && !this.dirty
                    && this.failed.is_none()
                {
                    this.reload(cx);
                }
            }),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify()
                }
            }),
            cx.subscribe(&models.query, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify()
                }
            }),
            cx.subscribe_in(&url, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.commit_url(window, cx);
                }
            }),
            cx.subscribe_in(
                &key_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.key_generation = this.key_generation.wrapping_add(1);
                    }
                    if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        this.add_first_key(window, cx);
                    }
                },
            ),
        ];
        let mut page = Self {
            state,
            providers: Vec::new(),
            loaded: false,
            selected: None,
            draft: None,
            line: Protocol::OpenaiChat,
            search,
            filter: Filter::All,
            url,
            sync_inputs: true,
            url_error: false,
            key_input,
            models,
            saving: false,
            dirty: false,
            failed: None,
            key_saving: false,
            key_generation: 0,
            reset_key: false,
            checking: false,
            checks: None,
            revealed: None,
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
        run(
            cx,
            move || service.seed_presets(),
            |this, result, cx| {
                this.loaded = true;
                match result {
                    Ok(providers) => this.providers = providers,
                    Err(error) => this.report(&error, cx),
                }
                let keep = this
                    .selected
                    .as_ref()
                    .filter(|id| this.providers.iter().any(|p| &p.id == *id))
                    .cloned();
                match keep.or_else(|| this.providers.first().map(|p| p.id.clone())) {
                    Some(id) if this.draft.is_none() || this.selected.as_ref() != Some(&id) => {
                        this.select(id, cx)
                    }
                    Some(id) => this.adopt(id, cx),
                    None => {
                        this.selected = None;
                        this.draft = None;
                    }
                }
                cx.notify();
            },
        );
    }

    fn provider(&self) -> Option<&Provider> {
        self.selected
            .as_ref()
            .and_then(|id| self.providers.iter().find(|p| &p.id == id))
    }

    /// Reloads `draft` from the stored snapshot (no pending edits exist).
    fn adopt(&mut self, id: ProviderId, cx: &mut Context<Self>) {
        let previous_url = self.line_url();
        if let Some(provider) = self.providers.iter().find(|p| p.id == id) {
            self.draft = Some(provider.input.clone());
            self.revealed = None;
            if !provider
                .input
                .endpoints
                .iter()
                .any(|e| e.protocol == self.line)
            {
                self.line = provider.input.default_protocol;
            }
            self.sync_inputs |= self.line_url() != previous_url;
            cx.notify();
        }
    }

    fn select(&mut self, id: ProviderId, cx: &mut Context<Self>) {
        self.key_generation = self.key_generation.wrapping_add(1);
        self.reset_key = true;
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
    pub(crate) fn clear_sensitive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.key_generation = self.key_generation.wrapping_add(1);
        self.key_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.revealed = None;
        self.checks = None;
        self.sync_inputs = true;
        cx.notify();
    }

    fn stored(&mut self, provider: Provider, cx: &mut Context<Self>) {
        let id = provider.id.clone();
        self.store_snapshot(provider, cx);
        if self.selected.as_ref() == Some(&id) && !self.dirty {
            self.adopt(id, cx);
        }
    }

    /// Publishes a saved snapshot without replacing a form's pending draft.
    fn store_snapshot(&mut self, provider: Provider, cx: &mut Context<Self>) {
        match self.providers.iter_mut().find(|p| p.id == provider.id) {
            Some(slot) => *slot = provider,
            None => self.providers.push(provider),
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
        let (Some(id), Some(input), Some(revision)) = (
            self.selected.clone(),
            self.draft.clone(),
            self.provider().map(|p| p.revision),
        ) else {
            return;
        };
        self.saving = true;
        let service = self.service(cx);
        let target = id.clone();
        run(
            cx,
            move || service.update(&id, revision, input),
            move |this, result, cx| {
                this.saving = false;
                match result {
                    Ok(provider) => {
                        this.stored(provider, cx);
                        if this.dirty {
                            this.drain(cx);
                        }
                    }
                    // The edit stays in `draft`; "重试" saves it again.
                    Err(error) if this.selected.as_ref() == Some(&target) => {
                        this.failed = Some(error.to_string())
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
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
        draft
            .and_then(|d| d.endpoints.iter().find(|e| e.protocol == self.line))
            .map(|e| e.base_url.clone())
            .unwrap_or_default()
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
    url::Url::parse(raw)
        .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

impl Render for ProvidersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.sync_inputs) {
            let url = self.line_url();
            self.url
                .update(cx, |input, cx| input.set_value(url, window, cx));
            self.url_error = false;
        }
        if std::mem::take(&mut self.reset_key) {
            self.key_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let viewport = window.viewport_size().width;
        let stacked_nav = viewport < lumakit_design::metrics::size::NARROW;
        let content_width = viewport.min(px(1040.))
            - thinkrail_ui::metrics::space::S64
            - if stacked_nav { px(0.) } else { px(192.) };
        ui::page(
            self.render_rail(cx),
            self.render_detail(cx),
            content_width >= lumakit_design::metrics::size::NARROW,
        )
    }
}

/// Provider/model initial used by the coding settings page as well.
pub(in crate::panels::settings) fn avatar(name: &str, cx: &App) -> impl IntoElement {
    lumakit_design::components::settings::avatar(name, &theme::roles(cx))
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
    run(
        cx,
        move || job(&*service),
        |this, result, cx| match result {
            Ok(provider) => this.stored(provider, cx),
            Err(error) => this.report(&error, cx),
        },
    );
}
