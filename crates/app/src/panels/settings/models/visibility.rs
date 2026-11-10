//! 可见模型 group: a disclosure row that opens in place (not a sub-page) to a search field and
//! one switch per model. Only hidden models are stored, so new models appear on their own.

use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    base::{h_flex, v_flex},
    component::{Disableable as _, input::Input, scroll::ScrollableElement as _, switch::Switch},
    px,
};
use thinkrail_ui::{
    Roles,
    components::label,
    metrics::{space, text},
};

use super::{super::rows::disclosure, ModelsPage, PreferenceWrite};
use crate::strings::settings_models as s;

impl ModelsPage {
    pub(super) fn visibility_group(&self, r: &Roles, cx: &mut Context<Self>) -> Option<AnyElement> {
        let snapshot = self.snapshot.as_ref()?;
        let hidden = snapshot
            .all_models
            .iter()
            .filter(|m| snapshot.prefs.hidden.contains(&m.reference))
            .count();
        let header = disclosure(
            "models-visible",
            s::VISIBLE,
            s::visibility_summary(hidden),
            self.expanded,
            r,
            {
                let page = cx.entity().downgrade();
                move |_, cx| {
                    if page
                        .update(cx, |page, cx| {
                            page.expanded = !page.expanded;
                            cx.notify();
                        })
                        .is_err()
                    {
                        tracing::debug!("models.visibility.view_closed");
                    }
                }
            },
        );
        if !self.expanded {
            return Some(header);
        }
        let disabled = self.busy();
        let rows = snapshot
            .all_models
            .iter()
            .enumerate()
            .filter(|(_, m)| m.matches(&self.query))
            .map(|(index, m)| {
                let is_hidden = snapshot.prefs.hidden.contains(&m.reference);
                let reference = m.reference.clone();
                h_flex()
                    .gap(space::S8)
                    .py(space::S8)
                    .border_b_1()
                    .border_color(r.border_muted)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                label(text::UI, m.model.name.clone()).text_color(if is_hidden {
                                    r.text_subtle
                                } else {
                                    r.text_default
                                }),
                            )
                            .child(
                                label(
                                    text::METADATA,
                                    s::model_meta(
                                        &m.provider_name,
                                        m.model.context_window,
                                        m.model.reasoning,
                                    ),
                                )
                                .text_color(r.text_subtle),
                            ),
                    )
                    .child(
                        Switch::new(format!("model-visibility-{index}"))
                            .checked(!is_hidden)
                            .color(r.feedback_success)
                            .accessibility_label(m.model.name.clone())
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.write(
                                    PreferenceWrite::Hidden {
                                        model: reference.clone(),
                                        hidden: !*checked,
                                    },
                                    cx,
                                );
                            })),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let empty = rows.is_empty().then(|| {
            label(
                text::METADATA,
                if snapshot.all_models.is_empty() {
                    s::EMPTY
                } else {
                    s::NO_MATCHES
                },
            )
            .text_color(r.text_subtle)
            .py(space::S8)
        });
        Some(
            v_flex()
                .child(header)
                .child(
                    v_flex()
                        .pt(space::S8)
                        .gap(space::S8)
                        .child(Input::new(&self.search))
                        .child(
                            v_flex()
                                .id("models-visible-list")
                                .max_h(px(320.))
                                .overflow_y_scrollbar()
                                .children(rows)
                                .children(empty),
                        ),
                )
                .into_any_element(),
        )
    }
}
