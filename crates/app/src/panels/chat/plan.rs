//! The chat's TODO strip with review status projection.
use std::collections::HashMap;

use gpui_kit::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::Icon,
    prelude::FluentBuilder as _,
};
use thinkrail_core::{Plan, PlanItem, PlanItemId, PlanStatus, ReviewEvent, ReviewOutcome};
use thinkrail_ui::{
    Roles,
    components::label,
    metrics::{size, space, text},
};

use super::transcript::Toggle;
use crate::strings::chat as s;

pub fn strip(
    key: &str,
    plan: &Plan,
    reviews: &HashMap<PlanItemId, ReviewEvent>,
    open: bool,
    toggle: &Toggle,
    r: &Roles,
) -> Option<AnyElement> {
    let items: Vec<&PlanItem> = plan
        .groups
        .iter()
        .flat_map(|g| g.items.iter())
        .chain(plan.loose.iter())
        .collect();
    if items.is_empty() {
        return None;
    }
    let done = items
        .iter()
        .filter(|i| i.status == PlanStatus::Done)
        .count();
    let toggle = toggle.clone();
    let key_owned = key.to_owned();
    let bar = h_flex()
        .id("chat-plan-strip")
        .h(size::PANEL_HEADER)
        .flex_none()
        .px(size::PANEL_PADDING)
        .gap(space::S6)
        .cursor_pointer()
        .text_color(r.text_muted)
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .size(size::ICON_SMALL),
        )
        .child(label(text::METADATA, s::PLAN).text_color(r.text_default))
        .child(label(text::METADATA, format!("{done}/{}", items.len())))
        .on_click(move |_, window, cx| toggle(key_owned.clone(), window, cx));
    Some(
        v_flex()
            .flex_none()
            .border_b_1()
            .border_color(r.border_muted)
            .child(bar)
            .when(open, |d| {
                d.child(
                    v_flex()
                        .px(size::PANEL_PADDING + space::S16)
                        .pb(space::S8)
                        .gap(space::S4)
                        .children(plan.groups.iter().filter(|g| !g.items.is_empty()).map(|g| {
                            v_flex()
                                .gap(space::S4)
                                .child(
                                    label(text::METADATA, g.title.clone())
                                        .text_color(r.text_subtle),
                                )
                                .children(g.items.iter().map(|i| row(i, reviews.get(&i.id), r)))
                        }))
                        .children(plan.loose.iter().map(|i| row(i, reviews.get(&i.id), r))),
                )
            })
            .into_any_element(),
    )
}

fn row(item: &PlanItem, review: Option<&ReviewEvent>, r: &Roles) -> AnyElement {
    let (icon, color) = match item.status {
        PlanStatus::Pending => (IconName::Circle, r.text_subtle),
        PlanStatus::InProgress => (IconName::LoaderCircle, r.feedback_info),
        PlanStatus::Done => (IconName::CircleCheck, r.feedback_success),
    };
    let title = label(text::METADATA, item.title.clone()).truncate();
    let status = review.map(|event| match event {
        ReviewEvent::Started { .. } => s::REVIEWING,
        ReviewEvent::Finished { result, .. } => match result.outcome {
            ReviewOutcome::Approved => s::REVIEW_APPROVED,
            ReviewOutcome::ChangesFix
            | ReviewOutcome::ChangesStop
            | ReviewOutcome::ApproveBlocked { .. } => s::REVIEW_CHANGES,
            ReviewOutcome::Failed { .. } => s::REVIEW_FAILED,
        },
    });
    h_flex()
        .gap(space::S6)
        .child(Icon::new(icon).size(size::ICON_SMALL).text_color(color))
        .child(match item.status {
            PlanStatus::Done => title.text_color(r.text_subtle).line_through(),
            PlanStatus::InProgress => title.text_color(r.text_default),
            PlanStatus::Pending => title.text_color(r.text_muted),
        })
        .when_some(status, |row, status| {
            row.child(label(text::METADATA, status).text_color(r.text_subtle))
        })
        .into_any_element()
}
