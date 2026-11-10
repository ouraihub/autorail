//! Discovered-model selection, Native type metadata and captured-provider writes.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Icon, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputEvent, InputState},
        menu::{DropdownMenu as _, PopupMenuItem},
        scroll::ScrollableElement as _,
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{
    DiscoveredModel, ModelType, Protocol, ProviderModel, merge_picked, model_hints, model_type,
};
use thinkrail_ui::{
    components::label,
    metrics::{space, text},
};

use super::super::theme;
use super::group_name;
use crate::{
    panels::settings::providers::ProvidersPage, state::run, strings::settings_providers as s,
};
use lumakit_gpui::providers as ui;

/// "同步模型" picker (Aether `ModelPickerDialog`): every + / − saves at once; models the probe
/// did not list are left alone by `merge_picked`.
struct ModelPicker {
    page: Entity<ProvidersPage>,
    provider: thinkrail_core::ProviderId,
    saving: bool,
    error: Option<String>,
    saved_picked: BTreeSet<String>,
    saved_types: BTreeMap<String, ModelType>,
    line: Protocol,
    found: Vec<DiscoveredModel>,
    picked: BTreeSet<String>,
    /// Type per listed id: the stored one for models already present, else inferred.
    types: BTreeMap<String, ModelType>,
    query: Entity<InputState>,
    filter: Option<ModelType>,
    folded: HashSet<String>,
    _subscription: Subscription,
}

pub(super) fn open_picker(
    page: Entity<ProvidersPage>,
    provider: thinkrail_core::ProviderId,
    found: Vec<DiscoveredModel>,
    draft: Vec<ProviderModel>,
    line: Protocol,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let picked: BTreeSet<String> = draft
        .iter()
        .filter(|m| m.protocols.contains(&line))
        .map(|m| m.model_id.clone())
        .collect();
    let types: BTreeMap<String, ModelType> = found
        .iter()
        .map(|f| {
            let kind = draft
                .iter()
                .find(|m| m.model_id == f.id)
                .map_or_else(|| model_type(&f.id), |m| m.model_type);
            (f.id.clone(), kind)
        })
        .collect();
    let total = found.len();
    let picker = cx.new(|cx| {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(s::SEARCH_MODELS));
        let subscription = cx.subscribe(&query, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify()
            }
        });
        ModelPicker {
            page,
            provider,
            saving: false,
            error: None,
            saved_picked: picked.clone(),
            saved_types: types.clone(),
            line,
            found,
            picked,
            types,
            query,
            filter: None,
            folded: HashSet::new(),
            _subscription: subscription,
        }
    });
    window.open_dialog(cx, move |dialog, _, cx| {
        let footer = ui::form_footer(
            None,
            Button::new("picker-close")
                .outline()
                .label(s::CLOSE)
                .on_click(|_, window, cx| window.close_dialog(cx))
                .into_any_element(),
            None,
        );
        dialog
            .w(px(640.))
            .on_ok(|_, _, _| false)
            .child(ui::form_body(
                format!("{} {total}", s::PICKER_TITLE),
                picker.clone().into_any_element(),
                footer.into_any_element(),
                None,
                &theme::roles(cx),
            ))
    });
}

impl ModelPicker {
    /// Writes the captured provider and rolls visible selection back on failure.
    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.error = None;
        let (found, picked, types, line, id) = (
            self.found.clone(),
            self.picked.clone(),
            self.types.clone(),
            self.line,
            self.provider.clone(),
        );
        let service = self.page.read(cx).service(cx);
        run(
            cx,
            move || {
                let provider = service.provider(&id)?;
                let mut input = provider.input;
                input.models = merge_picked(&input.models, &found, &picked, line);
                for model in input
                    .models
                    .iter_mut()
                    .filter(|m| picked.contains(&m.model_id))
                {
                    if let Some(&kind) = types.get(&model.model_id) {
                        model.model_type = kind;
                    }
                }
                service.update(&id, provider.revision, input)
            },
            |this, result, cx| {
                this.saving = false;
                match result {
                    Ok(provider) => {
                        this.saved_picked = this.picked.clone();
                        this.saved_types = this.types.clone();
                        this.page.update(cx, |page, cx| page.stored(provider, cx));
                    }
                    Err(error) => {
                        this.picked = this.saved_picked.clone();
                        this.types = this.saved_types.clone();
                        this.error = Some(error.to_string());
                        this.page.update(cx, |page, cx| page.report(&error, cx));
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn kind(&self, id: &str) -> ModelType {
        self.types.get(id).copied().unwrap_or(ModelType::Unknown)
    }
}

impl Render for ModelPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = theme::roles(cx);
        let query = self.query.read(cx).value().trim().to_lowercase();
        let searched: Vec<DiscoveredModel> = self
            .found
            .iter()
            .filter(|m| m.id.to_lowercase().contains(&query))
            .cloned()
            .collect();
        let shown: Vec<DiscoveredModel> = searched
            .iter()
            .filter(|m| self.filter.is_none_or(|f| self.kind(&m.id) == f))
            .cloned()
            .collect();
        let ids: Vec<String> = shown.iter().map(|m| m.id.clone()).collect();
        let all_picked = !ids.is_empty() && ids.iter().all(|id| self.picked.contains(id));
        let tabs = std::iter::once(None)
            .chain(ModelType::ALL.map(Some))
            .map(|value| {
                let hits = searched
                    .iter()
                    .filter(|m| value.is_none_or(|t| self.kind(&m.id) == t))
                    .count();
                let title = value.map_or(s::ALL_TYPES, s::model_type);
                Button::new(format!("picker-tab-{value:?}"))
                    .small()
                    .label(format!("{title} {hits}"))
                    .when(self.filter == value, |b| b.outline())
                    .when(self.filter != value, |b| b.ghost())
                    .disabled(hits == 0 && value.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = value;
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let mut groups: BTreeMap<String, Vec<DiscoveredModel>> = BTreeMap::new();
        for model in shown {
            groups.entry(group_name(&model.id)).or_default().push(model);
        }
        let searching = !query.is_empty();
        let list = groups
            .into_iter()
            .map(|(group, models)| {
                let folded = !searching && self.folded.contains(&group);
                let toggle = group.clone();
                v_flex()
                    .child(ui::model_group(
                        Button::new(format!("picker-group-{group}"))
                            .ghost()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap(space::S8)
                                    .child(
                                        Icon::new(if folded {
                                            IconName::ChevronRight
                                        } else {
                                            IconName::ChevronDown
                                        })
                                        .small(),
                                    )
                                    .child(label(text::UI, group.clone()).truncate())
                                    .child(
                                        label(text::METADATA, models.len().to_string())
                                            .text_color(r.text_subtle),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.folded.remove(&toggle) {
                                    this.folded.insert(toggle.clone());
                                }
                                cx.notify();
                            }))
                            .into_any_element(),
                        gpui_kit::div().into_any_element(),
                        true,
                        &r,
                    ))
                    .when(!folded, |d| {
                        d.children(models.into_iter().map(|model| self.render_pick(model, cx)))
                    })
            })
            .collect::<Vec<_>>();
        ui::form_fields([])
            .children(super::super::error_text(self.error.clone(), cx))
            .child(label(text::METADATA, s::PICKER_HINT).text_color(r.text_subtle))
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(
                        Input::new(&self.query)
                            .prefix(Icon::new(IconName::Search).small())
                            .flex_1(),
                    )
                    .child(
                        Button::new("picker-bulk")
                            .outline()
                            .label(if all_picked {
                                s::REMOVE_ALL
                            } else {
                                s::ADD_ALL
                            })
                            .disabled(ids.is_empty() || self.saving)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                for id in &ids {
                                    if all_picked {
                                        this.picked.remove(id);
                                    } else {
                                        this.picked.insert(id.clone());
                                    }
                                }
                                this.apply(cx);
                            })),
                    ),
            )
            .child(h_flex().gap(space::S4).flex_wrap().children(tabs))
            .child(
                v_flex()
                    .id("picker-list")
                    .h(px(440.))
                    .gap(space::S4)
                    .overflow_y_scrollbar()
                    .children(list),
            )
            .child(
                label(text::METADATA, s::picked_count(self.picked.len())).text_color(r.text_muted),
            )
    }
}

impl ModelPicker {
    fn render_pick(
        &self,
        model: DiscoveredModel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let r = theme::roles(cx);
        let id = model.id.clone();
        let kind = self.kind(&id);
        let picked = self.picked.contains(&id);
        let vision = kind == ModelType::Chat && model_hints(&id).vision;
        let entity = cx.entity();
        let menu_id = id.clone();
        let type_menu = Button::new(format!("pick-type-{id}"))
            .ghost()
            .small()
            .label(s::model_type(kind))
            .icon(IconName::ChevronDown)
            .disabled(self.saving)
            .dropdown_menu(move |menu, _, _| {
                ModelType::ALL.into_iter().fold(menu, |menu, value| {
                    let (entity, id) = (entity.clone(), menu_id.clone());
                    menu.item(
                        PopupMenuItem::new(s::model_type(value))
                            .checked(kind == value)
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.types.insert(id.clone(), value);
                                    if this.picked.contains(&id) {
                                        this.apply(cx);
                                    } else {
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                })
            });
        ui::picker_row(
            h_flex()
                .flex_1()
                .min_w(px(0.))
                .gap(space::S8)
                .child(
                    label(text::CODE_TEXT, id.clone())
                        .flex_1()
                        .min_w(px(0.))
                        .truncate(),
                )
                .child(type_menu)
                .children(
                    vision.then(|| label(text::METADATA, s::CAN_VISION).text_color(r.text_subtle)),
                )
                .child(
                    label(text::METADATA, if picked { s::PICKED } else { "" })
                        .w(px(32.))
                        .text_color(r.feedback_success),
                )
                .into_any_element(),
            Button::new(format!("pick-{id}"))
                .small()
                .ghost()
                .icon(if picked {
                    IconName::Minus
                } else {
                    IconName::Plus
                })
                .accessibility_label(if picked {
                    s::REMOVE_MODEL
                } else {
                    s::ADD_MODEL
                })
                .tooltip(if picked {
                    s::REMOVE_MODEL
                } else {
                    s::ADD_MODEL
                })
                .disabled(self.saving)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.picked.remove(&id) {
                        this.picked.insert(id.clone());
                    }
                    this.apply(cx);
                }))
                .into_any_element(),
            picked,
            &r,
        )
    }
}
