//! 评审 group (agent-settings 08 §5.1): 评审模型 with its effort, then the Agent 触发的评审 and
//! 自动修复循环 switches. No pinned model means the review follows the default chat model, so the
//! first entry reads "{默认模型名}（默认）".

use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Subscription, Window,
    base::v_flex,
    component::{Disableable as _, switch::Switch},
};
use thinkrail_core::{ModelChoice, ModelRef, ReviewSwitch, ThinkingLevel};
use thinkrail_ui::Roles;

use super::{
    super::rows::{notice, row},
    ModelsPage, PreferenceWrite, Snapshot, default_group,
    model_select::{ModelEffortSelects, PairState, find, kept_effort},
};
use crate::strings::settings_models as s;

/// The model a review runs on when it starts now: the pinned one, else the default chat model.
fn effective(snapshot: &Snapshot) -> Option<&ModelChoice> {
    match &snapshot.prefs.review.model {
        Some(model) => find(snapshot, model),
        None => default_group::selected(snapshot),
    }
}

fn pair(snapshot: &Snapshot) -> PairState<'_> {
    PairState {
        saved: snapshot.prefs.review.model.as_ref(),
        none_title: s::review_follows(
            default_group::selected(snapshot).map(|c| c.model.name.as_str()),
        ),
        effort_model: effective(snapshot),
        saved_effort: snapshot.prefs.review.effort,
    }
}

pub(super) fn new(
    window: &mut Window,
    cx: &mut Context<ModelsPage>,
) -> (ModelEffortSelects, Vec<Subscription>) {
    ModelEffortSelects::new(
        window,
        cx,
        ModelsPage::select_review_model,
        ModelsPage::select_review_effort,
    )
}

impl ModelsPage {
    /// The default entry clears both, as in the original; a model keeps the effort only where it
    /// declares it.
    fn select_review_model(&mut self, model: Option<ModelRef>, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let effort = match &model {
            None => None,
            Some(m) => kept_effort(snapshot.prefs.review.effort, find(snapshot, m)),
        };
        self.write(PreferenceWrite::Review { model, effort }, cx);
    }

    fn select_review_effort(&mut self, effort: Option<ThinkingLevel>, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let model = snapshot.prefs.review.model.clone();
        let effort = kept_effort(effort, effective(snapshot));
        self.write(PreferenceWrite::Review { model, effort }, cx);
    }

    fn review_switch(
        &self,
        id: &'static str,
        switch: ReviewSwitch,
        on: bool,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        Switch::new(id)
            .checked(on)
            .accessibility_label(label)
            .disabled(self.busy())
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                this.write(PreferenceWrite::Switch(switch, *checked), cx);
            }))
    }

    pub(super) fn review_group(
        &mut self,
        r: &Roles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let snapshot = self.snapshot.as_ref()?;
        let state = pair(snapshot);
        self.review_group.sync(snapshot, &state, window, cx);
        let controls = self
            .review_group
            .render("settings-review", &state, self.busy());
        let warning = s::review_unavailable(&snapshot.review_status).map(|t| notice(t, true, r));
        let review = snapshot.prefs.review.clone();
        let agent = self.review_switch(
            "settings-agent-review",
            ReviewSwitch::AgentReview,
            review.agent_review,
            s::AGENT_REVIEW,
            cx,
        );
        let auto_fix = self.review_switch(
            "settings-review-auto-fix",
            ReviewSwitch::AutoFix,
            review.auto_fix,
            s::AUTO_FIX,
            cx,
        );
        Some(
            v_flex()
                .child(row(s::REVIEW_MODEL, None, controls, r))
                .children(warning)
                .child(row(
                    s::AGENT_REVIEW,
                    Some(s::AGENT_REVIEW_DESC.to_owned()),
                    agent,
                    r,
                ))
                .child(row(
                    s::AUTO_FIX,
                    Some(s::AUTO_FIX_DESC.to_owned()),
                    auto_fix,
                    r,
                ))
                .into_any_element(),
        )
    }
}
