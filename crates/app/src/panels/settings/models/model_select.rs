//! A model select with its effort select beside it, shared by 会话模型 and 评审模型. Each
//! owner supplies what its first entry means and which model's levels the effort list shows;
//! the pair only renders and reports choices.

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Styled as _, Subscription,
    Window,
    base::h_flex,
    component::{
        IndexPath,
        select::{SearchableVec, Select, SelectEvent, SelectState},
    },
    px,
};
use thinkrail_core::{ModelChoice, ModelRef, ThinkingLevel, clamp_effort};
use thinkrail_ui::metrics::space;

use super::{ModelsPage, SelectOption, Snapshot, model_reference};
use crate::strings::settings_models as s;

type ModelOptions = SearchableVec<SelectOption<Option<ModelRef>>>;
/// `None` is the list's first item, 默认: clears the saved effort.
type EffortOptions = SearchableVec<SelectOption<Option<ThinkingLevel>>>;

pub(super) struct ModelEffortSelects {
    model: Entity<SelectState<ModelOptions>>,
    effort: Entity<SelectState<EffortOptions>>,
    dirty: bool,
}

/// What one pair shows, computed by its owner from the snapshot.
pub(super) struct PairState<'a> {
    /// The saved model, kept in the list even when hidden.
    pub(super) saved: Option<&'a ModelRef>,
    /// The first entry's title: 引擎默认, or "{name}（默认）" for the review model.
    pub(super) none_title: String,
    /// The model whose levels the effort list offers (the saved one, or what it falls back to).
    pub(super) effort_model: Option<&'a ModelChoice>,
    pub(super) saved_effort: Option<ThinkingLevel>,
}

pub(super) fn find<'a>(snapshot: &'a Snapshot, model: &ModelRef) -> Option<&'a ModelChoice> {
    snapshot
        .catalog
        .iter()
        .find(|c| c.provider == model.provider && c.model.id == model.model_id)
}

/// The saved effort kept only where `choice` declares it (the services store what they get).
pub(super) fn kept_effort(
    saved: Option<ThinkingLevel>,
    choice: Option<&ModelChoice>,
) -> Option<ThinkingLevel> {
    saved.filter(|effort| choice.is_some_and(|c| c.model.efforts.contains(effort)))
}

impl ModelEffortSelects {
    /// `on_model` / `on_effort` receive the confirmed value (`None`: the first entry).
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<ModelsPage>,
        on_model: fn(&mut ModelsPage, Option<ModelRef>, &mut Context<ModelsPage>),
        on_effort: fn(&mut ModelsPage, Option<ThinkingLevel>, &mut Context<ModelsPage>),
    ) -> (Self, Vec<Subscription>) {
        let model = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SelectOption<Option<ModelRef>>>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let effort = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SelectOption<Option<ThinkingLevel>>>::new()),
                None,
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(
                &model,
                move |page, _, event: &SelectEvent<ModelOptions>, cx| {
                    let SelectEvent::Confirm(model) = event;
                    on_model(page, model.clone().flatten(), cx);
                },
            ),
            cx.subscribe(
                &effort,
                move |page, _, event: &SelectEvent<EffortOptions>, cx| {
                    let SelectEvent::Confirm(effort) = event;
                    on_effort(page, effort.flatten(), cx);
                },
            ),
        ];
        (
            Self {
                model,
                effort,
                dirty: true,
            },
            subscriptions,
        )
    }

    /// The selects re-read the snapshot on the next render (after a load, write or refused write).
    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Rebuilds both lists when dirty. Hidden models are left out, except the saved one.
    pub(super) fn sync(
        &mut self,
        snapshot: &Snapshot,
        pair: &PairState<'_>,
        window: &mut Window,
        cx: &mut Context<ModelsPage>,
    ) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut options = vec![SelectOption {
            title: pair.none_title.clone().into(),
            value: None,
        }];
        options.extend(
            snapshot
                .catalog
                .iter()
                .filter(|c| {
                    let reference = model_reference(c);
                    !snapshot.prefs.hidden.contains(&reference) || pair.saved == Some(&reference)
                })
                .map(|c| SelectOption {
                    title: s::model_label(&c.provider_name, &c.model.name, &c.model.id).into(),
                    value: Some(model_reference(c)),
                }),
        );
        let shown_model = pair
            .saved
            .and_then(|m| find(snapshot, m))
            .map(model_reference);
        let efforts = pair
            .effort_model
            .map(|c| c.model.efforts.as_slice())
            .unwrap_or_default();
        let shown_effort = clamp_effort(pair.saved_effort, efforts);
        let mut effort_options = vec![SelectOption {
            title: s::EFFORT_CLEAR.into(),
            value: None,
        }];
        effort_options.extend(efforts.iter().map(|effort| SelectOption {
            title: s::effort_label(*effort).into(),
            value: Some(*effort),
        }));
        self.model.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(options), window, cx);
            match shown_model {
                Some(model) => select.set_selected_value(&Some(model), window, cx),
                None => select.set_selected_index(Some(IndexPath::default().row(0)), window, cx),
            }
        });
        self.effort.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(effort_options), window, cx);
            match shown_effort {
                Some(effort) => select.set_selected_value(&Some(effort), window, cx),
                None => select.set_selected_index(None, window, cx),
            }
        });
    }

    /// The two selects on one line. `id` prefixes their element ids.
    pub(super) fn render(
        &self,
        id: &'static str,
        pair: &PairState<'_>,
        disabled: bool,
    ) -> impl IntoElement + use<> {
        let efforts_empty = pair.effort_model.is_none_or(|c| c.model.efforts.is_empty());
        // A selected effort shows as "力度 {档位}" through the prefix; with none selected the
        // placeholder says whether the model has levels at all.
        let effort = Select::new(&self.effort)
            .id(format!("{id}-effort"))
            .w(px(120.))
            .title_prefix(s::EFFORT_PREFIX)
            .placeholder(if efforts_empty {
                s::EFFORT_NONE
            } else {
                s::EFFORT_DEFAULT
            })
            .disabled(disabled || efforts_empty);
        h_flex()
            .gap(space::S8)
            .child(
                Select::new(&self.model)
                    .id(format!("{id}-model"))
                    .w(px(220.))
                    .placeholder(pair.none_title.clone())
                    .search_placeholder(s::SEARCH)
                    .cleanable(true)
                    .disabled(disabled),
            )
            .child(effort)
    }
}
