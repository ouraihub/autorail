//! 选中服务商的详情：标题行、API 密钥与检测、密钥使用方式、API 地址与线路、模型。

use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Sizable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
        switch::Switch,
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_routing::models_url;
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, label},
    metrics::{space, text},
};

use super::{ProvidersPage, error_text};
use crate::strings::settings_providers as s;

mod key;

impl ProvidersPage {
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let (Some(provider), Some(draft)) = (self.provider().cloned(), self.draft.as_ref()) else {
            let message = if self.loaded {
                (s::EMPTY, s::EMPTY_HINT)
            } else {
                (s::LOADING, "")
            };
            return v_flex()
                .flex_1()
                .items_center()
                .py(space::S40)
                .gap(space::S4)
                .child(label(text::TITLE_SECTION, message.0))
                .child(label(text::METADATA, message.1).text_color(r.text_subtle))
                .into_any_element();
        };
        let enabled = draft.enabled;
        let failed = self.failed.clone().map(|error| {
            h_flex()
                .gap(space::S8)
                .child(
                    label(text::METADATA, format!("{} {error}", s::SAVE_FAILED))
                        .text_color(r.feedback_error),
                )
                .child(
                    Button::new("provider-retry")
                        .link()
                        .label(s::RETRY)
                        .on_click(cx.listener(|this, _, _, cx| this.commit(cx))),
                )
        });
        v_flex()
            .flex_1()
            .min_w(px(0.))
            .gap(space::S16)
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(label(text::HEADING_SM, draft.name.clone()).truncate())
                    .child(
                        IconButton::new("provider-settings", IconName::Settings2)
                            .tooltip(s::SETTINGS)
                            .on_click(cx.listener({
                                let p = provider.clone();
                                move |this, _, window, cx| {
                                    this.open_name_dialog(Some(p.clone()), window, cx)
                                }
                            })),
                    )
                    .child(gpui_kit::div().flex_1())
                    .child(
                        Switch::new("provider-enabled")
                            .checked(enabled)
                            .color(r.feedback_success)
                            .on_click(cx.listener(|this, value: &bool, _, cx| {
                                let value = *value;
                                this.edit(cx, |d| d.enabled = value)
                            })),
                    ),
            )
            .children(failed)
            .child(self.render_key(&provider, cx))
            .child(
                v_flex()
                    .gap(space::S4)
                    .child(label(text::TITLE_COMPACT, s::KEY_STRATEGY).text_color(r.text_muted))
                    .child(label(text::UI, s::KEY_STRATEGY_ROUND_ROBIN))
                    .child(
                        label(text::METADATA, s::KEY_STRATEGY_ROUND_ROBIN_DESC)
                            .text_color(r.text_subtle),
                    ),
            )
            .child(self.render_url(cx))
            .child(self.render_models(cx))
            .into_any_element()
    }
}

impl ProvidersPage {
    fn render_url(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let Some(draft) = self.draft.as_ref() else {
            return gpui_kit::div().into_any_element();
        };
        let can_add = !thinkrail_core::free_protocols(draft).is_empty();
        let lines = draft.endpoints.clone();
        let current = self.line;
        let page = cx.entity();
        let picker = (lines.len() > 1).then(|| {
            Button::new("provider-line")
                .outline()
                .w_full()
                .label(format!(
                    "{} · {}",
                    s::protocol_short(current),
                    self.line_url()
                ))
                .icon(IconName::ChevronDown)
                .dropdown_menu(move |menu, _, _| {
                    lines.iter().fold(menu, |menu, line| {
                        let (protocol, page) = (line.protocol, page.clone());
                        menu.item(
                            PopupMenuItem::new(format!(
                                "{} · {}",
                                s::protocol_short(protocol),
                                line.base_url
                            ))
                            .checked(protocol == current)
                            .on_click(move |_, _, cx| {
                                page.update(cx, |this, cx| {
                                    this.line = protocol;
                                    this.sync_inputs = true;
                                    this.models.reset();
                                    cx.notify();
                                })
                            }),
                        )
                    })
                })
        });
        let url = self.line_url();
        let hint = if self.url_error {
            error_text(Some(s::URL_INVALID.into()), cx).map(IntoElement::into_any_element)
        } else {
            (!url.is_empty()).then(|| {
                label(text::CODE_SMALL, s::probe_get(&models_url(&url, current)))
                    .text_color(r.text_subtle)
                    .truncate()
                    .into_any_element()
            })
        };
        v_flex()
            .gap(space::S6)
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(label(text::TITLE_COMPACT, s::BASE_URL).text_color(r.text_muted))
                    .when(can_add, |d| {
                        d.child(
                            Button::new("provider-add-line")
                                .link()
                                .xsmall()
                                .label(s::ADD_LINE)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_line_dialog(None, window, cx)
                                })),
                        )
                    }),
            )
            .children(picker)
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(Input::new(&self.url).flex_1())
                    .child(
                        Button::new("provider-line-settings")
                            .outline()
                            .icon(IconName::Settings2)
                            .tooltip(s::LINE_SETTINGS_TIP)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_line_dialog(Some(current), window, cx)
                            })),
                    ),
            )
            .children(hint)
            .into_any_element()
    }
}
