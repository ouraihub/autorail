//! MCP page rendering: scope switch, server rows, presets and the import panel.

use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _,
        button::{Button, ButtonVariants as _},
        input::Textarea,
        switch::Switch,
    },
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{McpOrigin, McpServer, McpTransport};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, Segment, label},
    metrics::{space, text},
};

use super::{McpPage, TransportKind, ViewScope, editor::set_input};
use crate::strings::settings_mcp as s;

impl McpPage {
    fn row(
        &self,
        server: McpServer,
        origin: McpOrigin,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let id = server.id.clone();
        let edit = server.clone();
        let toggle = server.clone();
        let delete = server.clone();
        let project_global = self.scope == ViewScope::Project && origin != McpOrigin::Project;
        let transport = match &server.input.transport {
            McpTransport::Stdio { command, .. } => format!("{} · {}", s::STDIO, command),
            McpTransport::Http { url, .. } => format!("{} · {}", s::HTTP, url),
        };
        let origin_label = match origin {
            McpOrigin::Global if self.scope == ViewScope::Global => s::GLOBAL,
            McpOrigin::Global => s::FROM_GLOBAL,
            McpOrigin::Project => s::PROJECT_SCOPE,
            McpOrigin::GlobalOverridden => s::OVERRIDDEN,
        };
        let confirming =
            self.confirm_delete.as_ref() == Some(&(server.id.clone(), server.revision));
        v_flex()
            .gap(space::S4)
            .py(space::S12)
            .border_b_1()
            .border_color(r.border_muted)
            .child(
                h_flex()
                    .id(format!("mcp-{id}"))
                    .gap(space::S8)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                h_flex()
                                    .gap(space::S6)
                                    .child(label(text::UI, server.input.name.clone()))
                                    .child(
                                        label(text::METADATA, origin_label)
                                            .text_color(r.text_subtle),
                                    ),
                            )
                            .child(label(text::METADATA, transport).text_color(r.text_subtle))
                            .when_some(server.input.description.clone(), |d, description| {
                                d.child(label(text::METADATA, description).text_color(r.text_muted))
                            }),
                    )
                    .child(
                        Switch::new(format!("mcp-enabled-{id}"))
                            .checked(enabled)
                            .color(r.feedback_success)
                            .disabled(origin == McpOrigin::GlobalOverridden)
                            .accessibility_label(server.input.name.clone())
                            .on_click(cx.listener(move |this, enabled: &bool, _, cx| {
                                this.toggle(toggle.clone(), *enabled, cx)
                            })),
                    )
                    .when(!project_global, |d| {
                        d.child(
                            IconButton::new(format!("mcp-edit-{id}"), IconName::Pencil)
                                .tooltip(s::EDIT)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.begin_edit(Some(edit.clone()), window, cx)
                                })),
                        )
                        .child(if confirming {
                            Button::new(format!("mcp-confirm-delete-{id}"))
                                .danger()
                                .label(s::CONFIRM_DELETE)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete(delete.clone(), cx)
                                }))
                                .into_any_element()
                        } else {
                            IconButton::new(format!("mcp-delete-{id}"), IconName::Trash)
                                .tooltip(s::DELETE)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete(delete.clone(), cx)
                                }))
                                .into_any_element()
                        })
                    }),
            )
            .when(confirming, |d| {
                d.child(
                    label(
                        text::METADATA,
                        format!("{} {}", server.input.name, s::REMOVE_WARNING),
                    )
                    .text_color(r.feedback_warning),
                )
            })
    }
}

impl Render for McpPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let project_label = self
            .state
            .read(cx)
            .current_project()
            .map(|project| format!("{}{}", s::PROJECT, project.name));
        let rows: Vec<_> = match self.scope {
            ViewScope::Global => self
                .global
                .clone()
                .into_iter()
                .map(|server| {
                    let enabled = server.input.enabled;
                    self.row(server, McpOrigin::Global, enabled, cx)
                        .into_any_element()
                })
                .collect(),
            ViewScope::Project => self
                .effective
                .clone()
                .into_iter()
                .map(|entry| {
                    self.row(entry.server, entry.origin, entry.enabled, cx)
                        .into_any_element()
                })
                .collect(),
        };
        let empty = rows.is_empty();
        let mut scopes = vec![s::GLOBAL.to_owned()];
        if let Some(project) = project_label {
            scopes.push(project);
        }
        v_flex()
            .id("settings-mcp")
            .gap(space::S12)
            .child(
                h_flex()
                    .child(label(text::HEADING_MD, s::TITLE).flex_1())
                    .child(
                        Button::new("mcp-add")
                            .primary()
                            .icon(IconName::Plus)
                            .label(s::ADD)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.begin_edit(None, window, cx)
                                }),
                            ),
                    ),
            )
            .child(label(text::METADATA, s::DESCRIPTION).text_color(r.text_subtle))
            .child(
                Segment::new(
                    "mcp-scope",
                    scopes,
                    usize::from(self.scope == ViewScope::Project),
                )
                .on_select(cx.listener(|this, index: &usize, _, cx| {
                    if *index == 1 && this.project(cx).is_none() {
                        return;
                    }
                    this.scope = if *index == 0 {
                        ViewScope::Global
                    } else {
                        ViewScope::Project
                    };
                    this.import_preview = None;
                    this.editor = None;
                    this.confirm_delete = None;
                    this.reload(cx);
                    cx.notify();
                })),
            )
            .when(self.editor.is_some(), |d| d.child(self.editor(cx)))
            .children(rows)
            .when(empty && !self.loading, |d| {
                d.child(label(text::METADATA, s::EMPTY).text_color(r.text_subtle))
            })
            .child(
                h_flex().gap(space::S8).children(
                    [
                        (s::CONTEXT7, "https://mcp.context7.com/mcp", "context7"),
                        (s::DEEPWIKI, "https://mcp.deepwiki.com/mcp", "deepwiki"),
                    ]
                    .into_iter()
                    .map(|(title, url, name)| {
                        Button::new(format!("mcp-preset-{name}"))
                            .ghost()
                            .label(title)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.begin_edit(None, window, cx);
                                if let Some(editor) = &mut this.editor {
                                    editor.transport = TransportKind::Http;
                                    set_input(&editor.name, name.to_owned(), window, cx);
                                    set_input(&editor.url, url.to_owned(), window, cx);
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                v_flex()
                    .gap(space::S8)
                    .border_t_1()
                    .border_color(r.border_muted)
                    .pt(space::S12)
                    .child(label(text::TITLE_SECTION, s::IMPORT_JSON))
                    .child(Textarea::new(&self.import))
                    .child(
                        h_flex()
                            .gap(space::S8)
                            .child(
                                Button::new("mcp-preview")
                                    .ghost()
                                    .label(s::PREVIEW)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.preview_import(cx)),
                                    ),
                            )
                            .child(
                                Button::new("mcp-import")
                                    .primary()
                                    .label(s::IMPORT)
                                    .disabled(self.importing)
                                    .on_click(cx.listener(|this, _, _, cx| this.import_valid(cx))),
                            ),
                    )
                    .when_some(self.import_preview.as_ref(), |d, preview| {
                        d.child(label(
                            text::METADATA,
                            format!(
                                "{}：{} / {}",
                                s::PREVIEW,
                                preview.valid.len(),
                                preview.invalid
                            ),
                        ))
                        .children(
                            preview
                                .valid
                                .iter()
                                .map(|input| label(text::METADATA, input.name.clone())),
                        )
                    }),
            )
            .when_some(self.error, |d, error| {
                d.child(label(text::METADATA, error).text_color(r.feedback_error))
            })
    }
}
