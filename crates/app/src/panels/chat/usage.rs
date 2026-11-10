//! Event-driven telemetry cache and UI adapter; all engine I/O runs through background jobs.

use std::{collections::HashMap, sync::Arc};

use gpui_kit::Context;
use lumakit_usage_status::{ContextStatus, StatusState};

use super::{Chat, ChatPanel};
use crate::state::run;
use thinkrail_core::{ChatId, SessionPhase, SessionUsage};

#[derive(Default)]
pub(super) struct Usage {
    snapshot: Option<SessionUsage>,
    generation: u64,
    in_flight: Option<u64>,
    pending: bool,
    pub(super) compacting: bool,
    catalog_windows: HashMap<(String, String), u64>,
}

impl Usage {
    pub(super) fn set_catalog(
        &mut self,
        catalog: &[thinkrail_core::ModelChoice],
        providers: &[thinkrail_core::Provider],
    ) {
        self.catalog_windows = catalog
            .iter()
            .filter_map(|choice| {
                let model = providers
                    .iter()
                    .find(|provider| provider.id == choice.provider)?
                    .input
                    .models
                    .iter()
                    .find(|model| model.model_id == choice.model.id)?;
                let window = model.context_window.filter(|window| *window > 0)?;
                Some((
                    (choice.route.clone(), choice.model.id.clone()),
                    u64::from(window),
                ))
            })
            .collect();
    }

    /// A new prompt/model makes the previous context occupancy obsolete, not the session total.
    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = false;
        if let Some(usage) = self.snapshot.as_mut() {
            usage.used_tokens = None;
            usage.context_window = None;
            usage.percent = None;
        }
    }

    fn finish(&mut self, generation: u64, current: bool, usage: Option<SessionUsage>) -> bool {
        if self.in_flight != Some(generation) {
            return false;
        }
        self.in_flight = None;
        if current && self.generation == generation {
            self.snapshot = usage;
        }
        std::mem::take(&mut self.pending)
    }
}

impl ChatPanel {
    pub(super) fn refresh_usage(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get_mut(id) else {
            return;
        };
        if chat.starting || chat.closing || chat.phase() != SessionPhase::Idle {
            return;
        }
        let Some(session) = chat.live().cloned() else {
            return;
        };
        if chat.usage.in_flight.is_some() {
            chat.usage.pending = true;
            return;
        }
        let generation = chat.usage.generation;
        chat.usage.in_flight = Some(generation);
        let model = chat.snapshot.as_ref().and_then(|s| s.model.clone());
        let worker = session.clone();
        let id = id.clone();
        run(
            cx,
            move || worker.usage_snapshot(),
            move |this, result, cx| {
                let Some(chat) = this.chats.get_mut(&id) else {
                    return;
                };
                let current = chat.live().is_some_and(|live| Arc::ptr_eq(live, &session))
                    && chat.phase() == SessionPhase::Idle
                    && !chat.closing
                    && chat.snapshot.as_ref().and_then(|s| s.model.as_ref()) == model.as_ref();
                // The background root records a failed query; the view displays unknown usage.
                let usage = result.unwrap_or(None);
                let pending = chat.usage.finish(generation, current, usage);
                if pending {
                    this.refresh_usage(&id, cx);
                }
                cx.notify();
            },
        );
    }

    pub(super) fn sync_usage_view(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(id) else {
            return;
        };
        let status = status(chat);
        self.usage_view
            .update(cx, |view, cx| view.set_status(status, cx));
    }
}

fn status(chat: &Chat) -> ContextStatus {
    let usage = chat.usage.snapshot.as_ref();
    let model = chat.snapshot.as_ref().and_then(|s| s.model.as_ref());
    let catalog_window = model.and_then(|model| {
        chat.usage
            .catalog_windows
            .get(&(model.provider.clone().unwrap_or_default(), model.id.clone()))
            .copied()
    });
    let context_window = usage.and_then(|u| u.context_window).or(catalog_window);
    let mut detail = format!(
        "当前上下文：{} / {} tokens\n窗口来源：{}\n累计 tokens：{}\n未配置中转模型单价，仅显示 token 消耗，不套用目录参考价。",
        number(usage.and_then(|u| u.used_tokens)),
        number(context_window),
        if usage.and_then(|u| u.context_window).is_some() {
            "引擎"
        } else if catalog_window.is_some() {
            "模型目录"
        } else {
            "未知"
        },
        number(usage.and_then(|u| u.total_tokens)),
    );
    if let Some(u) = usage {
        detail.push_str(&format!(
            "\n输入（含缓存）：{}\n输出：{}\n缓存读取（输入子集）：{}\n缓存写入（输入子集）：{}\n推理（输出子集）：{}\n上下文：{}",
            number(u.input_tokens), number(u.output_tokens), number(u.cache_read_tokens),
            number(u.cache_write_tokens), number(u.reasoning_tokens),
            if u.estimate { "引擎估算" } else { "引擎报告（未报告的值保持未知）" },
        ));
    } else {
        detail.push_str("\n此引擎尚无可用用量报告。");
    }
    ContextStatus {
        used_tokens: usage.and_then(|u| u.used_tokens),
        context_window,
        percent: usage.and_then(|u| u.percent),
        estimate: usage.is_some_and(|u| u.estimate),
        cumulative_tokens: usage.and_then(|u| u.total_tokens),
        cumulative_cost: None,
        auto_compact: usage.is_some_and(|u| u.auto_compact),
        state: if chat.usage.compacting {
            StatusState::Compacting
        } else {
            match chat.phase() {
                SessionPhase::Running | SessionPhase::Waiting => StatusState::Streaming,
                SessionPhase::Idle if usage.is_some() => StatusState::Ready,
                SessionPhase::Idle | SessionPhase::Closed => StatusState::Unknown,
            }
        },
        detail,
    }
}

fn number(value: Option<u64>) -> String {
    value.map_or_else(|| "未知".to_owned(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::Usage;
    use thinkrail_core::SessionUsage;

    #[test]
    fn prompt_or_model_change_rejects_in_flight_context() {
        let mut state = Usage {
            snapshot: Some(SessionUsage {
                used_tokens: Some(12),
                total_tokens: Some(30),
                ..SessionUsage::default()
            }),
            in_flight: Some(0),
            ..Usage::default()
        };
        state.invalidate();
        state.pending = true;
        assert!(state.finish(
            0,
            true,
            Some(SessionUsage {
                used_tokens: Some(99),
                ..SessionUsage::default()
            })
        ));
        assert_eq!(state.snapshot.as_ref().and_then(|u| u.used_tokens), None);
        assert_eq!(
            state.snapshot.as_ref().and_then(|u| u.total_tokens),
            Some(30)
        );
        state.in_flight = Some(1);
        state.finish(1, false, Some(SessionUsage::default()));
        assert_eq!(
            state.snapshot.as_ref().and_then(|u| u.total_tokens),
            Some(30)
        );
    }
}
