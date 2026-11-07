//! Maps host model contracts to the shared picker; engines and preferences remain host-owned.
use std::collections::HashSet;

use gpui_kit::{AppContext as _, Context, Entity, Subscription, Window};
use lumakit_model_picker::{Effort, Model, Picker, PickerData, PickerEvent, Selection};
use thinkrail_core::{
    ChatId, Error, ModelChoice, ModelPrefs, ModelRef, SessionPhase, ThinkingLevel,
};

use super::ChatPanel;
use crate::state::{AppEvent, run};

const LEVELS: [(ThinkingLevel, &str); 7] = [
    (ThinkingLevel::Off, "off"),
    (ThinkingLevel::Minimal, "minimal"),
    (ThinkingLevel::Low, "low"),
    (ThinkingLevel::Medium, "medium"),
    (ThinkingLevel::High, "high"),
    (ThinkingLevel::Xhigh, "xhigh"),
    (ThinkingLevel::Max, "max"),
];

pub(super) struct ModelPicker {
    view: Entity<Picker>,
    prefs: ModelPrefs,
    refreshing: HashSet<ChatId>,
    switching: HashSet<ChatId>,
    saving_default: bool,
    keyed_providers: HashSet<String>,
    notice: String,
    _subscription: Subscription,
}

impl ModelPicker {
    pub(super) fn view(&self) -> Entity<Picker> {
        self.view.clone()
    }
    pub(super) fn begin_refresh(&mut self, id: &ChatId) -> bool {
        self.refreshing.insert(id.clone())
    }
    pub(super) fn finish_refresh(
        &mut self,
        id: &ChatId,
        prefs: ModelPrefs,
        providers: &[thinkrail_core::Provider],
    ) {
        self.refreshing.remove(id);
        self.prefs = prefs;
        self.keyed_providers = providers
            .iter()
            .filter(|provider| provider.keys.iter().any(|key| key.enabled))
            .map(|provider| provider.id.to_string())
            .collect();
        self.notice.clear();
    }
    pub(super) fn refresh_failed(&mut self, id: &ChatId, error: &Error) {
        self.refreshing.remove(id);
        self.notice = error.to_string();
    }
    pub(super) fn begin_switch(&mut self, id: &ChatId) -> bool {
        self.switching.insert(id.clone())
    }
    pub(super) fn finish_switch(&mut self, id: &ChatId, error: Option<&Error>) {
        self.switching.remove(id);
        self.notice = error.map_or_else(String::new, ToString::to_string);
    }
}

pub(super) fn key(choice: &ModelChoice) -> String {
    format!(
        "{:?}",
        (
            choice.provider.as_str(),
            choice.route.as_str(),
            choice.model.id.as_str()
        )
    )
}

fn effort_id(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
        ThinkingLevel::Max => "max",
    }
}

fn effort(value: Option<&str>) -> Result<Option<ThinkingLevel>, Error> {
    value
        .map(|id| {
            LEVELS
                .iter()
                .find(|(_, word)| *word == id)
                .map(|(level, _)| *level)
                .ok_or_else(|| Error::invalid("无法识别的推理强度"))
        })
        .transpose()
}

impl ChatPanel {
    pub(super) fn new_model_picker(window: &mut Window, cx: &mut Context<Self>) -> ModelPicker {
        let view = cx.new(|cx| Picker::new(window, cx));
        let subscription = cx.subscribe(&view, |this, _, event: &PickerEvent, cx| {
            this.model_picker_event(event.clone(), cx);
        });
        ModelPicker {
            view,
            prefs: ModelPrefs::default(),
            refreshing: HashSet::new(),
            switching: HashSet::new(),
            saving_default: false,
            keyed_providers: HashSet::new(),
            notice: String::new(),
            _subscription: subscription,
        }
    }

    pub(super) fn sync_model_picker(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(id) else {
            return;
        };
        let current = chat
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.model.as_ref());
        let selected = current.and_then(|current| {
            chat.catalog
                .iter()
                .find(|choice| {
                    choice.route == current.provider.as_deref().unwrap_or_default()
                        && choice.model.id == current.id
                })
                .map(|choice| Selection {
                    model: key(choice),
                    effort: current.effort.map(|level| effort_id(level).into()),
                })
        });
        let default = self
            .model_picker
            .prefs
            .default_model
            .as_ref()
            .and_then(|saved| {
                chat.catalog
                    .iter()
                    .find(|choice| {
                        choice.provider == saved.provider && choice.model.id == saved.model_id
                    })
                    .map(|choice| Selection {
                        model: key(choice),
                        effort: self
                            .model_picker
                            .prefs
                            .default_effort
                            .map(|level| effort_id(level).into()),
                    })
            });
        let enabled = chat.live().is_some()
            && !chat.closing
            && !chat.starting
            && chat.phase() == SessionPhase::Idle
            && !self.model_picker.switching.contains(id)
            && !self.model_picker.saving_default
            && chat
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.capabilities.model_switch);
        let models = chat
            .catalog
            .iter()
            .map(|choice| Model {
                id: key(choice),
                provider_id: choice.provider.to_string(),
                provider: choice.provider_name.clone(),
                name: choice.model.name.clone(),
                detail: format!(
                    "{} · {} 上下文",
                    choice.route,
                    crate::strings::settings_models::format_context(choice.model.context_window)
                ),
                has_key: self
                    .model_picker
                    .keyed_providers
                    .contains(choice.provider.as_str()),
                efforts: LEVELS
                    .iter()
                    .filter(|(level, _)| choice.model.efforts.contains(level))
                    .map(|(level, id)| Effort {
                        id: (*id).into(),
                        label: crate::strings::settings_models::effort_label(*level).into(),
                        hint: match level {
                            ThinkingLevel::Off => "无推理",
                            ThinkingLevel::Minimal => "最简推理",
                            ThinkingLevel::Low => "轻量推理",
                            ThinkingLevel::Medium => "均衡推理",
                            ThinkingLevel::High => "深度推理",
                            ThinkingLevel::Xhigh => "超深度推理",
                            ThinkingLevel::Max => "最高推理",
                        }
                        .into(),
                    })
                    .collect(),
            })
            .collect();
        let data = PickerData {
            models,
            selected,
            default,
            enabled,
            refreshing: self.model_picker.refreshing.contains(id),
            notice: self.model_picker.notice.clone(),
        };
        self.model_picker
            .view
            .update(cx, |picker, cx| picker.set_data(data, cx));
    }

    fn model_picker_event(&mut self, event: PickerEvent, cx: &mut Context<Self>) {
        let Some(id) = self.active.clone() else {
            return;
        };
        match event {
            PickerEvent::Refresh { force: _ } => self.refresh_catalog(&id, cx),
            PickerEvent::Select(selection) => {
                let result = self.picker_selection(&id, &selection);
                match result {
                    Ok((choice, effort)) => {
                        self.set_model(id, choice, effort, cx);
                    }
                    Err(error) => self.report(&error, cx),
                }
            }
            PickerEvent::SetDefault(selection) => {
                let (choice, effort) = match self.picker_selection(&id, &selection) {
                    Ok(value) => value,
                    Err(error) => {
                        self.report(&error, cx);
                        return;
                    }
                };
                if self.model_picker.saving_default {
                    return;
                }
                self.model_picker.saving_default = true;
                let prefs = self.services(cx).prefs;
                let model = ModelRef {
                    provider: choice.provider,
                    model_id: choice.model.id,
                };
                run(
                    cx,
                    move || prefs.set_default(Some(model), effort),
                    |this, result, cx| {
                        this.model_picker.saving_default = false;
                        match result {
                            Ok(prefs) => {
                                this.model_picker.prefs = prefs;
                                this.model_picker.notice.clear();
                                this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                            }
                            Err(error) => {
                                this.model_picker.notice = error.to_string();
                                this.report(&error, cx);
                            }
                        }
                        cx.notify();
                    },
                );
            }
        }
    }

    fn picker_selection(
        &self,
        id: &ChatId,
        selection: &Selection,
    ) -> Result<(ModelChoice, Option<ThinkingLevel>), Error> {
        let choice = self
            .chats
            .get(id)
            .and_then(|chat| {
                chat.catalog
                    .iter()
                    .find(|choice| key(choice) == selection.model)
            })
            .ok_or_else(|| Error::invalid("模型已不可用，请刷新后重选"))?;
        let effort = effort(selection.effort.as_deref())?;
        if effort.is_some_and(|level| !choice.model.efforts.contains(&level)) {
            return Err(Error::invalid("模型不支持指定推理强度"));
        }
        Ok((choice.clone(), effort))
    }
}
