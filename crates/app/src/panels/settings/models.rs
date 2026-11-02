//! Default model, reasoning effort and model visibility preferences.
//!
//! Catalog choices drive the default selectors; enabled provider models drive visibility so
//! hidden models can always be restored. One preference write runs at a time, and routing
//! changes received during background work are coalesced into a subsequent refresh.

use gpui_kit::{
    AppContext as _, Context, Entity, SharedString, Subscription, Window,
    component::{
        IndexPath,
        input::{InputEvent, InputState},
        select::{SearchableVec, SelectEvent, SelectItem, SelectState},
    },
};
use thinkrail_core::{
    BackendKind, ModelChoice, ModelPrefs, ModelRef, Provider, ProviderModel, ThinkingLevel,
};

use crate::{
    state::{AppEvent, AppState, run},
    strings::settings_models as s,
};

#[cfg(test)]
mod tests;
mod view;

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
    for provider in providers
        .into_iter()
        .filter(|provider| provider.input.enabled)
    {
        for model in
            provider.input.models.into_iter().filter(|model| {
                model.enabled && model.model_type == thinkrail_core::ModelType::Chat
            })
        {
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
    Default {
        model: Option<ModelRef>,
        effort: Option<ThinkingLevel>,
    },
    Hidden {
        model: ModelRef,
        hidden: bool,
    },
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
                SearchableVec::new(vec![SelectOption {
                    title: s::NONE.into(),
                    value: None,
                }]),
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let default_effort = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SelectOption<ThinkingLevel>>::new()),
                None,
                window,
                cx,
            )
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
            cx.subscribe(
                &default_model,
                |this, _, event: &SelectEvent<ModelOptions>, cx| {
                    let SelectEvent::Confirm(model) = event;
                    this.select_default(model.clone().flatten(), cx);
                },
            ),
            cx.subscribe(
                &default_effort,
                |this, _, event: &SelectEvent<EffortOptions>, cx| {
                    let SelectEvent::Confirm(effort) = event;
                    if let Some(choice) = this.selected_model() {
                        let model = Some(model_reference(choice));
                        let effort = effort.filter(|effort| choice.model.efforts.contains(effort));
                        this.write(PreferenceWrite::Default { model, effort }, cx);
                    }
                },
            ),
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
                        if this
                            .failure
                            .as_ref()
                            .is_some_and(|failure| matches!(&failure.retry, Retry::Load))
                        {
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
            self.prefs
                .default_effort
                .filter(|effort| choice.model.efforts.contains(effort))
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
            PreferenceWrite::Hidden { model, hidden } => {
                self.prefs.hidden.contains(model) == *hidden
            }
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
}
