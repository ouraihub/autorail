//! 手动添加模型与模型设置对话框（`AddModel`、`ModelSettings`）。

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    assets::IconName,
    base::h_flex,
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        input::{Input, InputState},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{
    DiscoveredModel, ModelType, Protocol, ProviderModel, ThinkingLevel, model_from_discovery,
};
use thinkrail_ui::{
    components::label,
    metrics::{space, text},
};

use super::Cap;
use crate::{
    panels::settings::providers::{ProvidersPage, error_text, open_form, theme},
    strings::{settings_models::effort_label, settings_providers as s},
};
use lumakit_gpui::providers as ui;

const EFFORTS: [ThinkingLevel; 7] = [
    ThinkingLevel::Off,
    ThinkingLevel::Minimal,
    ThinkingLevel::Low,
    ThinkingLevel::Medium,
    ThinkingLevel::High,
    ThinkingLevel::Xhigh,
    ThinkingLevel::Max,
];

struct AddModel {
    id: Entity<InputState>,
    saving: bool,
    error: Option<String>,
}

impl Render for AddModel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::form_fields([
            ui::field(s::MODEL_ID, Input::new(&self.id), &theme::roles(cx)).into_any_element(),
        ])
        .children(error_text(self.error.clone(), cx))
    }
}

struct ModelSettings {
    model: ProviderModel,
    lines: Vec<Protocol>,
    name: Entity<InputState>,
    context_window: Entity<InputState>,
    max_tokens: Entity<InputState>,
    error: Option<String>,
    saving: bool,
}

impl ProvidersPage {
    pub(super) fn open_add_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = cx.new(|cx| AddModel {
            id: cx.new(|cx| InputState::new(window, cx).placeholder(s::MODEL_ID)),
            saving: false,
            error: None,
        });
        let (page, line) = (cx.entity(), self.line);
        let Some(target) = self.selected.clone() else {
            return;
        };
        open_form(
            window,
            cx,
            s::ADD_MODEL,
            px(420.),
            body,
            s::CONFIRM,
            move |body, window, cx| {
                if body.saving {
                    return false;
                }
                let id = body.id.read(cx).value().trim().to_owned();
                if id.is_empty() {
                    return false;
                }
                let target = target.clone();
                body.saving = true;
                crate::panels::settings::providers::forms::save(
                    page.clone(),
                    window,
                    cx,
                    move |service| {
                        let provider = service.provider(&target)?;
                        let mut input = provider.input;
                        match input.models.iter_mut().find(|m| m.model_id == id) {
                            Some(model) if !model.protocols.contains(&line) => {
                                model.protocols.push(line)
                            }
                            Some(_) => {}
                            None => input.models.push(model_from_discovery(
                                &DiscoveredModel {
                                    id,
                                    name: None,
                                    context_window: None,
                                },
                                line,
                            )),
                        }
                        service.update(&target, provider.revision, input)
                    },
                    false,
                    |body, error| {
                        body.saving = false;
                        body.error = Some(error);
                    },
                );
                false
            },
        );
    }

    pub(super) fn open_model_settings(
        &mut self,
        model_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        let Some(model) = draft
            .models
            .iter()
            .find(|m| m.model_id == model_id)
            .cloned()
        else {
            return;
        };
        let lines = draft.endpoints.iter().map(|e| e.protocol).collect();
        let number = |value: Option<u32>| value.map(|v| v.to_string()).unwrap_or_default();
        let body = cx.new(|cx| {
            let mut input =
                |value: String| cx.new(|cx| InputState::new(window, cx).default_value(value));
            ModelSettings {
                name: input(model.name.clone()),
                context_window: input(number(model.context_window)),
                max_tokens: input(number(model.max_tokens)),
                model,
                lines,
                error: None,
                saving: false,
            }
        });
        let page = cx.entity();
        let Some(target) = self.selected.clone() else {
            return;
        };
        open_form(
            window,
            cx,
            s::EDIT_MODEL,
            px(520.),
            body,
            s::SAVE,
            move |view, window, cx| {
                if view.saving {
                    return false;
                }
                match view.result(cx) {
                    Ok(model) => {
                        let target = target.clone();
                        view.saving = true;
                        crate::panels::settings::providers::forms::save(
                            page.clone(),
                            window,
                            cx,
                            move |service| {
                                let provider = service.provider(&target)?;
                                let mut input = provider.input;
                                let slot = input
                                    .models
                                    .iter_mut()
                                    .find(|m| m.model_id == model.model_id)
                                    .ok_or_else(|| {
                                        thinkrail_core::Error::not_found("模型已移除")
                                    })?;
                                *slot = model;
                                service.update(&target, provider.revision, input)
                            },
                            false,
                            |view, error| {
                                view.saving = false;
                                view.error = Some(error);
                            },
                        );
                        false
                    }
                    Err(error) => {
                        view.error = Some(error.into());
                        cx.notify();
                        false
                    }
                }
            },
        );
    }
}

impl ModelSettings {
    fn result(&self, cx: &gpui_kit::App) -> Result<ProviderModel, &'static str> {
        let number = |input: &Entity<InputState>| {
            let raw = input.read(cx).value().trim().to_owned();
            if raw.is_empty() {
                Ok(None)
            } else {
                raw.parse::<u32>()
                    .ok()
                    .filter(|v| *v > 0)
                    .map(Some)
                    .ok_or(s::NUMBER_INVALID)
            }
        };
        let mut model = self.model.clone();
        let name = self.name.read(cx).value().trim().to_owned();
        model.name = if name.is_empty() {
            model.model_id.clone()
        } else {
            name
        };
        model.context_window = number(&self.context_window)?;
        model.max_tokens = number(&self.max_tokens)?;
        if model.protocols.is_empty() {
            return Err(s::MODEL_LINE_REQUIRED);
        }
        if !model.reasoning {
            model.efforts.clear();
        } else if model.efforts.is_empty() {
            return Err(s::EFFORTS_REQUIRED);
        }
        Ok(model)
    }
}

impl Render for ModelSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = theme::roles(cx);
        let type_menu = self.type_menu(cx);
        let lines = h_flex()
            .gap(space::S12)
            .children(self.lines.iter().map(|&protocol| {
                let title = if self.model.protocols.first() == Some(&protocol) {
                    format!("{} · {}", s::protocol_short(protocol), s::DEFAULT_BADGE)
                } else {
                    s::protocol_short(protocol).to_owned()
                };
                Checkbox::new(format!("model-line-{protocol:?}"))
                    .label(title)
                    .checked(self.model.protocols.contains(&protocol))
                    .on_click(cx.listener(move |this, value: &bool, _, cx| {
                        this.model.protocols.retain(|p| *p != protocol);
                        if *value {
                            this.model.protocols.push(protocol);
                        }
                        cx.notify();
                    }))
            }));
        let caps =
            h_flex()
                .gap(space::S12)
                .children([Cap::Vision, Cap::Reasoning, Cap::Tools].map(|cap| {
                    Checkbox::new(format!("model-cap-{cap:?}"))
                        .label(cap.title())
                        .checked(cap.has(&self.model))
                        .on_click(cx.listener(move |this, value: &bool, _, cx| {
                            let m = &mut this.model;
                            match cap {
                                Cap::Vision => m.vision = *value,
                                Cap::Reasoning => m.reasoning = *value,
                                Cap::Tools => m.tools = *value,
                            }
                            cx.notify();
                        }))
                }));
        let efforts = self.model.reasoning.then(|| {
            ui::field(
                s::EFFORTS,
                h_flex()
                    .gap(space::S4)
                    .flex_wrap()
                    .children(EFFORTS.map(|effort| {
                        let on = self.model.efforts.contains(&effort);
                        Button::new(format!("model-effort-{effort:?}"))
                            .small()
                            .label(effort_label(effort))
                            .when(on, |b| b.primary())
                            .when(!on, |b| b.outline())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let efforts = &mut this.model.efforts;
                                if efforts.contains(&effort) {
                                    efforts.retain(|e| *e != effort)
                                } else {
                                    efforts.push(effort);
                                    efforts.sort();
                                }
                                cx.notify();
                            }))
                    })),
                &r,
            )
        });
        ui::form_fields([])
            .child(label(text::CODE_TEXT, self.model.model_id.clone()).text_color(r.text_muted))
            .child(
                h_flex()
                    .gap(space::S12)
                    .items_end()
                    .child(div().flex_1().child(ui::field(
                        s::MODEL_NAME,
                        Input::new(&self.name),
                        &r,
                    )))
                    .child(ui::field(s::MODEL_TYPE, type_menu, &r)),
            )
            .child(ui::field(s::MODEL_PROTOCOLS, lines, &r))
            .child(
                h_flex()
                    .gap(space::S12)
                    .child(div().flex_1().child(ui::field(
                        s::CONTEXT_WINDOW,
                        Input::new(&self.context_window),
                        &r,
                    )))
                    .child(div().flex_1().child(ui::field(
                        s::MAX_OUTPUT,
                        Input::new(&self.max_tokens),
                        &r,
                    ))),
            )
            .child(ui::field(s::CAPABILITIES, caps, &r))
            .children(efforts)
            .children(error_text(self.error.clone(), cx))
            .child(label(text::METADATA, s::APPLIES_NEXT).text_color(r.text_subtle))
    }
}

impl ModelSettings {
    fn type_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (kind, entity) = (self.model.model_type, cx.entity());
        Button::new("model-type")
            .outline()
            .label(s::model_type(kind))
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                ModelType::ALL.into_iter().fold(menu, |menu, value| {
                    let entity = entity.clone();
                    menu.item(
                        PopupMenuItem::new(s::model_type(value))
                            .checked(kind == value)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.model.model_type = value;
                                    cx.notify();
                                })
                            }),
                    )
                })
            })
    }
}
