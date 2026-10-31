//! 模型区：按组列出当前线路的模型；同步模型（挑选对话框）、手动添加、模型设置、移除。

use std::collections::{BTreeMap, BTreeSet, HashSet};

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        input::{Input, InputEvent, InputState},
        menu::{DropdownMenu as _, PopupMenuItem},
        scroll::ScrollableElement as _,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{
    DiscoveredModel, ProbeTarget, Protocol, ProviderInput, ProviderModel, ThinkingLevel, line_models, merge_picked,
    model_from_discovery, model_group,
};
use thinkrail_ui::{ThinkrailTheme, components::{IconButton, label}, metrics::{radius, space, text}};

use super::{ProvidersPage, avatar, error_text, field, open_form};
use crate::{state::run, strings::{settings_models::effort_label, settings_providers as s}};

const EFFORTS: [ThinkingLevel; 7] = [
    ThinkingLevel::Off, ThinkingLevel::Minimal, ThinkingLevel::Low, ThinkingLevel::Medium,
    ThinkingLevel::High, ThinkingLevel::Xhigh, ThinkingLevel::Max,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cap { Vision, Reasoning, Tools }

impl Cap {
    fn has(self, model: &ProviderModel) -> bool {
        match self { Cap::Vision => model.vision, Cap::Reasoning => model.reasoning, Cap::Tools => model.tools }
    }
    fn title(self) -> &'static str {
        match self { Cap::Vision => s::CAP_VISION, Cap::Reasoning => s::CAP_REASONING, Cap::Tools => s::CAP_TOOLS }
    }
    fn icon(self) -> IconName {
        match self { Cap::Vision => IconName::Eye, Cap::Reasoning => IconName::Brain, Cap::Tools => IconName::Wrench }
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
        Self { query, cap: None, folded: HashSet::new(), syncing: false, sync_error: None }
    }

    pub(super) fn reset(&mut self) {
        self.cap = None;
        self.folded.clear();
        self.syncing = false;
        self.sync_error = None;
    }
}

/// Takes the models matching `remove` off the line `protocol`; a model left on no line is dropped.
fn drop_from_line(draft: &mut ProviderInput, protocol: Protocol, remove: impl Fn(&ProviderModel) -> bool) {
    draft.models.retain_mut(|model| {
        if remove(model) {
            model.protocols.retain(|p| *p != protocol);
        }
        !model.protocols.is_empty()
    });
}

fn group_name(model_id: &str) -> String {
    let group = model_group(model_id);
    if group.is_empty() { s::NO_GROUP.to_owned() } else { group.to_owned() }
}

impl ProvidersPage {
    pub(super) fn render_models(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let Some(draft) = self.draft.as_ref() else { return div().into_any_element() };
        let line = self.line;
        let query = self.models.query.read(cx).value().trim().to_lowercase();
        let cap = self.models.cap;
        let total = line_models(draft, line).count();
        let mut groups: BTreeMap<String, Vec<ProviderModel>> = BTreeMap::new();
        for (_, model) in line_models(draft, line) {
            let hit = (query.is_empty() || model.model_id.to_lowercase().contains(&query) || model.name.to_lowercase().contains(&query))
                && cap.is_none_or(|c| c.has(model));
            if hit {
                groups.entry(group_name(&model.model_id)).or_default().push(model.clone());
            }
        }
        let all_folded = !groups.is_empty() && groups.keys().all(|g| self.models.folded.contains(g));
        let names: Vec<String> = groups.keys().cloned().collect();
        let page = cx.entity();
        let cap_menu = Button::new("models-cap").ghost().small().label(cap.map_or(s::ALL_TYPES, Cap::title)).icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                [None, Some(Cap::Vision), Some(Cap::Reasoning), Some(Cap::Tools)].into_iter().fold(menu, |menu, value| {
                    let page = page.clone();
                    menu.item(PopupMenuItem::new(value.map_or(s::ALL_TYPES, Cap::title)).checked(cap == value)
                        .on_click(move |_, _, cx| page.update(cx, |this, cx| { this.models.cap = value; cx.notify(); })))
                })
            });
        let header = h_flex().gap(space::S4)
            .child(label(text::TITLE_SECTION, s::MODELS).mr(space::S4))
            .child(IconButton::new("models-fold", if all_folded { IconName::ChevronsUpDown } else { IconName::ChevronsDownUp })
                .tooltip(if all_folded { s::EXPAND_ALL } else { s::COLLAPSE_ALL }).disabled(names.is_empty())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if all_folded { this.models.folded.clear() } else { this.models.folded.extend(names.iter().cloned()) }
                    cx.notify();
                })))
            .child(Input::new(&self.models.query).small().prefix(Icon::new(IconName::Search).small()).w(px(160.)))
            .child(cap_menu)
            .child(div().flex_1())
            .child(Button::new("models-sync").outline().icon(IconName::RefreshCw).label(s::SYNC_MODELS)
                .disabled(self.models.syncing || self.saving || self.dirty)
                .on_click(cx.listener(|this, _, window, cx| this.sync_models(window, cx))))
            .child(Button::new("models-add").outline().icon(IconName::Plus).tooltip(s::ADD_MODEL)
                .on_click(cx.listener(|this, _, window, cx| this.open_add_model(window, cx))));
        let body: AnyElement = if groups.is_empty() {
            div().py(space::S24).rounded(radius::MD).border_1().border_dashed().border_color(r.border_default).flex().justify_center()
                .child(label(text::METADATA, if total == 0 { s::NO_MODELS } else { s::NO_MODEL_MATCHES }).text_color(r.text_subtle))
                .into_any_element()
        } else {
            v_flex().rounded(radius::LG).border_1().border_color(r.border_muted).overflow_hidden()
                .children(groups.into_iter().map(|(group, models)| self.render_group(group, models, cx)))
                .into_any_element()
        };
        v_flex().gap(space::S8).child(header).children(error_text(self.models.sync_error.clone(), cx)).child(body).into_any_element()
    }

    fn render_group(&self, group: String, models: Vec<ProviderModel>, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let folded = self.models.folded.contains(&group);
        let line = self.line;
        let ids: HashSet<String> = models.iter().map(|m| m.model_id.clone()).collect();
        let toggle = group.clone();
        v_flex()
            .child(h_flex().id(format!("model-group-{group}")).h(px(40.)).px(space::S12).gap(space::S8).bg(r.container_elevated_bg)
                .border_b_1().border_color(r.border_muted).cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.models.folded.remove(&toggle) { this.models.folded.insert(toggle.clone()); }
                    cx.notify();
                }))
                .child(Icon::new(if folded { IconName::ChevronRight } else { IconName::ChevronDown }).small())
                .child(label(text::UI, group.clone()))
                .child(label(text::METADATA, models.len().to_string()).text_color(r.text_subtle))
                .child(div().flex_1())
                .child(IconButton::new(format!("model-group-remove-{group}"), IconName::Minus).tooltip(s::REMOVE_GROUP)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let ids = ids.clone();
                        this.edit(cx, |d| drop_from_line(d, line, |m| ids.contains(&m.model_id)));
                    }))))
            .when(!folded, |d| d.children(models.iter().map(|model| self.render_model(model, cx))))
            .into_any_element()
    }

    fn render_model(&self, model: &ProviderModel, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let line = self.line;
        let (id, edit_id) = (model.model_id.clone(), model.model_id.clone());
        let caps = [Cap::Vision, Cap::Reasoning, Cap::Tools].into_iter().filter(|c| c.has(model)).map(|c| {
            let color = match c { Cap::Vision => r.feedback_success, Cap::Reasoning => r.feedback_info, Cap::Tools => r.feedback_warning };
            div().size(px(24.)).rounded(radius::MD).bg(color.opacity(0.15)).flex().items_center().justify_center()
                .child(Icon::new(c.icon()).xsmall().text_color(color))
        });
        h_flex().h(px(44.)).px(space::S12).gap(space::S8).border_b_1().border_color(r.border_muted)
            .when(!model.enabled, |d| d.opacity(0.6))
            .child(avatar(&model.model_id, cx))
            .child(label(text::UI, model.model_id.clone()).flex_1().min_w(px(0.)).truncate())
            .child(h_flex().gap(space::S4).children(caps))
            .child(IconButton::new(format!("model-settings-{id}"), IconName::Settings2).tooltip(s::MODEL_SETTINGS)
                .on_click(cx.listener(move |this, _, window, cx| this.open_model_settings(edit_id.clone(), window, cx))))
            .child(IconButton::new(format!("model-remove-{id}"), IconName::Minus).tooltip(s::REMOVE_MODEL)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.edit(cx, |d| drop_from_line(d, line, |m| m.model_id == id));
                })))
            .into_any_element()
    }
}

impl ProvidersPage {
    /// "同步模型": lists the line's models with the first enabled key, then opens the picker.
    fn sync_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self.provider().cloned() else { return };
        let Some(endpoint) = provider.input.endpoints.iter().find(|e| e.protocol == self.line).cloned() else { return };
        self.models.syncing = true;
        self.models.sync_error = None;
        let (service, probe) = (self.service(cx), self.state.read(cx).services.probe.clone());
        let (line, selected) = (self.line, self.selected.clone());
        let handle = window.window_handle();
        run(cx, move || {
            let key = provider.keys.iter().find(|k| k.enabled).map(|k| service.reveal_key(&k.id)).transpose()?;
            probe.list_models(&ProbeTarget { endpoint, headers: provider.input.headers.clone(), key })
        }, move |this, result, cx| {
            this.models.syncing = false;
            if this.selected != selected || this.line != line {
                return;
            }
            match result {
                Ok(found) => {
                    let page = cx.entity();
                    let picked = this.draft.as_ref().map(|d| line_models(d, line).map(|(_, m)| m.model_id.clone()).collect()).unwrap_or_default();
                    let _ = handle.update(cx, |_, window, cx| open_picker(page, found, picked, line, window, cx));
                }
                Err(error) => this.models.sync_error = Some(error.to_string()),
            }
            cx.notify();
        });
    }
}

struct ModelPicker {
    found: Vec<DiscoveredModel>,
    picked: BTreeSet<String>,
    query: Entity<InputState>,
    _subscription: Subscription,
}

fn open_picker(
    page: Entity<ProvidersPage>,
    found: Vec<DiscoveredModel>,
    picked: BTreeSet<String>,
    line: Protocol,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let picker = cx.new(|cx| {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(s::SEARCH_MODELS));
        let subscription = cx.subscribe(&query, |_, _, event: &InputEvent, cx| if matches!(event, InputEvent::Change) { cx.notify() });
        ModelPicker { found, picked, query, _subscription: subscription }
    });
    open_form(window, cx, s::PICKER_TITLE, px(560.), picker, s::CONFIRM, move |picker, _, cx| {
        let (found, picked) = (picker.found.clone(), picker.picked.clone());
        page.update(cx, |this, cx| this.edit(cx, |d| d.models = merge_picked(&d.models, &found, &picked, line)));
        true
    });
}

impl Render for ModelPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let query = self.query.read(cx).value().trim().to_lowercase();
        let shown: Vec<DiscoveredModel> = self.found.iter()
            .filter(|m| query.is_empty() || m.id.to_lowercase().contains(&query) || m.name.as_deref().is_some_and(|n| n.to_lowercase().contains(&query)))
            .cloned().collect();
        let ids: Vec<String> = shown.iter().map(|m| m.id.clone()).collect();
        let remove = ids.clone();
        v_flex().gap(space::S8)
            .child(label(text::METADATA, s::PICKER_HINT).text_color(r.text_subtle))
            .child(h_flex().gap(space::S8).child(Input::new(&self.query).prefix(Icon::new(IconName::Search).small()).flex_1())
                .child(Button::new("picker-all").ghost().label(s::ADD_ALL)
                    .on_click(cx.listener(move |this, _, _, cx| { this.picked.extend(ids.iter().cloned()); cx.notify(); })))
                .child(Button::new("picker-none").ghost().label(s::REMOVE_ALL)
                    .on_click(cx.listener(move |this, _, _, cx| { for id in &remove { this.picked.remove(id); } cx.notify(); }))))
            .child(label(text::METADATA, s::picked_count(self.picked.len())).text_color(r.text_muted))
            .child(v_flex().id("picker-list").max_h(px(360.)).overflow_y_scrollbar().children(shown.into_iter().map(|model| {
                let id = model.id.clone();
                let detail = [model.name.clone().filter(|n| n != &model.id), model.context_window.map(|c| format!("{c} tokens"))]
                    .into_iter().flatten().collect::<Vec<_>>().join(" · ");
                h_flex().h(px(32.)).gap(space::S8)
                    .child(Checkbox::new(format!("pick-{id}")).checked(self.picked.contains(&id))
                        .on_click(cx.listener(move |this, value: &bool, _, cx| {
                            if *value { this.picked.insert(id.clone()); } else { this.picked.remove(&id); }
                            cx.notify();
                        })))
                    .child(label(text::CODE_TEXT, model.id.clone()).truncate())
                    .child(label(text::METADATA, detail).text_color(r.text_subtle).truncate())
            })))
    }
}

struct AddModel { id: Entity<InputState> }

impl Render for AddModel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        field(s::MODEL_ID, Input::new(&self.id), cx)
    }
}

struct ModelSettings {
    model: ProviderModel,
    lines: Vec<Protocol>,
    name: Entity<InputState>,
    context_window: Entity<InputState>,
    max_tokens: Entity<InputState>,
    error: Option<&'static str>,
}

impl ProvidersPage {
    fn open_add_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = cx.new(|cx| AddModel { id: cx.new(|cx| InputState::new(window, cx).placeholder(s::MODEL_ID)) });
        let (page, line) = (cx.entity(), self.line);
        open_form(window, cx, s::ADD_MODEL, px(420.), body, s::CONFIRM, move |body, _, cx| {
            let id = body.id.read(cx).value().trim().to_owned();
            if id.is_empty() {
                return false;
            }
            page.update(cx, |this, cx| this.edit(cx, |d| match d.models.iter_mut().find(|m| m.model_id == id) {
                Some(model) if !model.protocols.contains(&line) => model.protocols.push(line),
                Some(_) => {}
                None => d.models.push(model_from_discovery(&DiscoveredModel { id, name: None, context_window: None }, line)),
            }));
            true
        });
    }

    fn open_model_settings(&mut self, model_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_ref() else { return };
        let Some(model) = draft.models.iter().find(|m| m.model_id == model_id).cloned() else { return };
        let lines = draft.endpoints.iter().map(|e| e.protocol).collect();
        let number = |value: Option<u32>| value.map(|v| v.to_string()).unwrap_or_default();
        let body = cx.new(|cx| {
            let mut input = |value: String| cx.new(|cx| InputState::new(window, cx).default_value(value));
            ModelSettings {
                name: input(model.name.clone()),
                context_window: input(number(model.context_window)),
                max_tokens: input(number(model.max_tokens)),
                model, lines, error: None,
            }
        });
        let page = cx.entity();
        open_form(window, cx, s::EDIT_MODEL, px(520.), body, s::SAVE, move |view, _, cx| match view.result(cx) {
            Ok(model) => {
                page.update(cx, |this, cx| this.edit(cx, |d| {
                    if let Some(slot) = d.models.iter_mut().find(|m| m.model_id == model.model_id) { *slot = model; }
                }));
                true
            }
            Err(error) => { view.error = Some(error); cx.notify(); false }
        });
    }
}

impl ModelSettings {
    fn result(&self, cx: &gpui_kit::App) -> Result<ProviderModel, &'static str> {
        let number = |input: &Entity<InputState>| {
            let raw = input.read(cx).value().trim().to_owned();
            if raw.is_empty() { Ok(None) } else { raw.parse::<u32>().ok().filter(|v| *v > 0).map(Some).ok_or(s::NUMBER_INVALID) }
        };
        let mut model = self.model.clone();
        let name = self.name.read(cx).value().trim().to_owned();
        model.name = if name.is_empty() { model.model_id.clone() } else { name };
        model.context_window = number(&self.context_window)?;
        model.max_tokens = number(&self.max_tokens)?;
        if model.protocols.is_empty() { return Err(s::MODEL_LINE_REQUIRED); }
        if !model.reasoning { model.efforts.clear(); } else if model.efforts.is_empty() { return Err(s::EFFORTS_REQUIRED); }
        Ok(model)
    }
}

impl Render for ModelSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let lines = h_flex().gap(space::S12).children(self.lines.iter().map(|&protocol| {
            let title = if self.model.protocols.first() == Some(&protocol) {
                format!("{} · {}", s::protocol_short(protocol), s::DEFAULT_BADGE)
            } else {
                s::protocol_short(protocol).to_owned()
            };
            Checkbox::new(format!("model-line-{protocol:?}")).label(title).checked(self.model.protocols.contains(&protocol))
                .on_click(cx.listener(move |this, value: &bool, _, cx| {
                    this.model.protocols.retain(|p| *p != protocol);
                    if *value { this.model.protocols.push(protocol); }
                    cx.notify();
                }))
        }));
        let caps = h_flex().gap(space::S12).children([Cap::Vision, Cap::Reasoning, Cap::Tools].map(|cap| {
            Checkbox::new(format!("model-cap-{cap:?}")).label(cap.title()).checked(cap.has(&self.model))
                .on_click(cx.listener(move |this, value: &bool, _, cx| {
                    let m = &mut this.model;
                    match cap { Cap::Vision => m.vision = *value, Cap::Reasoning => m.reasoning = *value, Cap::Tools => m.tools = *value }
                    cx.notify();
                }))
        }));
        let efforts = self.model.reasoning.then(|| field(s::EFFORTS, h_flex().gap(space::S4).flex_wrap().children(EFFORTS.map(|effort| {
            let on = self.model.efforts.contains(&effort);
            Button::new(format!("model-effort-{effort:?}")).small().label(effort_label(effort))
                .when(on, |b| b.primary()).when(!on, |b| b.outline())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let efforts = &mut this.model.efforts;
                    if efforts.contains(&effort) { efforts.retain(|e| *e != effort) } else { efforts.push(effort); efforts.sort(); }
                    cx.notify();
                }))
        })), cx));
        v_flex().gap(space::S12)
            .child(label(text::CODE_TEXT, self.model.model_id.clone()).text_color(r.text_muted))
            .child(field(s::MODEL_NAME, Input::new(&self.name), cx))
            .child(field(s::MODEL_PROTOCOLS, lines, cx))
            .child(h_flex().gap(space::S12)
                .child(div().flex_1().child(field(s::CONTEXT_WINDOW, Input::new(&self.context_window), cx)))
                .child(div().flex_1().child(field(s::MAX_OUTPUT, Input::new(&self.max_tokens), cx))))
            .child(field(s::CAPABILITIES, caps, cx))
            .children(efforts)
            .children(error_text(self.error.map(str::to_owned), cx))
            .child(label(text::METADATA, s::APPLIES_NEXT).text_color(r.text_subtle))
    }
}
