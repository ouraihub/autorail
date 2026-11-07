//! 设置 → 模型 (agent-settings 08): the default model and effort, model visibility, the 评审
//! settings, GPT context windows and, per engine, what its 编码搭档 choice does to these settings.
//!
//! The coordinator owns the loaded snapshot, the one-at-a-time preference write and its error.
//! Routing changes received during background work are coalesced into a subsequent refresh.
//! Each group renders in its own submodule and changes state only through the methods here.

use gpui_kit::{
    AppContext as _, Context, Entity, SharedString, Subscription, Window,
    component::{
        input::{InputEvent, InputState},
        select::SelectItem,
    },
};
use thinkrail_core::{
    BackendKind, EngineNote, ModelChoice, ModelPrefs, ModelRef, ModelStatus, Provider,
    ProviderModel, ReviewSwitch, ThinkingLevel,
};

use crate::{
    state::{AppEvent, AppState, run},
    strings::settings_models as s,
};

mod context_group;
mod default_group;
mod model_select;
mod review_group;
#[cfg(test)]
mod tests;
mod view;
mod visibility;

use context_group::ContextGroup;
use model_select::ModelEffortSelects;

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

/// One enabled chat model of an enabled provider, hidden or not, so hidden models can be restored.
struct VisibilityModel {
    reference: ModelRef,
    provider_name: String,
    model: ProviderModel,
}

impl VisibilityModel {
    /// Case-insensitive substring match on name, model id or provider; `query` is lowercase.
    fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.model.model_id.to_lowercase().contains(query)
            || self.model.name.to_lowercase().contains(query)
            || self.provider_name.to_lowercase().contains(query)
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

/// Everything one load reads, applied together.
struct Snapshot {
    prefs: ModelPrefs,
    /// `settings_catalog`: every routable model, hidden ones included.
    catalog: Vec<ModelChoice>,
    all_models: Vec<VisibilityModel>,
    default_status: ModelStatus,
    /// The pinned review model's status; `Available` when the review follows the default.
    review_status: ModelStatus,
    notes: Vec<(BackendKind, EngineNote)>,
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
    /// GPT 上下文窗口: `None` removes a model's override.
    Context(Vec<(ModelRef, Option<u32>)>),
    Review {
        model: Option<ModelRef>,
        effort: Option<ThinkingLevel>,
    },
    Switch(ReviewSwitch, bool),
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
    snapshot: Option<Snapshot>,
    default_group: ModelEffortSelects,
    review_group: ModelEffortSelects,
    context_group: ContextGroup,
    search: Entity<InputState>,
    query: String,
    expanded: bool,
    loading: bool,
    saving: bool,
    reload_pending: bool,
    failure: Option<Failure>,
    _subscriptions: Vec<Subscription>,
}

impl ModelsPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(s::SEARCH));
        let (default_group, mut subscriptions) = default_group::new(window, cx);
        let (review_group, review_subscriptions) = review_group::new(window, cx);
        subscriptions.extend(review_subscriptions);
        let (context_group, context_subscriptions) = ContextGroup::new(window, cx);
        subscriptions.extend(context_subscriptions);
        subscriptions.push(cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
            if matches!(event, AppEvent::Routing) {
                this.reload(cx);
            }
        }));
        subscriptions.push(
            cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = input.read(cx).value().trim().to_lowercase();
                    cx.notify();
                }
            }),
        );
        let mut page = Self {
            state,
            snapshot: None,
            default_group,
            review_group,
            context_group,
            search,
            query: String::new(),
            expanded: false,
            loading: false,
            saving: false,
            reload_pending: false,
            failure: None,
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        page
    }

    fn busy(&self) -> bool {
        self.loading || self.saving
    }

    /// Both model/effort pairs re-read the snapshot on the next render.
    fn mark_selects_dirty(&mut self) {
        self.default_group.mark_dirty();
        self.review_group.mark_dirty();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
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
                let status = |model: &Option<ModelRef>| match model {
                    Some(model) => router.model_status(model),
                    None => Ok(ModelStatus::Available),
                };
                let default_status = status(&prefs.default_model)?;
                let review_status = status(&prefs.review.model)?;
                Ok(Snapshot {
                    catalog: router.settings_catalog()?,
                    all_models: visibility_models(providers.providers()?),
                    notes: router.engine_notes()?,
                    default_status,
                    review_status,
                    prefs,
                })
            },
            |this, result, cx| {
                this.loading = false;
                match result {
                    Ok(snapshot) => {
                        this.snapshot = Some(snapshot);
                        this.mark_selects_dirty();
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

    fn write(&mut self, write: PreferenceWrite, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        if self.busy() {
            self.mark_selects_dirty();
            cx.notify();
            return;
        }
        let prefs = &snapshot.prefs;
        let unchanged = match &write {
            PreferenceWrite::Default { model, effort } => {
                prefs.default_model == *model && prefs.default_effort == *effort
            }
            PreferenceWrite::Hidden { model, hidden } => prefs.hidden.contains(model) == *hidden,
            PreferenceWrite::Context(changes) => changes
                .iter()
                .all(|(model, tokens)| prefs.context_override(model) == *tokens),
            PreferenceWrite::Review { model, effort } => {
                prefs.review.model == *model && prefs.review.effort == *effort
            }
            PreferenceWrite::Switch(switch, on) => {
                let current = match switch {
                    ReviewSwitch::AgentReview => prefs.review.agent_review,
                    ReviewSwitch::AutoFix => prefs.review.auto_fix,
                };
                current == *on
            }
        };
        if unchanged {
            return;
        }
        self.saving = true;
        self.failure = None;
        let service = self.state.read(cx).services.prefs.clone();
        let retry = write.clone();
        run(
            cx,
            move || match write {
                PreferenceWrite::Default { model, effort } => service.set_default(model, effort),
                PreferenceWrite::Hidden { model, hidden } => service.set_hidden(&model, hidden),
                PreferenceWrite::Context(changes) => service.set_context_overrides(&changes),
                PreferenceWrite::Review { model, effort } => {
                    service.set_review_model(model, effort)
                }
                PreferenceWrite::Switch(switch, on) => service.set_review_switch(switch, on),
            },
            move |this, result, cx| {
                this.saving = false;
                this.mark_selects_dirty();
                match result {
                    // Shown at once; the reload this triggers brings the new status and notes.
                    Ok(prefs) => {
                        if let Some(snapshot) = this.snapshot.as_mut() {
                            snapshot.prefs = prefs;
                        }
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
