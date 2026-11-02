//! 同步模型：用首个启用的密钥探测线路模型，再打开挑选对话框（`ModelPicker`）。

use std::collections::{BTreeMap, BTreeSet, HashSet};

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
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
    DiscoveredModel, ModelType, ProbeTarget, Protocol, ProviderModel, merge_picked, model_hints,
    model_type,
};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, label},
    metrics::{radius, space, text},
};

use super::group_name;
use crate::{
    panels::settings::providers::ProvidersPage, state::run, strings::settings_providers as s,
};

impl ProvidersPage {
    /// "同步模型": lists the line's models with the first enabled key, then opens the picker.
    pub(super) fn sync_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self.provider().cloned() else {
            return;
        };
        let Some(endpoint) = provider
            .input
            .endpoints
            .iter()
            .find(|e| e.protocol == self.line)
            .cloned()
        else {
            return;
        };
        self.models.syncing = true;
        self.models.sync_error = None;
        let (service, probe) = (self.service(cx), self.state.read(cx).services.probe.clone());
        let (line, selected) = (self.line, self.selected.clone());
        let handle = window.window_handle();
        run(
            cx,
            move || {
                let key = provider
                    .keys
                    .iter()
                    .find(|k| k.enabled)
                    .map(|k| service.reveal_key(&k.id))
                    .transpose()?;
                probe.list_models(&ProbeTarget {
                    endpoint,
                    headers: provider.input.headers.clone(),
                    key,
                })
            },
            move |this, result, cx| {
                this.models.syncing = false;
                if this.selected != selected || this.line != line {
                    return;
                }
                match result {
                    Ok(found) => {
                        let page = cx.entity();
                        let draft = this
                            .draft
                            .as_ref()
                            .map(|d| d.models.clone())
                            .unwrap_or_default();
                        let _ = handle.update(cx, |_, window, cx| {
                            open_picker(page, found, draft, line, window, cx)
                        });
                    }
                    Err(error) => this.models.sync_error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
    }
}

/// "同步模型" picker (Aether `ModelPickerDialog`): every + / − saves at once; models the probe
/// did not list are left alone by `merge_picked`.
struct ModelPicker {
    page: Entity<ProvidersPage>,
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

fn open_picker(
    page: Entity<ProvidersPage>,
    found: Vec<DiscoveredModel>,
    draft: Vec<ProviderModel>,
    line: Protocol,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let picked = draft
        .iter()
        .filter(|m| m.protocols.contains(&line))
        .map(|m| m.model_id.clone())
        .collect();
    let types = found
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
        let r = ThinkrailTheme::global(cx).roles;
        dialog
            .w(px(640.))
            .on_ok(|_, _, _| false)
            .title(
                h_flex()
                    .gap(space::S8)
                    .child(label(text::TITLE_DIALOG, s::PICKER_TITLE))
                    .child(
                        label(text::METADATA, total.to_string())
                            .px(space::S6)
                            .rounded(radius::SM)
                            .bg(r.control_bg),
                    ),
            )
            .child(picker.clone())
    });
}

impl ModelPicker {
    /// Saves the current selection and types into the page draft.
    fn apply(&mut self, cx: &mut Context<Self>) {
        let (found, picked, types, line) = (
            self.found.clone(),
            self.picked.clone(),
            self.types.clone(),
            self.line,
        );
        self.page.update(cx, |page, cx| {
            page.edit(cx, |d| {
                d.models = merge_picked(&d.models, &found, &picked, line);
                for model in d.models.iter_mut().filter(|m| picked.contains(&m.model_id)) {
                    if let Some(&kind) = types.get(&model.model_id) {
                        model.model_type = kind;
                    }
                }
            })
        });
        cx.notify();
    }

    fn kind(&self, id: &str) -> ModelType {
        self.types.get(id).copied().unwrap_or(ModelType::Unknown)
    }
}

impl Render for ModelPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
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
                    .child(
                        h_flex()
                            .id(format!("picker-group-{group}"))
                            .h(px(32.))
                            .px(space::S8)
                            .gap(space::S8)
                            .rounded(radius::MD)
                            .bg(r.container_elevated_bg)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.folded.remove(&toggle) {
                                    this.folded.insert(toggle.clone());
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
                                label(text::METADATA, models.len().to_string())
                                    .text_color(r.text_subtle),
                            ),
                    )
                    .when(!folded, |d| {
                        d.children(models.into_iter().map(|model| self.render_pick(model, cx)))
                    })
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap(space::S8)
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
                            .disabled(ids.is_empty())
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
        let r = ThinkrailTheme::global(cx).roles;
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
        h_flex()
            .h(px(36.))
            .pl(space::S16)
            .pr(space::S4)
            .gap(space::S8)
            .rounded(radius::MD)
            .hover(|s| s.bg(r.control_bg_hovered))
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
            .child(
                IconButton::new(
                    format!("pick-{id}"),
                    if picked {
                        IconName::Minus
                    } else {
                        IconName::Plus
                    },
                )
                .tooltip(if picked {
                    s::REMOVE_MODEL
                } else {
                    s::ADD_MODEL
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.picked.remove(&id) {
                        this.picked.insert(id.clone());
                    }
                    this.apply(cx);
                })),
            )
    }
}
