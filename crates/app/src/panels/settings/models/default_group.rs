//! 默认模型 group: the 会话模型 row (model and effort selects on one line), the unavailable-default
//! notice and one note per engine whose 编码搭档 choice bypasses these settings (08 §4.1–4.4).

use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Subscription, Window, base::v_flex,
};
use thinkrail_core::{ModelChoice, ModelRef, ThinkingLevel};
use thinkrail_ui::Roles;

use super::{
    super::rows::{notice, row},
    ModelsPage, PreferenceWrite, Snapshot, model_reference,
    model_select::{ModelEffortSelects, PairState, find, kept_effort},
};
use crate::strings::settings_models as s;

/// The saved default's catalog entry, if it is routable now.
pub(super) fn selected(snapshot: &Snapshot) -> Option<&ModelChoice> {
    find(snapshot, snapshot.prefs.default_model.as_ref()?)
}

fn pair(snapshot: &Snapshot) -> PairState<'_> {
    PairState {
        saved: snapshot.prefs.default_model.as_ref(),
        none_title: s::NONE.to_owned(),
        effort_model: selected(snapshot),
        saved_effort: snapshot.prefs.default_effort,
    }
}

pub(super) fn new(
    window: &mut Window,
    cx: &mut Context<ModelsPage>,
) -> (ModelEffortSelects, Vec<Subscription>) {
    ModelEffortSelects::new(
        window,
        cx,
        ModelsPage::select_default_model,
        ModelsPage::select_default_effort,
    )
}

impl ModelsPage {
    fn select_default_model(&mut self, model: Option<ModelRef>, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let choice = model.as_ref().and_then(|m| find(snapshot, m));
        let effort = kept_effort(snapshot.prefs.default_effort, choice);
        self.write(PreferenceWrite::Default { model, effort }, cx);
    }

    fn select_default_effort(&mut self, effort: Option<ThinkingLevel>, cx: &mut Context<Self>) {
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

    pub(super) fn default_group(
        &mut self,
        r: &Roles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let snapshot = self.snapshot.as_ref()?;
        let state = pair(snapshot);
        self.default_group.sync(snapshot, &state, window, cx);
        let controls = self
            .default_group
            .render("settings-default", &state, self.busy());
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
