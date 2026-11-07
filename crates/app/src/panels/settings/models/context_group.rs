//! GPT 上下文窗口 group (agent-settings 08 §6): overrides of the context window for OpenAI
//! Responses models, shared (全局) or per model. Each control picks 默认 (no override), 1M or
//! 自定义 (272,000–1,000,000 typed into one shared field, saved on Enter or blur, Esc reverts).
//! The provider's own `context_window` is never rewritten.

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, Subscription, Window,
    base::{h_flex, v_flex},
    component::{
        Sizable as _,
        input::{Escape, Input, InputEvent, InputState},
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{CONTEXT_OVERRIDE_RANGE, ModelPrefs, ModelRef, Protocol};
use thinkrail_ui::{
    Roles,
    components::{Segment, label},
    metrics::{space, text},
};

use super::{
    super::rows::{disclosure, notice, row},
    ModelsPage, PreferenceWrite, Snapshot, VisibilityModel,
};
use crate::strings::settings_models as s;

const MILLION: u32 = 1_000_000;
/// What routing projects when the provider leaves the window empty (`routing` `plan.rs`).
const ROUTING_DEFAULT: u32 = 128_000;
/// `Segment` index that matches no option: mixed overrides select nothing.
const NONE_ACTIVE: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Default,
    Million,
    Custom(u32),
}

impl Choice {
    fn of(tokens: Option<u32>) -> Self {
        match tokens {
            None => Self::Default,
            Some(MILLION) => Self::Million,
            Some(tokens) => Self::Custom(tokens),
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Default => 0,
            Self::Million => 1,
            Self::Custom(_) => 2,
        }
    }
}

/// The one choice every override shares; `None` when they differ (the original's "mixed").
pub(super) fn shared_choice(overrides: &[Option<u32>]) -> Option<Choice> {
    let first = *overrides.first()?;
    overrides
        .iter()
        .all(|tokens| *tokens == first)
        .then(|| Choice::of(first))
}

/// A typed window: digits only (thousands separators allowed), inside the override range.
pub(super) fn parse_tokens(draft: &str) -> Option<u32> {
    let digits: String = draft.trim().chars().filter(|c| *c != ',').collect();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits
        .parse()
        .ok()
        .filter(|tokens| CONTEXT_OVERRIDE_RANGE.contains(tokens))
}

/// Models the group governs: enabled chat models that speak OpenAI Responses.
fn eligible(snapshot: &Snapshot) -> Vec<&VisibilityModel> {
    snapshot
        .all_models
        .iter()
        .filter(|m| m.model.protocols.contains(&Protocol::OpenaiResponses))
        .collect()
}

/// Which control the shared 自定义 field belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    All,
    One(ModelRef),
}

pub(super) struct ContextGroup {
    input: Entity<InputState>,
    /// The control whose 自定义 field is open.
    editing: Option<Target>,
    each_open: bool,
}

impl ContextGroup {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<ModelsPage>,
    ) -> (Self, Vec<Subscription>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(s::CONTEXT_PLACEHOLDER));
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |page, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => cx.notify(),
                        InputEvent::PressEnter { .. } | InputEvent::Blur => {
                            page.commit_custom(window, cx)
                        }
                        InputEvent::Focus => {}
                    }
                }),
            ];
        (
            Self {
                input,
                editing: None,
                each_open: false,
            },
            subscriptions,
        )
    }
}

fn targets(snapshot: &Snapshot, target: &Target) -> Vec<ModelRef> {
    match target {
        Target::All => eligible(snapshot)
            .into_iter()
            .map(|m| m.reference.clone())
            .collect(),
        Target::One(model) => vec![model.clone()],
    }
}

fn current(prefs: &ModelPrefs, models: &[ModelRef]) -> Vec<Option<u32>> {
    models.iter().map(|m| prefs.context_override(m)).collect()
}

impl ModelsPage {
    fn write_context(&mut self, target: &Target, tokens: Option<u32>, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let changes = targets(snapshot, target)
            .into_iter()
            .map(|m| (m, tokens))
            .collect();
        self.write(PreferenceWrite::Context(changes), cx);
    }

    /// A segment click: 默认 and 1M save at once; 自定义 opens the field with the current value.
    fn choose(
        &mut self,
        target: Target,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match index {
            0 | 1 => {
                self.context_group.editing = None;
                let tokens = (index == 1).then_some(MILLION);
                self.write_context(&target, tokens, cx);
            }
            _ => {
                let shown = self.snapshot.as_ref().and_then(|snapshot| {
                    match shared_choice(&current(&snapshot.prefs, &targets(snapshot, &target))) {
                        Some(Choice::Custom(tokens)) => Some(tokens.to_string()),
                        _ => None,
                    }
                });
                self.context_group.editing = Some(target);
                self.context_group.input.update(cx, |input, cx| {
                    input.set_value(shown.unwrap_or_default(), window, cx);
                    input.focus(window, cx);
                });
            }
        }
        cx.notify();
    }

    fn commit_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.context_group.editing.clone() else {
            return;
        };
        let Some(tokens) = parse_tokens(&self.context_group.input.read(cx).value()) else {
            cx.notify();
            return;
        };
        self.write_context(&target, Some(tokens), cx);
        self.context_group.input.update(cx, |input, cx| {
            input.set_value(tokens.to_string(), window, cx)
        });
    }

    fn revert_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.context_group.editing.as_ref().and_then(|target| {
            let snapshot = self.snapshot.as_ref()?;
            match shared_choice(&current(&snapshot.prefs, &targets(snapshot, target))) {
                Some(Choice::Custom(tokens)) => Some(tokens),
                _ => None,
            }
        });
        match shown {
            Some(tokens) => self.context_group.input.update(cx, |input, cx| {
                input.set_value(tokens.to_string(), window, cx)
            }),
            None => self.context_group.editing = None,
        }
        cx.notify();
    }

    /// One 默认 / 1M / 自定义 control, plus the field when this control's 自定义 is open.
    fn context_control(
        &self,
        id: String,
        target: Target,
        overrides: &[Option<u32>],
        r: &Roles,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let choice = shared_choice(overrides);
        let editing = self.context_group.editing.as_ref() == Some(&target);
        let active = match choice {
            _ if editing => 2,
            Some(choice) => choice.index(),
            None => NONE_ACTIVE,
        };
        let disabled = self.busy();
        let page = cx.entity().downgrade();
        let segment = Segment::new(id.clone(), s::CONTEXT_CHOICES, active);
        let segment = if disabled {
            segment
        } else {
            segment.on_select(move |index, window, cx| {
                let target = target.clone();
                let _ = page.update(cx, |page, cx| page.choose(target, *index, window, cx));
            })
        };
        let draft = self.context_group.input.read(cx).value();
        let invalid = editing && !draft.trim().is_empty() && parse_tokens(&draft).is_none();
        let field = (editing || matches!(choice, Some(Choice::Custom(_)))).then(|| {
            v_flex()
                .id(format!("{id}-custom"))
                .w(px(150.))
                .capture_action(cx.listener(|page, _: &Escape, window, cx| {
                    page.revert_custom(window, cx);
                    cx.stop_propagation();
                }))
                .when(editing, |field| {
                    field.child(
                        Input::new(&self.context_group.input)
                            .small()
                            .aria_label(s::CONTEXT_INPUT),
                    )
                })
                .when(!editing, |field| {
                    field.child(label(
                        text::UI,
                        match choice {
                            Some(Choice::Custom(tokens)) => s::format_context(tokens),
                            _ => String::new(),
                        },
                    ))
                })
                .when(invalid, |field| {
                    field.border_1().border_color(r.feedback_error).rounded_sm()
                })
        });
        v_flex()
            .items_end()
            .gap(space::S4)
            .child(
                h_flex()
                    .gap(space::S8)
                    .when(disabled, |d| d.opacity(0.6))
                    .child(segment)
                    .children(field),
            )
            .when(invalid, |d| {
                d.child(label(text::METADATA, s::CONTEXT_INVALID).text_color(r.feedback_error))
            })
            .into_any_element()
    }

    pub(super) fn context_group(&self, r: &Roles, cx: &mut Context<Self>) -> Option<AnyElement> {
        let snapshot = self.snapshot.as_ref()?;
        let models = eligible(snapshot);
        if models.is_empty() {
            return Some(row(
                s::CONTEXT_ALL,
                None,
                label(text::UI, s::CONTEXT_NONE).text_color(r.text_muted),
                r,
            ));
        }
        let all: Vec<ModelRef> = models.iter().map(|m| m.reference.clone()).collect();
        let overrides = current(&snapshot.prefs, &all);
        let description = if shared_choice(&overrides).is_some() {
            s::context_available(models.len())
        } else {
            s::CONTEXT_MIXED.to_owned()
        };
        let each = disclosure(
            "models-context-each",
            s::CONTEXT_EACH,
            String::new(),
            self.context_group.each_open,
            r,
            {
                let page = cx.entity().downgrade();
                move |_, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.context_group.each_open = !page.context_group.each_open;
                        cx.notify();
                    });
                }
            },
        );
        let rows = self.context_group.each_open.then(|| {
            models
                .iter()
                .enumerate()
                .map(|(index, m)| {
                    let reference = m.reference.clone();
                    row(
                        m.model.name.clone(),
                        Some(s::context_model(
                            &m.provider_name,
                            m.model.context_window.unwrap_or(ROUTING_DEFAULT),
                        )),
                        self.context_control(
                            format!("context-model-{index}"),
                            Target::One(reference.clone()),
                            &[snapshot.prefs.context_override(&reference)],
                            r,
                            cx,
                        ),
                        r,
                    )
                })
                .collect::<Vec<_>>()
        });
        Some(
            v_flex()
                .child(row(
                    s::CONTEXT_ALL,
                    Some(description),
                    self.context_control("context-all".into(), Target::All, &overrides, r, cx),
                    r,
                ))
                .child(notice(s::CONTEXT_NOTE.to_owned(), false, r))
                .child(each)
                .children(rows.into_iter().flatten())
                .into_any_element(),
        )
    }
}
