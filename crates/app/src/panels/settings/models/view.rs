//! Models page rendering: default model and effort selectors, and the visible-models list.

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        select::{SearchableVec, Select},
        switch::Switch,
    },
    prelude::FluentBuilder as _,
};
use thinkrail_ui::{
    ThinkrailTheme,
    components::label,
    metrics::{space, text},
};

use super::{ModelsPage, PreferenceWrite, SelectOption, model_reference};
use crate::strings::settings_models as s;

impl ModelsPage {
    fn sync_selectors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selectors_dirty {
            return;
        }
        self.selectors_dirty = false;
        let mut options = vec![SelectOption {
            title: s::NONE.into(),
            value: None,
        }];
        options.extend(self.catalog.iter().map(|choice| SelectOption {
            title:
                s::model_label(&choice.provider_name, &choice.model.name, &choice.model.id).into(),
            value: Some(model_reference(choice)),
        }));
        let selected = self.selected_model();
        let model = selected.map(model_reference);
        let efforts = selected
            .map(|choice| choice.model.efforts.as_slice())
            .unwrap_or_default();
        let effort = self
            .prefs
            .default_effort
            .filter(|effort| efforts.contains(effort));
        let effort_options = efforts
            .iter()
            .map(|effort| SelectOption {
                title: s::effort_label(*effort).into(),
                value: *effort,
            })
            .collect::<Vec<_>>();
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
        let effort_disabled = disabled
            || self
                .selected_model()
                .is_none_or(|choice| choice.model.efforts.is_empty());
        let unavailable_default =
            self.prefs.default_model.is_some() && self.selected_model().is_none();
        let has_matches = self
            .all_models
            .iter()
            .any(|model| model.matches(&self.query));
        let rows = self
            .all_models
            .iter()
            .enumerate()
            .filter(|(_, model)| model.matches(&self.query))
            .map(|(index, model)| {
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
                            .child(
                                label(
                                    text::METADATA,
                                    s::model_label(
                                        &model.provider_name,
                                        &model.model.name,
                                        &model.model.model_id,
                                    ),
                                )
                                .text_color(roles.text_subtle),
                            )
                            .child(
                                label(
                                    text::METADATA,
                                    s::model_details(
                                        model.model.context_window,
                                        model.model.reasoning,
                                        model.model.vision,
                                    ),
                                )
                                .text_color(roles.text_subtle),
                            ),
                    )
                    .child(
                        Switch::new(format!("model-visibility-{index}"))
                            .checked(!hidden)
                            .color(roles.feedback_success)
                            .accessibility_label(model.model.name.clone())
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
            })
            .collect::<Vec<_>>();
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
                    .child(
                        Select::new(&self.default_model)
                            .id("settings-default-model")
                            .placeholder(s::NONE)
                            .search_placeholder(s::SEARCH)
                            .cleanable(true)
                            .disabled(disabled),
                    )
                    .when(unavailable_default, |view| {
                        view.child(
                            label(text::METADATA, s::UNAVAILABLE_DEFAULT)
                                .text_color(roles.text_subtle),
                        )
                    })
                    .child(label(text::TITLE_COMPACT, s::EFFORT))
                    .child(
                        Select::new(&self.default_effort)
                            .id("settings-default-effort")
                            .placeholder(if effort_disabled {
                                s::NO_EFFORT
                            } else {
                                s::DEFAULT_EFFORT
                            })
                            .cleanable(true)
                            .disabled(effort_disabled),
                    ),
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
                            .child(
                                Button::new("models-expand")
                                    .ghost()
                                    .label(if self.expanded { s::HIDE } else { s::SHOW })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.expanded = !this.expanded;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.expanded, |view| {
                        view.child(Input::new(&self.search)).children(rows).when(
                            self.loaded && !has_matches,
                            |view| {
                                view.child(
                                    label(
                                        text::METADATA,
                                        if self.all_models.is_empty() {
                                            s::EMPTY
                                        } else {
                                            s::NO_MATCHES
                                        },
                                    )
                                    .text_color(roles.text_subtle),
                                )
                            },
                        )
                    })
                    .child(
                        label(
                            text::METADATA,
                            s::visibility_summary(self.prefs.hidden.len()),
                        )
                        .text_color(roles.text_subtle),
                    ),
            )
            .when(busy, |view| {
                view.child(
                    label(
                        text::METADATA,
                        if self.saving { s::SAVING } else { s::LOADING },
                    )
                    .text_color(roles.text_subtle),
                )
            })
            .when_some(failure, |view, message| {
                view.child(
                    v_flex()
                        .gap(space::S8)
                        .child(label(text::METADATA, message).text_color(roles.feedback_error))
                        .child(
                            Button::new("models-retry")
                                .ghost()
                                .label(s::RETRY)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                        ),
                )
            })
            .when(self.loaded && self.catalog.is_empty(), |view| {
                view.child(label(text::METADATA, s::EMPTY).text_color(roles.text_subtle))
            })
    }
}
