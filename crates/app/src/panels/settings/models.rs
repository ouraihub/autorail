//! Default model, reasoning effort and model visibility preferences.
//!
//! Catalog choices drive the default selectors; enabled provider models drive visibility so
//! hidden models can always be restored. One preference write runs at a time, and routing
//! changes received during background work are coalesced into a subsequent refresh.

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, IndexPath,
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState},
        select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
        switch::Switch,
    },
    prelude::FluentBuilder as _,
};
use thinkrail_core::{
    BackendKind, ModelChoice, ModelPrefs, ModelRef, Provider, ProviderModel, ThinkingLevel,
};
use thinkrail_ui::{
    ThinkrailTheme,
    components::label,
    metrics::{space, text},
};

use crate::{
    state::{AppEvent, AppState, run},
    strings::settings_models as s,
};

#[derive(Clone)]
struct SelectOption<T> {
    title: SharedString,
    value: T,
}

impl<T: Clone + PartialEq> SelectItem for SelectOption<T> {
    type Value = T;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &T {
        &self.value
    }
}

type ModelOptions = SearchableVec<SelectOption<Option<ModelRef>>>;
type EffortOptions = SearchableVec<SelectOption<ThinkingLevel>>;

struct VisibilityModel {
    reference: ModelRef,
    provider_name: String,
    model: ProviderModel,
}

impl VisibilityModel {
    fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.model.model_id.to_lowercase().contains(query)
            || self.model.name.to_lowercase().contains(query)
    }
}

fn visibility_models(providers: Vec<Provider>) -> Vec<VisibilityModel> {
    let mut models = Vec::new();
    for provider in providers.into_iter().filter(|provider| provider.input.enabled) {
        for model in provider.input.models.into_iter().filter(|model| model.enabled) {
            models.push(VisibilityModel {
                reference: ModelRef {
                    provider: provider.id.clone(),
                    model_id: model.model_id.clone(),
                },
                provider_name: provider.input.name.clone(),
                model,
            });
        }
    }
    models
}

fn model_reference(choice: &ModelChoice) -> ModelRef {
    ModelRef {
        provider: choice.provider.clone(),
        model_id: choice.model.id.clone(),
    }
}

#[derive(Clone)]
enum PreferenceWrite {
    Default { model: Option<ModelRef>, effort: Option<ThinkingLevel> },
    Hidden { model: ModelRef, hidden: bool },
}

#[derive(Clone)]
enum Retry {
    Load,
    Write(PreferenceWrite),
}

struct Failure {
    message: String,
    retry: Retry,
}

pub struct ModelsPage {
    state: Entity<AppState>,
    prefs: ModelPrefs,
    catalog: Vec<ModelChoice>,
    all_models: Vec<VisibilityModel>,
    default_model: Entity<SelectState<ModelOptions>>,
    default_effort: Entity<SelectState<EffortOptions>>,
    selectors_dirty: bool,
    search: Entity<InputState>,
    query: String,
    expanded: bool,
    loaded: bool,
    loading: bool,
    saving: bool,
    reload_pending: bool,
    failure: Option<Failure>,
    _subscriptions: Vec<Subscription>,
}

impl ModelsPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(s::SEARCH));
        let default_model = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![SelectOption { title: s::NONE.into(), value: None }]),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let default_effort = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::<SelectOption<ThinkingLevel>>::new()), None, window, cx)
        });
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
                if matches!(event, AppEvent::Routing) {
                    this.reload(cx);
                }
            }),
            cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = input.read(cx).value().trim().to_lowercase();
                    cx.notify();
                }
            }),
            cx.subscribe(&default_model, |this, _, event: &SelectEvent<ModelOptions>, cx| {
                let SelectEvent::Confirm(model) = event;
                this.select_default(model.clone().flatten(), cx);
            }),
            cx.subscribe(&default_effort, |this, _, event: &SelectEvent<EffortOptions>, cx| {
                let SelectEvent::Confirm(effort) = event;
                if let Some(choice) = this.selected_model() {
                    let model = Some(model_reference(choice));
                    let effort = effort.filter(|effort| choice.model.efforts.contains(effort));
                    this.write(PreferenceWrite::Default { model, effort }, cx);
                }
            }),
        ];
        let mut page = Self {
            state,
            prefs: ModelPrefs::default(),
            catalog: Vec::new(),
            all_models: Vec::new(),
            default_model,
            default_effort,
            selectors_dirty: true,
            search,
            query: String::new(),
            expanded: false,
            loaded: false,
            loading: false,
            saving: false,
            reload_pending: false,
            failure: None,
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        page
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.saving {
            self.reload_pending = true;
            return;
        }
        self.loading = true;
        self.reload_pending = false;
        let services = &self.state.read(cx).services;
        let prefs = services.prefs.clone();
        let router = services.router.clone();
        let providers = services.providers.clone();
        run(
            cx,
            move || {
                let prefs = prefs.prefs()?;
                let catalog = router.catalog(BackendKind::Pi)?;
                let all_models = visibility_models(providers.providers()?);
                Ok((prefs, catalog, all_models))
            },
            |this, result, cx| {
                this.loading = false;
                match result {
                    Ok((prefs, catalog, all_models)) => {
                        this.prefs = prefs;
                        this.catalog = catalog;
                        this.all_models = all_models;
                        this.loaded = true;
                        this.selectors_dirty = true;
                        if this.failure.as_ref().is_some_and(|failure| matches!(&failure.retry, Retry::Load)) {
                            this.failure = None;
                        }
                    }
                    Err(error) => {
                        this.failure = Some(Failure {
                            message: s::load_error(&error.to_string()),
                            retry: Retry::Load,
                        });
                        this.state.update(cx, |state, cx| state.report(&error, cx));
                    }
                }
                if this.reload_pending {
                    this.reload(cx);
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn selected_model(&self) -> Option<&ModelChoice> {
        let selected = self.prefs.default_model.as_ref()?;
        self.catalog.iter().find(|choice| {
            choice.provider == selected.provider && choice.model.id == selected.model_id
        })
    }

    fn select_default(&mut self, model: Option<ModelRef>, cx: &mut Context<Self>) {
        let effort = model.as_ref().and_then(|model| {
            let choice = self.catalog.iter().find(|choice| {
                choice.provider == model.provider && choice.model.id == model.model_id
            })?;
            self.prefs.default_effort.filter(|effort| choice.model.efforts.contains(effort))
        });
        self.write(PreferenceWrite::Default { model, effort }, cx);
    }

    fn write(&mut self, write: PreferenceWrite, cx: &mut Context<Self>) {
        if self.loading || self.saving || !self.loaded {
            self.selectors_dirty = true;
            cx.notify();
            return;
        }
        let unchanged = match &write {
            PreferenceWrite::Default { model, effort } => {
                self.prefs.default_model == *model && self.prefs.default_effort == *effort
            }
            PreferenceWrite::Hidden { model, hidden } => self.prefs.hidden.contains(model) == *hidden,
        };
        if unchanged {
            return;
        }
        self.saving = true;
        self.failure = None;
        let prefs = self.state.read(cx).services.prefs.clone();
        let retry = write.clone();
        run(
            cx,
            move || match write {
                PreferenceWrite::Default { model, effort } => prefs.set_default(model, effort),
                PreferenceWrite::Hidden { model, hidden } => prefs.set_hidden(&model, hidden),
            },
            move |this, result, cx| {
                this.saving = false;
                this.selectors_dirty = true;
                match result {
                    Ok(prefs) => {
                        this.prefs = prefs;
                        this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    }
                    Err(error) => {
                        this.failure = Some(Failure {
                            message: s::save_error(&error.to_string()),
                            retry: Retry::Write(retry),
                        });
                        this.state.update(cx, |state, cx| state.report(&error, cx));
                    }
                }
                if this.reload_pending {
                    this.reload(cx);
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        let retry = self.failure.as_ref().map(|failure| failure.retry.clone());
        match retry {
            Some(Retry::Load) => self.reload(cx),
            Some(Retry::Write(write)) => self.write(write, cx),
            None => {}
        }
    }

    fn sync_selectors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selectors_dirty {
            return;
        }
        self.selectors_dirty = false;
        let mut options = vec![SelectOption { title: s::NONE.into(), value: None }];
        options.extend(self.catalog.iter().map(|choice| SelectOption {
            title: s::model_label(&choice.provider_name, &choice.model.name, &choice.model.id).into(),
            value: Some(model_reference(choice)),
        }));
        let selected = self.selected_model();
        let model = selected.map(model_reference);
        let efforts = selected.map(|choice| choice.model.efforts.as_slice()).unwrap_or_default();
        let effort = self.prefs.default_effort.filter(|effort| efforts.contains(effort));
        let effort_options = efforts.iter().map(|effort| SelectOption {
            title: s::effort_label(*effort).into(),
            value: *effort,
        }).collect::<Vec<_>>();
        self.default_model.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(options), window, cx);
            select.set_selected_value(&model, window, cx);
        });
        self.default_effort.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(effort_options), window, cx);
            if let Some(effort) = effort {
                select.set_selected_value(&effort, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }
}

impl Render for ModelsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_selectors(window, cx);
        let roles = ThinkrailTheme::global(cx).roles;
        let busy = self.loading || self.saving;
        let disabled = busy || !self.loaded;
        let effort_disabled = disabled || self.selected_model().is_none_or(|choice| choice.model.efforts.is_empty());
        let unavailable_default = self.prefs.default_model.is_some() && self.selected_model().is_none();
        let has_matches = self.all_models.iter().any(|model| model.matches(&self.query));
        let rows = self.all_models.iter().enumerate().filter(|(_, model)| model.matches(&self.query)).map(|(index, model)| {
            let hidden = self.prefs.hidden.contains(&model.reference);
            let reference = model.reference.clone();
            h_flex()
                .gap(space::S8)
                .py(space::S8)
                .border_b_1()
                .border_color(roles.border_muted)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(label(text::UI, model.model.name.clone()))
                        .child(label(text::METADATA, s::model_label(
                            &model.provider_name, &model.model.name, &model.model.model_id,
                        )).text_color(roles.text_subtle))
                        .child(label(text::METADATA, s::model_details(
                            model.model.context_window, model.model.reasoning, model.model.vision,
                        )).text_color(roles.text_subtle)),
                )
                .child(
                    Switch::new(format!("model-visibility-{index}"))
                        .checked(!hidden)
                        .color(roles.feedback_success)
                        .accessibility_label(model.model.name.clone())
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.write(PreferenceWrite::Hidden { model: reference.clone(), hidden: !*checked }, cx);
                        })),
                )
        }).collect::<Vec<_>>();
        let failure = self.failure.as_ref().map(|failure| failure.message.clone());

        v_flex()
            .id("settings-models")
            .gap(space::S16)
            .child(label(text::HEADING_MD, s::TITLE))
            .child(
                v_flex()
                    .gap(space::S8)
                    .border_t_1()
                    .border_color(roles.border_muted)
                    .pt(space::S24)
                    .child(label(text::TITLE_SECTION, s::DEFAULT))
                    .child(label(text::TITLE_COMPACT, s::SESSION_MODEL))
                    .child(Select::new(&self.default_model)
                        .id("settings-default-model")
                        .placeholder(s::NONE)
                        .search_placeholder(s::SEARCH)
                        .cleanable(true)
                        .disabled(disabled))
                    .when(unavailable_default, |view| view.child(
                        label(text::METADATA, s::UNAVAILABLE_DEFAULT).text_color(roles.text_subtle),
                    ))
                    .child(label(text::TITLE_COMPACT, s::EFFORT))
                    .child(Select::new(&self.default_effort)
                        .id("settings-default-effort")
                        .placeholder(if effort_disabled { s::NO_EFFORT } else { s::DEFAULT_EFFORT })
                        .cleanable(true)
                        .disabled(effort_disabled)),
            )
            .child(
                v_flex()
                    .gap(space::S8)
                    .border_t_1()
                    .border_color(roles.border_muted)
                    .pt(space::S24)
                    .child(
                        h_flex()
                            .child(label(text::TITLE_SECTION, s::VISIBLE).flex_1())
                            .child(Button::new("models-expand")
                                .ghost()
                                .label(if self.expanded { s::HIDE } else { s::SHOW })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.expanded = !this.expanded;
                                    cx.notify();
                                }))),
                    )
                    .when(self.expanded, |view| {
                        view.child(Input::new(&self.search))
                            .children(rows)
                            .when(self.loaded && !has_matches, |view| view.child(
                                label(text::METADATA, if self.all_models.is_empty() { s::EMPTY } else { s::NO_MATCHES })
                                    .text_color(roles.text_subtle),
                            ))
                    })
                    .child(label(text::METADATA, s::visibility_summary(self.prefs.hidden.len()))
                        .text_color(roles.text_subtle)),
            )
            .when(busy, |view| view.child(
                label(text::METADATA, if self.saving { s::SAVING } else { s::LOADING })
                    .text_color(roles.text_subtle),
            ))
            .when_some(failure, |view, message| view.child(
                v_flex()
                    .gap(space::S8)
                    .child(label(text::METADATA, message).text_color(roles.feedback_error))
                    .child(Button::new("models-retry")
                        .ghost()
                        .label(s::RETRY)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.retry(cx)))),
            ))
            .when(self.loaded && self.catalog.is_empty(), |view| view.child(
                label(text::METADATA, s::EMPTY).text_color(roles.text_subtle),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinkrail_core::{Access, Protocol, ProviderId, ProviderInput};

    fn provider(enabled: bool) -> Provider {
        Provider {
            id: ProviderId::from("provider"),
            revision: 0,
            input: ProviderInput {
                name: "Provider".into(),
                endpoints: Vec::new(),
                default_protocol: Protocol::OpenaiChat,
                headers: Vec::new(),
                access: Access::Direct,
                enabled,
                models: vec![ProviderModel {
                    model_id: "hidden-model-id".into(),
                    name: "Hidden Model".into(),
                    protocols: vec![Protocol::OpenaiChat],
                    context_window: None,
                    max_tokens: None,
                    vision: false,
                    reasoning: false,
                    tools: true,
                    efforts: Vec::new(),
                    enabled: true,
                }],
            },
            keys: Vec::new(),
        }
    }

    #[test]
    fn visibility_uses_enabled_provider_models_not_the_catalog() {
        let mut disabled_model = provider(true);
        disabled_model.input.models[0].enabled = false;
        let models = visibility_models(vec![provider(true), provider(false), disabled_model]);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].reference.model_id, "hidden-model-id");
        assert!(models[0].matches("hidden-model-id"));
        assert!(models[0].matches("hidden model"));
        assert!(!models[0].matches("missing"));
    }
}
