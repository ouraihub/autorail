//! 默认模型 group: the 会话模型 row (model and effort selects on one line), the unavailable-default
//! notice and one note per engine whose 编码搭档 choice bypasses these settings (08 §4.1–4.4).

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Styled as _,
    Subscription, Window,
    base::{h_flex, v_flex},
    component::{
        IndexPath,
        select::{SearchableVec, Select, SelectEvent, SelectState},
    },
    px,
};
use thinkrail_core::{ModelChoice, ModelRef, ThinkingLevel, clamp_effort};
use thinkrail_ui::{Roles, metrics::space};

use super::{
    super::rows::{notice, row},
    ModelsPage, PreferenceWrite, SelectOption, Snapshot, model_reference,
};
use crate::strings::settings_models as s;

type ModelOptions = SearchableVec<SelectOption<Option<ModelRef>>>;
/// `None` is the list's first item, 默认: clears the saved effort.
type EffortOptions = SearchableVec<SelectOption<Option<ThinkingLevel>>>;

pub(super) struct DefaultGroup {
    model: Entity<SelectState<ModelOptions>>,
    effort: Entity<SelectState<EffortOptions>>,
    dirty: bool,
}

/// The saved default's catalog entry, if it is routable now.
fn selected(snapshot: &Snapshot) -> Option<&ModelChoice> {
    let wanted = snapshot.prefs.default_model.as_ref()?;
    snapshot
        .catalog
        .iter()
        .find(|c| c.provider == wanted.provider && c.model.id == wanted.model_id)
}

fn find<'a>(snapshot: &'a Snapshot, model: &ModelRef) -> Option<&'a ModelChoice> {
    snapshot
        .catalog
        .iter()
        .find(|c| c.provider == model.provider && c.model.id == model.model_id)
}

/// The saved effort kept only where `choice` declares it (`set_default` stores what it is given).
pub(super) fn kept_effort(
    saved: Option<ThinkingLevel>,
    choice: Option<&ModelChoice>,
) -> Option<ThinkingLevel> {
    saved.filter(|effort| choice.is_some_and(|c| c.model.efforts.contains(effort)))
}

impl DefaultGroup {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<ModelsPage>,
    ) -> (Self, Vec<Subscription>) {
        let model = cx.new(|cx| {
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
        let effort = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SelectOption<Option<ThinkingLevel>>>::new()),
                None,
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(&model, |page, _, event: &SelectEvent<ModelOptions>, cx| {
                let SelectEvent::Confirm(model) = event;
                page.select_model(model.clone().flatten(), cx);
            }),
            cx.subscribe(
                &effort,
                |page, _, event: &SelectEvent<EffortOptions>, cx| {
                    let SelectEvent::Confirm(effort) = event;
                    page.select_effort(effort.flatten(), cx);
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
}

impl ModelsPage {
    fn select_model(&mut self, model: Option<ModelRef>, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let choice = model.as_ref().and_then(|m| find(snapshot, m));
        let effort = kept_effort(snapshot.prefs.default_effort, choice);
        self.write(PreferenceWrite::Default { model, effort }, cx);
    }

    fn select_effort(&mut self, effort: Option<ThinkingLevel>, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let Some(choice) = selected(snapshot) else {
            return;
        };
        let model = Some(model_reference(choice));
        let effort = kept_effort(effort, Some(choice));
        self.write(PreferenceWrite::Default { model, effort }, cx);
    }

    fn sync_default_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        if !self.default_group.dirty {
            return;
        }
        self.default_group.dirty = false;
        let current = snapshot.prefs.default_model.clone();
        let mut options = vec![SelectOption {
            title: s::NONE.into(),
            value: None,
        }];
        // Hidden models are left out of the list, except the one currently chosen.
        options.extend(
            snapshot
                .catalog
                .iter()
                .filter(|c| {
                    let reference = model_reference(c);
                    !snapshot.prefs.hidden.contains(&reference)
                        || current.as_ref() == Some(&reference)
                })
                .map(|c| SelectOption {
                    title: s::model_label(&c.provider_name, &c.model.name, &c.model.id).into(),
                    value: Some(model_reference(c)),
                }),
        );
        let choice = selected(snapshot);
        let shown_model = choice.map(model_reference);
        let efforts = choice
            .map(|c| c.model.efforts.as_slice())
            .unwrap_or_default();
        let shown_effort = clamp_effort(snapshot.prefs.default_effort, efforts);
        let mut effort_options = vec![SelectOption {
            title: s::EFFORT_CLEAR.into(),
            value: None,
        }];
        effort_options.extend(efforts.iter().map(|effort| SelectOption {
            title: s::effort_label(*effort).into(),
            value: Some(*effort),
        }));
        self.default_group.model.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(options), window, cx);
            select.set_selected_value(&shown_model, window, cx);
        });
        self.default_group.effort.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(effort_options), window, cx);
            match shown_effort {
                Some(effort) => select.set_selected_value(&Some(effort), window, cx),
                None => select.set_selected_index(None, window, cx),
            }
        });
    }

    pub(super) fn default_group(
        &mut self,
        r: &Roles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.sync_default_selects(window, cx);
        let snapshot = self.snapshot.as_ref()?;
        let disabled = self.busy();
        let choice = selected(snapshot);
        let efforts = choice
            .map(|c| c.model.efforts.as_slice())
            .unwrap_or_default();
        // A selected effort shows as "力度 {档位}" through the prefix; with none selected the
        // placeholder says whether the model has levels at all.
        let effort_select = Select::new(&self.default_group.effort)
            .id("settings-default-effort")
            .w(px(120.))
            .title_prefix(s::EFFORT_PREFIX)
            .placeholder(if efforts.is_empty() {
                s::EFFORT_NONE
            } else {
                s::EFFORT_DEFAULT
            })
            .disabled(disabled || efforts.is_empty());
        let controls = h_flex()
            .gap(space::S8)
            .child(
                Select::new(&self.default_group.model)
                    .id("settings-default-model")
                    .w(px(220.))
                    .placeholder(s::NONE)
                    .search_placeholder(s::SEARCH)
                    .cleanable(true)
                    .disabled(disabled),
            )
            .child(effort_select);
        let warning = s::unavailable(&snapshot.default_status).map(|text| notice(text, true, r));
        let notes = snapshot
            .notes
            .iter()
            .map(|(backend, note)| notice(s::engine_note(*backend, note), false, r));
        Some(
            v_flex()
                .child(row(s::SESSION_MODEL, None, controls, r))
                .children(warning)
                .children(notes)
                .into_any_element(),
        )
    }
}
