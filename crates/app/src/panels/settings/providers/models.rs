//! 模型区：按组列出当前线路的模型；同步模型（挑选对话框）、手动添加、模型设置、移除。

use std::collections::{BTreeMap, HashSet};

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputState},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{ModelType, Protocol, ProviderInput, ProviderModel, line_models, model_group};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, label},
    metrics::{radius, space, text},
};

use super::{ProvidersPage, avatar, error_text};
use crate::strings::settings_providers as s;

mod editor;
mod picker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cap {
    Vision,
    Reasoning,
    Tools,
}

impl Cap {
    fn has(self, model: &ProviderModel) -> bool {
        match self {
            Cap::Vision => model.vision,
            Cap::Reasoning => model.reasoning,
            Cap::Tools => model.tools,
        }
    }
    fn title(self) -> &'static str {
        match self {
            Cap::Vision => s::CAP_VISION,
            Cap::Reasoning => s::CAP_REASONING,
            Cap::Tools => s::CAP_TOOLS,
        }
    }
    fn icon(self) -> IconName {
        match self {
            Cap::Vision => IconName::Eye,
            Cap::Reasoning => IconName::Brain,
            Cap::Tools => IconName::Wrench,
        }
    }
}

/// Model-section view state owned by the page (search, capability filter, folded groups).
pub(super) struct ModelView {
    pub(super) query: Entity<InputState>,
    cap: Option<Cap>,
    folded: HashSet<String>,
    syncing: bool,
    sync_error: Option<String>,
}

impl ModelView {
    pub(super) fn new(window: &mut Window, cx: &mut Context<ProvidersPage>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(s::MODEL_SEARCH));
        Self {
            query,
            cap: None,
            folded: HashSet::new(),
            syncing: false,
            sync_error: None,
        }
    }

    pub(super) fn reset(&mut self) {
        self.cap = None;
        self.folded.clear();
        self.syncing = false;
        self.sync_error = None;
    }
}

/// Takes the models matching `remove` off the line `protocol`; a model left on no line is dropped.
fn drop_from_line(
    draft: &mut ProviderInput,
    protocol: Protocol,
    remove: impl Fn(&ProviderModel) -> bool,
) {
    draft.models.retain_mut(|model| {
        if remove(model) {
            model.protocols.retain(|p| *p != protocol);
        }
        !model.protocols.is_empty()
    });
}

fn group_name(model_id: &str) -> String {
    let group = model_group(model_id);
    if group.is_empty() {
        s::NO_GROUP.to_owned()
    } else {
        group.to_owned()
    }
}

impl ProvidersPage {
    pub(super) fn render_models(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let Some(draft) = self.draft.as_ref() else {
            return div().into_any_element();
        };
        let line = self.line;
        let query = self.models.query.read(cx).value().trim().to_lowercase();
        let cap = self.models.cap;
        let total = line_models(draft, line).count();
        let mut groups: BTreeMap<String, Vec<ProviderModel>> = BTreeMap::new();
        for (_, model) in line_models(draft, line) {
            let hit = (query.is_empty()
                || model.model_id.to_lowercase().contains(&query)
                || model.name.to_lowercase().contains(&query))
                && cap.is_none_or(|c| c.has(model));
            if hit {
                groups
                    .entry(group_name(&model.model_id))
                    .or_default()
                    .push(model.clone());
            }
        }
        let all_folded =
            !groups.is_empty() && groups.keys().all(|g| self.models.folded.contains(g));
        let names: Vec<String> = groups.keys().cloned().collect();
        let page = cx.entity();
        let cap_menu = Button::new("models-cap")
            .ghost()
            .small()
            .label(cap.map_or(s::ALL_TYPES, Cap::title))
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                [
                    None,
                    Some(Cap::Vision),
                    Some(Cap::Reasoning),
                    Some(Cap::Tools),
                ]
                .into_iter()
                .fold(menu, |menu, value| {
                    let page = page.clone();
                    menu.item(
                        PopupMenuItem::new(value.map_or(s::ALL_TYPES, Cap::title))
                            .checked(cap == value)
                            .on_click(move |_, _, cx| {
                                page.update(cx, |this, cx| {
                                    this.models.cap = value;
                                    cx.notify();
                                })
                            }),
                    )
                })
            });
        let header = h_flex()
            .gap(space::S4)
            .child(label(text::TITLE_SECTION, s::MODELS).mr(space::S4))
            .child(
                IconButton::new(
                    "models-fold",
                    if all_folded {
                        IconName::ChevronsUpDown
                    } else {
                        IconName::ChevronsDownUp
                    },
                )
                .tooltip(if all_folded {
                    s::EXPAND_ALL
                } else {
                    s::COLLAPSE_ALL
                })
                .disabled(names.is_empty())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if all_folded {
                        this.models.folded.clear()
                    } else {
                        this.models.folded.extend(names.iter().cloned())
                    }
                    cx.notify();
                })),
            )
            .child(
                Input::new(&self.models.query)
                    .small()
                    .prefix(Icon::new(IconName::Search).small())
                    .w(px(160.)),
            )
            .child(cap_menu)
            .child(div().flex_1())
            .child(
                Button::new("models-sync")
                    .outline()
                    .icon(IconName::RefreshCw)
                    .label(s::SYNC_MODELS)
                    .disabled(self.models.syncing || self.saving || self.dirty)
                    .on_click(cx.listener(|this, _, window, cx| this.sync_models(window, cx))),
            )
            .child(
                Button::new("models-add")
                    .outline()
                    .icon(IconName::Plus)
                    .tooltip(s::ADD_MODEL)
                    .on_click(cx.listener(|this, _, window, cx| this.open_add_model(window, cx))),
            );
        let body: AnyElement = if groups.is_empty() {
            div()
                .py(space::S24)
                .rounded(radius::MD)
                .border_1()
                .border_dashed()
                .border_color(r.border_default)
                .flex()
                .justify_center()
                .child(
                    label(
                        text::METADATA,
                        if total == 0 {
                            s::NO_MODELS
                        } else {
                            s::NO_MODEL_MATCHES
                        },
                    )
                    .text_color(r.text_subtle),
                )
                .into_any_element()
        } else {
            v_flex()
                .rounded(radius::LG)
                .border_1()
                .border_color(r.border_muted)
                .overflow_hidden()
                .children(
                    groups
                        .into_iter()
                        .map(|(group, models)| self.render_group(group, models, cx)),
                )
                .into_any_element()
        };
        v_flex()
            .gap(space::S8)
            .child(header)
            .children(error_text(self.models.sync_error.clone(), cx))
            .child(body)
            .into_any_element()
    }

    fn render_group(
        &self,
        group: String,
        models: Vec<ProviderModel>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let folded = self.models.folded.contains(&group);
        let line = self.line;
        let ids: HashSet<String> = models.iter().map(|m| m.model_id.clone()).collect();
        let toggle = group.clone();
        v_flex()
            .child(
                h_flex()
                    .id(format!("model-group-{group}"))
                    .h(px(40.))
                    .px(space::S12)
                    .gap(space::S8)
                    .bg(r.container_elevated_bg)
                    .border_b_1()
                    .border_color(r.border_muted)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.models.folded.remove(&toggle) {
                            this.models.folded.insert(toggle.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        Icon::new(if folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .small(),
                    )
                    .child(label(text::UI, group.clone()))
                    .child(
                        label(text::METADATA, models.len().to_string()).text_color(r.text_subtle),
                    )
                    .child(div().flex_1())
                    .child(
                        IconButton::new(format!("model-group-remove-{group}"), IconName::Minus)
                            .tooltip(s::REMOVE_GROUP)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let ids = ids.clone();
                                this.edit(cx, |d| {
                                    drop_from_line(d, line, |m| ids.contains(&m.model_id))
                                });
                            })),
                    ),
            )
            .when(!folded, |d| {
                d.children(models.iter().map(|model| self.render_model(model, cx)))
            })
            .into_any_element()
    }

    fn render_model(&self, model: &ProviderModel, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let line = self.line;
        let (id, edit_id) = (model.model_id.clone(), model.model_id.clone());
        let caps = [Cap::Vision, Cap::Reasoning, Cap::Tools]
            .into_iter()
            .filter(|c| c.has(model))
            .map(|c| {
                let color = match c {
                    Cap::Vision => r.feedback_success,
                    Cap::Reasoning => r.feedback_info,
                    Cap::Tools => r.feedback_warning,
                };
                div()
                    .size(px(24.))
                    .rounded(radius::MD)
                    .bg(color.opacity(0.15))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(c.icon()).xsmall().text_color(color))
            });
        h_flex()
            .h(px(44.))
            .px(space::S12)
            .gap(space::S8)
            .border_b_1()
            .border_color(r.border_muted)
            .when(!model.enabled, |d| d.opacity(0.6))
            .child(avatar(&model.model_id, cx))
            .child(
                label(text::UI, model.model_id.clone())
                    .flex_1()
                    .min_w(px(0.))
                    .truncate(),
            )
            .when(model.model_type != ModelType::Chat, |d| {
                d.child(
                    label(text::METADATA, s::model_type(model.model_type))
                        .text_color(r.text_subtle),
                )
            })
            .child(h_flex().gap(space::S4).children(caps))
            .child(
                IconButton::new(format!("model-settings-{id}"), IconName::Settings2)
                    .tooltip(s::MODEL_SETTINGS)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_model_settings(edit_id.clone(), window, cx)
                    })),
            )
            .child(
                IconButton::new(format!("model-remove-{id}"), IconName::Minus)
                    .tooltip(s::REMOVE_MODEL)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let id = id.clone();
                        this.edit(cx, |d| drop_from_line(d, line, |m| m.model_id == id));
                    })),
            )
            .into_any_element()
    }
}
