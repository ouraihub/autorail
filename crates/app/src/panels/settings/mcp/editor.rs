//! MCP server editor: opening, saving and rendering the add/edit form.

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        input::{Input, InputState, Textarea, TextareaState},
    },
    prelude::FluentBuilder as _,
};
use thinkrail_core::{EnvValue, McpScope, McpServer, McpTransport};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, Segment, label},
    metrics::{radius, space, text},
};

use super::{
    Editor, McpPage, PairFields, TransportKind, ViewScope,
    form::{Form, to_input},
};
use crate::{
    state::{AppEvent, run},
    strings::settings_mcp as s,
};

impl McpPage {
    pub(super) fn begin_edit(
        &mut self,
        server: Option<McpServer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input =
            |cx: &mut Context<Self>, window: &mut Window| cx.new(|cx| InputState::new(window, cx));
        let area = |cx: &mut Context<Self>, window: &mut Window| {
            cx.new(|cx| TextareaState::new(window, cx).rows(3))
        };
        let scope = server.as_ref().map_or_else(
            || match self.scope {
                ViewScope::Global => McpScope::Global,
                ViewScope::Project => self
                    .project(cx)
                    .map_or(McpScope::Global, |project| McpScope::Project { project }),
            },
            |server| server.input.scope.clone(),
        );
        let (name, transport, command, args, url, pairs, description, enabled, id, revision) =
            server.as_ref().map_or_else(
                || {
                    (
                        String::new(),
                        TransportKind::Stdio,
                        String::new(),
                        String::new(),
                        String::new(),
                        Vec::<(String, String)>::new(),
                        String::new(),
                        true,
                        None,
                        0,
                    )
                },
                |server| match &server.input.transport {
                    McpTransport::Stdio { command, args, env } => (
                        server.input.name.clone(),
                        TransportKind::Stdio,
                        command.clone(),
                        args.join("\n"),
                        String::new(),
                        env.iter()
                            .map(|pair| (pair.name.clone(), pair.value.clone()))
                            .collect(),
                        server.input.description.clone().unwrap_or_default(),
                        server.input.enabled,
                        Some(server.id.clone()),
                        server.revision,
                    ),
                    McpTransport::Http { url, headers } => (
                        server.input.name.clone(),
                        TransportKind::Http,
                        String::new(),
                        String::new(),
                        url.clone(),
                        headers
                            .iter()
                            .map(|pair| (pair.name.clone(), pair.value.clone()))
                            .collect(),
                        server.input.description.clone().unwrap_or_default(),
                        server.input.enabled,
                        Some(server.id.clone()),
                        server.revision,
                    ),
                },
            );
        let mut env_pairs = Vec::new();
        let mut header_pairs = Vec::new();
        for (key, value) in pairs {
            let key_field = input(cx, window);
            let value_field = input(cx, window);
            set_input(&key_field, key, window, cx);
            set_input(&value_field, value, window, cx);
            let pair = PairFields {
                key: key_field,
                value: value_field,
            };
            if transport == TransportKind::Stdio {
                env_pairs.push(pair);
            } else {
                header_pairs.push(pair);
            }
        }
        let editor = Editor {
            id,
            revision,
            name: input(cx, window),
            command: input(cx, window),
            url: input(cx, window),
            args: area(cx, window),
            env: env_pairs,
            headers: header_pairs,
            description: input(cx, window),
            transport,
            enabled,
            scope,
            saving: false,
        };
        set_input(&editor.name, name, window, cx);
        set_input(&editor.command, command, window, cx);
        set_input(&editor.url, url, window, cx);
        set_area(&editor.args, args, window, cx);
        set_input(&editor.description, description, window, cx);
        self.editor = Some(editor);
        self.error = None;
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        if editor.saving {
            return;
        }
        let value = |field: &Entity<InputState>| field.read(cx).value().to_string();
        let area = |field: &Entity<TextareaState>| field.read(cx).value().to_string();
        let pairs = |fields: &[PairFields]| {
            fields
                .iter()
                .map(|pair| EnvValue {
                    name: value(&pair.key),
                    value: value(&pair.value),
                })
                .collect()
        };
        let form = Form {
            name: value(&editor.name),
            transport: editor.transport,
            command: value(&editor.command),
            args: area(&editor.args),
            url: value(&editor.url),
            env: pairs(&editor.env),
            headers: pairs(&editor.headers),
            description: value(&editor.description),
            enabled: editor.enabled,
            scope: editor.scope.clone(),
        };
        let input = match to_input(&form) {
            Ok(input) => input,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        editor.saving = true;
        let id = editor.id.clone();
        let revision = editor.revision;
        let pending_name = editor.name.clone();
        let service = self.state.read(cx).services.mcp.clone();
        run(
            cx,
            move || match id {
                Some(id) => service.update(&id, revision, input).map(drop),
                None => service.create(input).map(drop),
            },
            move |this, result, cx| {
                let same_editor = this
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.name == pending_name);
                match result {
                    Ok(()) => {
                        if same_editor {
                            this.editor = None;
                        }
                        this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                        this.reload(cx);
                    }
                    Err(error) => {
                        if same_editor && let Some(editor) = &mut this.editor {
                            editor.saving = false;
                        }
                        this.state.update(cx, |state, cx| state.report(&error, cx));
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn editor(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let Some(editor) = &self.editor else {
            return v_flex().into_any_element();
        };
        let field = |title: &'static str, input: &Entity<InputState>| {
            v_flex()
                .gap(space::S2)
                .child(label(text::TITLE_COMPACT, title).text_color(r.text_muted))
                .child(Input::new(input))
        };
        let area = |title: &'static str, input: &Entity<TextareaState>| {
            v_flex()
                .gap(space::S2)
                .child(label(text::TITLE_COMPACT, title).text_color(r.text_muted))
                .child(Textarea::new(input))
        };
        let pairs = |title: &'static str, fields: &Vec<PairFields>| {
            v_flex()
                .gap(space::S4)
                .child(label(text::TITLE_COMPACT, title).text_color(r.text_muted))
                .children(fields.iter().enumerate().map(|(index, pair)| {
                    h_flex()
                        .gap(space::S4)
                        .child(Input::new(&pair.key))
                        .child(Input::new(&pair.value))
                        .child(
                            IconButton::new(format!("mcp-remove-{title}-{index}"), IconName::Trash)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(editor) = &mut this.editor {
                                        let target = if title == s::ENV {
                                            &mut editor.env
                                        } else {
                                            &mut editor.headers
                                        };
                                        if index < target.len() {
                                            target.remove(index);
                                            cx.notify();
                                        }
                                    }
                                })),
                        )
                }))
                .child(
                    Button::new(format!("mcp-add-{title}"))
                        .ghost()
                        .label(s::ADD)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(editor) = &mut this.editor {
                                let pair = PairFields {
                                    key: cx.new(|cx| InputState::new(window, cx)),
                                    value: cx.new(|cx| InputState::new(window, cx)),
                                };
                                if title == s::ENV {
                                    editor.env.push(pair);
                                } else {
                                    editor.headers.push(pair);
                                }
                                cx.notify();
                            }
                        })),
                )
        };
        v_flex()
            .gap(space::S8)
            .p(space::S12)
            .rounded(radius::MD)
            .bg(r.container_content_bg)
            .child(label(
                text::TITLE_CARD,
                if editor.id.is_some() { s::EDIT } else { s::ADD },
            ))
            .child(field(s::NAME, &editor.name))
            .child(label(text::METADATA, s::NAME_HINT).text_color(r.text_subtle))
            .child(
                Segment::new(
                    "mcp-transport",
                    [s::STDIO, s::HTTP],
                    usize::from(editor.transport == TransportKind::Http),
                )
                .on_select(cx.listener(|this, index: &usize, _, cx| {
                    if let Some(editor) = &mut this.editor {
                        editor.transport = if *index == 0 {
                            TransportKind::Stdio
                        } else {
                            TransportKind::Http
                        };
                        cx.notify();
                    }
                })),
            )
            .when(editor.transport == TransportKind::Stdio, |d| {
                d.child(field(s::COMMAND, &editor.command))
                    .child(area(s::ARGS, &editor.args))
                    .child(pairs(s::ENV, &editor.env))
            })
            .when(editor.transport == TransportKind::Http, |d| {
                d.child(field(s::URL, &editor.url))
                    .child(label(text::METADATA, s::URL_HINT).text_color(r.text_subtle))
                    .child(pairs(s::HEADERS, &editor.headers))
            })
            .child(label(text::METADATA, s::VALUE_HINT).text_color(r.text_subtle))
            .child(field(s::DETAILS, &editor.description))
            .child(
                Segment::new(
                    "mcp-enabled",
                    [s::ENABLED, s::DISABLED],
                    usize::from(!editor.enabled),
                )
                .on_select(cx.listener(|this, index: &usize, _, cx| {
                    if let Some(editor) = &mut this.editor {
                        editor.enabled = *index == 0;
                        cx.notify();
                    }
                })),
            )
            .child(
                h_flex()
                    .gap(space::S8)
                    .justify_end()
                    .child(
                        Button::new("mcp-cancel")
                            .ghost()
                            .label(s::CANCEL)
                            .disabled(editor.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editor = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("mcp-save")
                            .primary()
                            .label(s::SAVE)
                            .disabled(editor.saving)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    ),
            )
            .into_any_element()
    }
}

pub(super) fn set_input(
    field: &Entity<InputState>,
    value: String,
    window: &mut Window,
    cx: &mut Context<impl Sized>,
) {
    field.update(cx, |input, cx| input.set_value(value, window, cx));
}
fn set_area(
    field: &Entity<TextareaState>,
    value: String,
    window: &mut Window,
    cx: &mut Context<impl Sized>,
) {
    field.update(cx, |input, cx| input.set_value(value, window, cx));
}
