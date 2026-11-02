//! 服务商设置（新建 / 改名 / 接入方式 / 删除）与线路设置（协议、API 地址、请求头）对话框。

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Icon, Sizable as _, WindowExt as _,
        button::Button,
        input::{Input, InputState},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{Access, Endpoint, LINE_PROTOCOLS, Protocol, Provider, ProviderInput};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{Segment, label},
    metrics::{space, text},
};

use super::{ProvidersPage, error_text, field, open_form, valid_url};
use crate::{state::run, strings::settings_providers as s};

mod line;

/// Full-width protocol dropdown with the current choice checked; `set` stores the pick in `form`.
fn protocol_menu<V: 'static>(
    id: &'static str,
    current: Protocol,
    options: Vec<Protocol>,
    form: Entity<V>,
    set: fn(&mut V, Protocol),
) -> impl IntoElement {
    // A select look: text left, chevron right (a Button centers label + icon).
    let trigger = h_flex()
        .w_full()
        .child(label(text::UI, s::protocol(current)))
        .child(gpui_kit::div().flex_1())
        .child(Icon::new(IconName::ChevronDown).small());
    Button::new(id)
        .outline()
        .w_full()
        .child(trigger)
        .dropdown_menu(move |menu, _, _| {
            options.iter().fold(menu, |menu, &protocol| {
                let form = form.clone();
                menu.item(
                    PopupMenuItem::new(s::protocol(protocol))
                        .checked(protocol == current)
                        .on_click(move |_, _, cx| {
                            form.update(cx, |view, cx| {
                                set(view, protocol);
                                cx.notify();
                            })
                        }),
                )
            })
        })
}

/// 新建（`provider` 为 None，含第一条线路）或编辑服务商的名称与接入方式。
struct NameForm {
    provider: Option<Provider>,
    name: Entity<InputState>,
    url: Entity<InputState>,
    protocol: Protocol,
    access: Access,
    error: Option<String>,
}

impl Render for NameForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let creating = self.provider.is_none();
        let protocol = protocol_menu(
            "new-protocol",
            self.protocol,
            LINE_PROTOCOLS.to_vec(),
            cx.entity(),
            |form: &mut NameForm, p| form.protocol = p,
        );
        let access = usize::from(self.access == Access::Gateway);
        v_flex()
            .gap(space::S12)
            .child(field(s::NAME, Input::new(&self.name), cx))
            .when(creating, |d| {
                d.child(field(s::PROTOCOL, protocol, cx)).child(field(
                    s::BASE_URL,
                    Input::new(&self.url),
                    cx,
                ))
            })
            .child(field(
                s::ACCESS,
                Segment::new("provider-access", [s::DIRECT, s::GATEWAY], access).on_select(
                    cx.listener(|this, index: &usize, _, cx| {
                        this.access = if *index == 1 {
                            Access::Gateway
                        } else {
                            Access::Direct
                        };
                        cx.notify();
                    }),
                ),
                cx,
            ))
            .child(label(text::METADATA, s::GATEWAY_HELP).text_color(r.text_subtle))
            .children(error_text(self.error.clone(), cx))
    }
}

impl ProvidersPage {
    pub(super) fn open_name_dialog(
        &mut self,
        provider: Option<Provider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = provider
            .as_ref()
            .map(|p| p.input.name.clone())
            .unwrap_or_default();
        let access = provider.as_ref().map_or(Access::Direct, |p| p.input.access);
        let form = cx.new(|cx| NameForm {
            name: cx.new(|cx| InputState::new(window, cx).default_value(name)),
            url: cx.new(|cx| InputState::new(window, cx).placeholder(s::URL_PLACEHOLDER)),
            protocol: Protocol::OpenaiChat,
            access,
            error: None,
            provider,
        });
        let page = cx.entity();
        let title = if form.read(cx).provider.is_some() {
            s::SETTINGS
        } else {
            s::ADD
        };
        open_form(
            window,
            cx,
            title,
            px(440.),
            form,
            s::CONFIRM,
            move |form, _, cx| {
                let name = form.name.read(cx).value().trim().to_owned();
                if name.is_empty() {
                    form.error = Some(s::NAME_REQUIRED.into());
                    cx.notify();
                    return false;
                }
                let access = form.access;
                if form.provider.is_some() {
                    page.update(cx, |this, cx| {
                        this.edit(cx, |d| {
                            d.name = name;
                            d.access = access;
                        })
                    });
                    return true;
                }
                let url = form.url.read(cx).value().trim().to_owned();
                if !valid_url(&url) {
                    form.error = Some(s::URL_INVALID.into());
                    cx.notify();
                    return false;
                }
                let protocol = form.protocol;
                let input = ProviderInput {
                    name,
                    endpoints: vec![Endpoint {
                        protocol,
                        base_url: url,
                    }],
                    default_protocol: protocol,
                    headers: Vec::new(),
                    access,
                    enabled: true,
                    models: Vec::new(),
                };
                page.update(cx, |this, cx| {
                    let service = this.service(cx);
                    run(
                        cx,
                        move || service.create(input),
                        |this, result, cx| match result {
                            Ok(provider) => {
                                let id = provider.id.clone();
                                this.stored(provider, cx);
                                this.select(id, cx);
                            }
                            Err(error) => this.report(&error, cx),
                        },
                    );
                });
                true
            },
        );
    }

    pub(super) fn confirm_delete(
        &mut self,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (page, provider) = (page.clone(), provider.clone());
            alert
                .title(s::DELETE_PROVIDER)
                .description(s::delete_confirm(&provider.input.name))
                .ok_text(s::DELETE)
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let (id, revision) = (provider.id.clone(), provider.revision);
                    page.update(cx, |this, cx| {
                        let service = this.service(cx);
                        run(
                            cx,
                            move || service.delete(&id, revision).map(|()| id),
                            |this, result, cx| match result {
                                Ok(id) => {
                                    this.providers.retain(|p| p.id != id);
                                    if this.selected.as_ref() == Some(&id) {
                                        this.selected = None;
                                        this.draft = None;
                                        if let Some(first) =
                                            this.providers.first().map(|p| p.id.clone())
                                        {
                                            this.select(first, cx);
                                        }
                                    }
                                    this.state.update(cx, |_, cx| {
                                        cx.emit(crate::state::AppEvent::Routing)
                                    });
                                    cx.notify();
                                }
                                Err(error) => this.report(&error, cx),
                            },
                        );
                    });
                    true
                })
        });
    }
}
