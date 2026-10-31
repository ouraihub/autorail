//! 服务商设置（新建 / 改名 / 接入方式 / 删除）与线路设置（协议、API 地址、请求头）对话框。

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Icon, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputState},
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{
    Access, Endpoint, Header, LINE_PROTOCOLS, Protocol, Provider, ProviderInput, free_protocols, remove_line,
    retarget_line, set_line,
};
use thinkrail_routing::preview;
use thinkrail_ui::{ThinkrailTheme, components::{IconButton, Segment, label}, metrics::{space, text}};

use super::{ProvidersPage, error_text, field, open_form, valid_url};
use crate::{state::run, strings::settings_providers as s};

/// Full-width protocol dropdown with the current choice checked; `set` stores the pick in `form`.
fn protocol_menu<V: 'static>(
    id: &'static str,
    current: Protocol,
    options: Vec<Protocol>,
    form: Entity<V>,
    set: fn(&mut V, Protocol),
) -> impl IntoElement {
    // A select look: text left, chevron right (a Button centers label + icon).
    let trigger = h_flex().w_full().child(label(text::UI, s::protocol(current))).child(gpui_kit::div().flex_1()).child(Icon::new(IconName::ChevronDown).small());
    Button::new(id).outline().w_full().child(trigger).dropdown_menu(move |menu, _, _| {
        options.iter().fold(menu, |menu, &protocol| {
            let form = form.clone();
            menu.item(PopupMenuItem::new(s::protocol(protocol)).checked(protocol == current)
                .on_click(move |_, _, cx| form.update(cx, |view, cx| { set(view, protocol); cx.notify(); })))
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
        let protocol = protocol_menu("new-protocol", self.protocol, LINE_PROTOCOLS.to_vec(), cx.entity(), |form: &mut NameForm, p| form.protocol = p);
        let access = usize::from(self.access == Access::Gateway);
        v_flex().gap(space::S12)
            .child(field(s::NAME, Input::new(&self.name), cx))
            .when(creating, |d| d
                .child(field(s::PROTOCOL, protocol, cx))
                .child(field(s::BASE_URL, Input::new(&self.url), cx)))
            .child(field(s::ACCESS, Segment::new("provider-access", [s::DIRECT, s::GATEWAY], access)
                .on_select(cx.listener(|this, index: &usize, _, cx| {
                    this.access = if *index == 1 { Access::Gateway } else { Access::Direct };
                    cx.notify();
                })), cx))
            .child(label(text::METADATA, s::GATEWAY_HELP).text_color(r.text_subtle))
            .children(error_text(self.error.clone(), cx))
    }
}

impl ProvidersPage {
    pub(super) fn open_name_dialog(&mut self, provider: Option<Provider>, window: &mut Window, cx: &mut Context<Self>) {
        let name = provider.as_ref().map(|p| p.input.name.clone()).unwrap_or_default();
        let access = provider.as_ref().map_or(Access::Direct, |p| p.input.access);
        let form = cx.new(|cx| NameForm {
            name: cx.new(|cx| InputState::new(window, cx).default_value(name)),
            url: cx.new(|cx| InputState::new(window, cx).placeholder(s::URL_PLACEHOLDER)),
            protocol: Protocol::OpenaiChat, access, error: None, provider,
        });
        let page = cx.entity();
        let title = if form.read(cx).provider.is_some() { s::SETTINGS } else { s::ADD };
        open_form(window, cx, title, px(440.), form, s::CONFIRM, move |form, _, cx| {
            let name = form.name.read(cx).value().trim().to_owned();
            if name.is_empty() {
                form.error = Some(s::NAME_REQUIRED.into());
                cx.notify();
                return false;
            }
            let access = form.access;
            if form.provider.is_some() {
                page.update(cx, |this, cx| this.edit(cx, |d| { d.name = name; d.access = access; }));
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
                name, endpoints: vec![Endpoint { protocol, base_url: url }], default_protocol: protocol,
                headers: Vec::new(), access, enabled: true, models: Vec::new(),
            };
            page.update(cx, |this, cx| {
                let service = this.service(cx);
                run(cx, move || service.create(input), |this, result, cx| match result {
                    Ok(provider) => { let id = provider.id.clone(); this.stored(provider, cx); this.select(id, cx); }
                    Err(error) => this.report(&error, cx),
                });
            });
            true
        });
    }

    pub(super) fn confirm_delete(&mut self, provider: Provider, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (page, provider) = (page.clone(), provider.clone());
            alert.title(s::DELETE_PROVIDER).description(s::delete_confirm(&provider.input.name)).ok_text(s::DELETE).show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let (id, revision) = (provider.id.clone(), provider.revision);
                    page.update(cx, |this, cx| {
                        let service = this.service(cx);
                        run(cx, move || service.delete(&id, revision).map(|()| id), |this, result, cx| match result {
                            Ok(id) => {
                                this.providers.retain(|p| p.id != id);
                                if this.selected.as_ref() == Some(&id) {
                                    this.selected = None;
                                    this.draft = None;
                                    if let Some(first) = this.providers.first().map(|p| p.id.clone()) { this.select(first, cx); }
                                }
                                this.state.update(cx, |_, cx| cx.emit(crate::state::AppEvent::Routing));
                                cx.notify();
                            }
                            Err(error) => this.report(&error, cx),
                        });
                    });
                    true
                })
        });
    }
}

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
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(s::HEADER_NAME).default_value(header.name));
        let value = cx.new(|cx| InputState::new(window, cx).placeholder(s::HEADER_VALUE).default_value(header.value));
        self.headers.push((name, value));
    }

    fn result(&self, cx: &gpui_kit::App) -> Result<(String, Vec<Header>), &'static str> {
        let url = self.url.read(cx).value().trim().to_owned();
        if !valid_url(&url) { return Err(s::URL_INVALID); }
        let mut headers = Vec::new();
        for (name, value) in &self.headers {
            let (name, value) = (name.read(cx).value().trim().to_owned(), value.read(cx).value().trim().to_owned());
            if name.is_empty() && value.is_empty() { continue; }
            if name.is_empty() || value.is_empty() { return Err(s::HEADER_INVALID); }
            headers.push(Header { name, value });
        }
        Ok((url, headers))
    }
}

impl Render for LineForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let protocol = protocol_menu("line-protocol", self.protocol, self.options.clone(), cx.entity(), |form: &mut LineForm, p| form.protocol = p);
        let url = self.url.read(cx).value().trim().to_owned();
        let _ = window;
        v_flex().gap(space::S12)
            .child(field(s::PROTOCOL, protocol, cx))
            .child(field(s::BASE_URL, Input::new(&self.url), cx))
            .when(valid_url(&url), |d| d.child(label(text::CODE_SMALL, s::request_post(&preview(&url, self.protocol))).text_color(r.text_subtle)))
            .child(label(text::METADATA, s::URL_HINT).text_color(r.text_subtle))
            .child(field(s::HEADERS, v_flex().gap(space::S6)
                .children(self.headers.iter().enumerate().map(|(index, (name, value))| h_flex().gap(space::S6)
                    .child(Input::new(name).flex_1()).child(Input::new(value).flex_1())
                    .child(IconButton::new(("line-header-remove", index), gpui_kit::assets::IconName::Minus)
                        .on_click(cx.listener(move |this, _, _, cx| { this.headers.remove(index); cx.notify(); })))))
                .child(Button::new("line-header-add").ghost().small().label(s::HEADERS_ADD)
                    .on_click(cx.listener(|this, _, window, cx| { this.add_header(Header { name: String::new(), value: String::new() }, window, cx); cx.notify(); })))
                .child(label(text::METADATA, s::HEADERS_SHARED).text_color(r.text_subtle)), cx))
            .children(error_text(self.error.map(str::to_owned), cx))
            .when(self.editing.is_some(), |d| d.child(h_flex().gap(space::S8)
                .when(!self.is_default, |d| d.child(Button::new("line-default").outline().small().label(s::SET_DEFAULT_LINE)
                    .on_click(cx.listener(|this, _, _, cx| { this.is_default = true; cx.notify(); }))))
                .when(self.is_default, |d| d.child(label(text::METADATA, s::DEFAULT_BADGE).text_color(r.primary)))
                .child(gpui_kit::div().flex_1())
                .child(Button::new("line-remove").danger().small().label(s::REMOVE_LINE).disabled(!self.can_remove)
                    .tooltip(if self.can_remove { s::REMOVE_LINE } else { s::LAST_LINE })
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_remove(window, cx))))))
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
    pub(super) fn open_line_dialog(&mut self, editing: Option<Protocol>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_ref() else { return };
        let options: Vec<Protocol> = editing.into_iter().chain(free_protocols(draft)).collect();
        let Some(&protocol) = options.first() else { return };
        let url = editing.map(|_| self.line_url()).unwrap_or_default();
        let headers = draft.headers.clone();
        let is_default = editing == Some(draft.default_protocol);
        let can_remove = draft.endpoints.len() > 1;
        let page = cx.entity();
        let form = cx.new(|cx| {
            let mut form = LineForm {
                page: page.clone(), editing, options, protocol, is_default, can_remove,
                url: cx.new(|cx| InputState::new(window, cx).placeholder(s::URL_PLACEHOLDER).default_value(url)),
                headers: Vec::new(), error: None,
            };
            for header in headers { form.add_header(header, window, cx); }
            form
        });
        let title = if editing.is_some() { s::LINE_SETTINGS } else { s::ADD_LINE };
        open_form(window, cx, title, px(520.), form, s::CONFIRM, move |form, _, cx| match form.result(cx) {
            Ok((url, headers)) => {
                let (from, to, is_default) = (form.editing, form.protocol, form.is_default);
                page.update(cx, |this, cx| {
                    this.edit(cx, |d| {
                        if let Some(from) = from { retarget_line(d, from, to); }
                        set_line(d, to, url);
                        d.headers = headers;
                        if is_default { d.default_protocol = to; }
                    });
                    this.line = to;
                    this.sync_inputs = true;
                });
                true
            }
            Err(error) => { form.error = Some(error); cx.notify(); false }
        });
    }

    fn open_remove_line(&mut self, protocol: Protocol, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let page = page.clone();
            alert.title(s::REMOVE_LINE).description(s::remove_line_confirm(protocol)).ok_text(s::DELETE).show_cancel(true)
                .on_ok(move |_, _, cx| {
                    page.update(cx, |this, cx| {
                        this.edit(cx, |d| { remove_line(d, protocol); });
                        if let Some(d) = &this.draft { this.line = d.default_protocol; }
                        this.sync_inputs = true;
                        this.models.reset();
                    });
                    true
                })
        });
    }
}
