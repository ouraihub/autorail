//! 线路设置对话框（协议、API 地址、请求头）与移除线路确认。

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputState},
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{Header, Protocol, free_protocols, remove_line, retarget_line, set_line};
use thinkrail_routing::preview;
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, label},
    metrics::{space, text},
};

use super::protocol_menu;
use crate::{
    panels::settings::providers::{ProvidersPage, error_text, field, open_form, valid_url},
    strings::settings_providers as s,
};

/// 线路设置：协议、API 地址、请求头（请求头对整个服务商生效）。`editing` 为 None 时新建线路。
struct LineForm {
    page: Entity<ProvidersPage>,
    editing: Option<Protocol>,
    options: Vec<Protocol>,
    protocol: Protocol,
    is_default: bool,
    can_remove: bool,
    url: Entity<InputState>,
    headers: Vec<(Entity<InputState>, Entity<InputState>)>,
    error: Option<&'static str>,
}

impl LineForm {
    fn add_header(&mut self, header: Header, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(s::HEADER_NAME)
                .default_value(header.name)
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(s::HEADER_VALUE)
                .default_value(header.value)
        });
        self.headers.push((name, value));
    }

    fn result(&self, cx: &gpui_kit::App) -> Result<(String, Vec<Header>), &'static str> {
        let url = self.url.read(cx).value().trim().to_owned();
        if !valid_url(&url) {
            return Err(s::URL_INVALID);
        }
        let mut headers = Vec::new();
        for (name, value) in &self.headers {
            let (name, value) = (
                name.read(cx).value().trim().to_owned(),
                value.read(cx).value().trim().to_owned(),
            );
            if name.is_empty() && value.is_empty() {
                continue;
            }
            if name.is_empty() || value.is_empty() {
                return Err(s::HEADER_INVALID);
            }
            headers.push(Header { name, value });
        }
        Ok((url, headers))
    }
}

impl Render for LineForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let protocol = protocol_menu(
            "line-protocol",
            self.protocol,
            self.options.clone(),
            cx.entity(),
            |form: &mut LineForm, p| form.protocol = p,
        );
        let url = self.url.read(cx).value().trim().to_owned();
        let _ = window;
        v_flex()
            .gap(space::S12)
            .child(field(s::PROTOCOL, protocol, cx))
            .child(field(s::BASE_URL, Input::new(&self.url), cx))
            .when(valid_url(&url), |d| {
                d.child(
                    label(
                        text::CODE_SMALL,
                        s::request_post(&preview(&url, self.protocol)),
                    )
                    .text_color(r.text_subtle),
                )
            })
            .child(label(text::METADATA, s::URL_HINT).text_color(r.text_subtle))
            .child(field(
                s::HEADERS,
                v_flex()
                    .gap(space::S6)
                    .children(
                        self.headers
                            .iter()
                            .enumerate()
                            .map(|(index, (name, value))| {
                                h_flex()
                                    .gap(space::S6)
                                    .child(Input::new(name).flex_1())
                                    .child(Input::new(value).flex_1())
                                    .child(
                                        IconButton::new(
                                            ("line-header-remove", index),
                                            gpui_kit::assets::IconName::Minus,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.headers.remove(index);
                                                cx.notify();
                                            }),
                                        ),
                                    )
                            }),
                    )
                    .child(
                        Button::new("line-header-add")
                            .ghost()
                            .small()
                            .label(s::HEADERS_ADD)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_header(
                                    Header {
                                        name: String::new(),
                                        value: String::new(),
                                    },
                                    window,
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(label(text::METADATA, s::HEADERS_SHARED).text_color(r.text_subtle)),
                cx,
            ))
            .children(error_text(self.error.map(str::to_owned), cx))
            .when(self.editing.is_some(), |d| {
                d.child(
                    h_flex()
                        .gap(space::S8)
                        .when(!self.is_default, |d| {
                            d.child(
                                Button::new("line-default")
                                    .outline()
                                    .small()
                                    .label(s::SET_DEFAULT_LINE)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.is_default = true;
                                        cx.notify();
                                    })),
                            )
                        })
                        .when(self.is_default, |d| {
                            d.child(label(text::METADATA, s::DEFAULT_BADGE).text_color(r.primary))
                        })
                        .child(gpui_kit::div().flex_1())
                        .child(
                            Button::new("line-remove")
                                .danger()
                                .small()
                                .label(s::REMOVE_LINE)
                                .disabled(!self.can_remove)
                                .tooltip(if self.can_remove {
                                    s::REMOVE_LINE
                                } else {
                                    s::LAST_LINE
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_remove(window, cx)
                                })),
                        ),
                )
            })
    }
}

impl LineForm {
    fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(protocol) = self.editing else { return };
        let page = self.page.clone();
        window.close_dialog(cx);
        page.update(cx, |this, cx| this.open_remove_line(protocol, window, cx));
    }
}

impl ProvidersPage {
    pub(in crate::panels::settings::providers) fn open_line_dialog(
        &mut self,
        editing: Option<Protocol>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.draft.as_ref() else {
            return;
        };
        let options: Vec<Protocol> = editing.into_iter().chain(free_protocols(draft)).collect();
        let Some(&protocol) = options.first() else {
            return;
        };
        let url = editing.map(|_| self.line_url()).unwrap_or_default();
        let headers = draft.headers.clone();
        let is_default = editing == Some(draft.default_protocol);
        let can_remove = draft.endpoints.len() > 1;
        let page = cx.entity();
        let form = cx.new(|cx| {
            let mut form = LineForm {
                page: page.clone(),
                editing,
                options,
                protocol,
                is_default,
                can_remove,
                url: cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(s::URL_PLACEHOLDER)
                        .default_value(url)
                }),
                headers: Vec::new(),
                error: None,
            };
            for header in headers {
                form.add_header(header, window, cx);
            }
            form
        });
        let title = if editing.is_some() {
            s::LINE_SETTINGS
        } else {
            s::ADD_LINE
        };
        open_form(
            window,
            cx,
            title,
            px(520.),
            form,
            s::CONFIRM,
            move |form, _, cx| match form.result(cx) {
                Ok((url, headers)) => {
                    let (from, to, is_default) = (form.editing, form.protocol, form.is_default);
                    page.update(cx, |this, cx| {
                        this.edit(cx, |d| {
                            if let Some(from) = from {
                                retarget_line(d, from, to);
                            }
                            set_line(d, to, url);
                            d.headers = headers;
                            if is_default {
                                d.default_protocol = to;
                            }
                        });
                        this.line = to;
                        this.sync_inputs = true;
                    });
                    true
                }
                Err(error) => {
                    form.error = Some(error);
                    cx.notify();
                    false
                }
            },
        );
    }

    fn open_remove_line(
        &mut self,
        protocol: Protocol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let page = page.clone();
            alert
                .title(s::REMOVE_LINE)
                .description(s::remove_line_confirm(protocol))
                .ok_text(s::DELETE)
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    page.update(cx, |this, cx| {
                        this.edit(cx, |d| {
                            remove_line(d, protocol);
                        });
                        if let Some(d) = &this.draft {
                            this.line = d.default_protocol;
                        }
                        this.sync_inputs = true;
                        this.models.reset();
                    });
                    true
                })
        });
    }
}
