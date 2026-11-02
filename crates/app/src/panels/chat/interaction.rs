//! Interaction cards: confirm, select, text and multi-question answers sent back to the engine.

use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Styled as _,
    Window,
    base::{h_flex, v_flex},
    component::{
        button::{Button, ButtonVariants as _},
        input::{Input, InputState},
    },
    prelude::FluentBuilder as _,
};
use serde_json::{Value, json};
use thinkrail_core::{Interaction, InteractionAnswer, InteractionKind};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{Status, StatusDot, label},
    metrics::{radius, space, text},
};

use super::ChatPanel;
use crate::strings::chat as s;

impl ChatPanel {
    pub(super) fn interaction_card(
        &mut self,
        interaction: Interaction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let id = interaction.request_id.clone();
        let cancel_id = id.clone();
        let title = match (interaction.kind, interaction.title.is_empty()) {
            (InteractionKind::Permission, true) => s::PERMISSION.to_owned(),
            (_, true) => s::QUESTION.to_owned(),
            (_, false) => interaction.title.clone(),
        };
        let mut actions: Vec<AnyElement> = Vec::new();
        let mut body: Vec<AnyElement> = Vec::new();
        if !interaction.message.is_empty() && interaction.kind == InteractionKind::Confirm {
            body.push(label(text::UI, interaction.message.clone()).into_any_element());
        }
        match interaction.kind {
            InteractionKind::Confirm => {
                for (confirmed, text_label) in [(true, s::YES), (false, s::NO)] {
                    let rid = id.clone();
                    actions.push(
                        Button::new(format!("confirm-{id}-{confirmed}"))
                            .label(text_label)
                            .when(confirmed, |b| b.primary())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.respond(
                                    rid.clone(),
                                    InteractionAnswer::Confirm { confirmed },
                                    cx,
                                )
                            }))
                            .into_any_element(),
                    );
                }
            }
            InteractionKind::Select | InteractionKind::Permission => {
                for option in &interaction.options {
                    let (rid, option_id) = (id.clone(), option.id.clone());
                    actions.push(
                        Button::new(format!("select-{id}-{}", option.id))
                            .label(option.label.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.respond(
                                    rid.clone(),
                                    InteractionAnswer::Select {
                                        option_id: option_id.clone(),
                                    },
                                    cx,
                                )
                            }))
                            .into_any_element(),
                    );
                }
            }
            InteractionKind::Input | InteractionKind::Editor => {
                let multi = interaction.kind == InteractionKind::Editor;
                let input = self.answer_input(&id, s::OTHER, window, cx);
                if multi && !interaction.message.is_empty() && input.read(cx).value().is_empty() {
                    let prefill = interaction.message.clone();
                    input.update(cx, |state, cx| state.set_value(prefill, window, cx));
                }
                body.push(Input::new(&input).into_any_element());
                let rid = id.clone();
                actions.push(
                    Button::new(format!("submit-{id}"))
                        .label(s::SUBMIT)
                        .primary()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let text = input.read(cx).value().to_string();
                            this.respond(rid.clone(), InteractionAnswer::Text { text }, cx)
                        }))
                        .into_any_element(),
                );
            }
            InteractionKind::Ask => {
                let questions = interaction.detail["questions"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                for question in &questions {
                    body.push(self.question(&id, question, window, cx));
                }
                let rid = id.clone();
                actions.push(
                    Button::new(format!("ask-submit-{id}"))
                        .label(s::SUBMIT)
                        .primary()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let answers = this.ask_answers(&rid, &questions, cx);
                            this.respond(rid.clone(), InteractionAnswer::Answers { answers }, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        actions.push(
            Button::new(format!("cancel-{id}"))
                .label(s::CANCEL)
                .ghost()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.respond(cancel_id.clone(), InteractionAnswer::Cancel, cx)
                }))
                .into_any_element(),
        );
        v_flex()
            .gap(space::S8)
            .p(space::S12)
            .rounded(radius::MD)
            .border_1()
            .border_color(r.feedback_warning_muted)
            .bg(r.feedback_warning_subtle)
            .child(
                h_flex()
                    .gap(space::S6)
                    .child(StatusDot::new(Status::Warning))
                    .child(label(text::TITLE_COMPACT, title).text_color(r.text_default)),
            )
            .children(body)
            .child(h_flex().flex_wrap().gap(space::S6).children(actions))
            .into_any_element()
    }

    fn question(
        &mut self,
        request: &str,
        question: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let qid = question["id"].as_str().unwrap_or_default().to_owned();
        let key = format!("{request}/{qid}");
        let multi = question["multi"] == true;
        let picked = self.picks.get(&key).cloned().unwrap_or_default();
        let options: Vec<AnyElement> = question["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| o["label"].as_str().map(str::to_owned))
            .enumerate()
            .map(|(index, option)| {
                let (key, chosen) = (key.clone(), picked.contains(&option));
                let value = option.clone();
                Button::new(format!("pick-{key}-{index}"))
                    .label(option)
                    .when(chosen, |b| b.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let picks = this.picks.entry(key.clone()).or_default();
                        match picks.iter().position(|p| p == &value) {
                            Some(at) => {
                                picks.remove(at);
                            }
                            None if multi => picks.push(value.clone()),
                            None => *picks = vec![value.clone()],
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let other = self.answer_input(&key, s::OTHER, window, cx);
        let title = question["header"]
            .as_str()
            .map(|h| format!("{h} · "))
            .unwrap_or_default()
            + question["question"].as_str().unwrap_or_default();
        v_flex()
            .gap(space::S6)
            .child(label(text::UI, title).text_color(r.text_default))
            .child(h_flex().flex_wrap().gap(space::S6).children(options))
            .child(Input::new(&other))
            .into_any_element()
    }

    fn ask_answers(&self, request: &str, questions: &[Value], cx: &Context<Self>) -> Value {
        let answers: Vec<Value> = questions
            .iter()
            .map(|question| {
                let qid = question["id"].as_str().unwrap_or_default();
                let key = format!("{request}/{qid}");
                let mut answer = json!({"id": qid, "selectedOptions": self.picks.get(&key).cloned().unwrap_or_default()});
                if let Some(text) = self.answers.get(&key).map(|i| i.read(cx).value().trim().to_owned()).filter(|t| !t.is_empty()) {
                    answer["customInput"] = json!(text);
                }
                answer
            })
            .collect();
        Value::Array(answers)
    }

    fn answer_input(
        &mut self,
        key: &str,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = self.answers.get(key) {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        self.answers.insert(key.to_owned(), input.clone());
        input
    }
}
