//! File and diff viewer (04 §5, §9.2): read-only, opened from the Changes panel. Large files are
//! virtualised (one uniform row per drawn line); long lines wrap at the line width (08 A19).

mod diff;
mod rows;
mod wrap;

use std::rc::Rc;

use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement,
    ListHorizontalSizingBehavior, ParentElement as _, Pixels, Render, Styled as _, Subscription,
    Window,
    assets::IconName,
    base::{h_flex, v_flex},
    canvas, div,
    prelude::FluentBuilder as _,
    px, uniform_list,
};
use thinkrail_core::{DiffRequest, LineWidth, WorkspaceId};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, Tab, TabStrip, label},
    metrics::{size, space, text},
    typography::code_cell_width,
};

use rows::{Piece, Row, cut, diff_rows, file_rows, gutter_width, render_piece};

use crate::{
    appearance::AppearanceState,
    state::{AppEvent, AppState, run},
    strings::files as s,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum DocKind {
    File,
    Diff { staged: bool },
}

enum Body {
    Loading,
    Rows(Rc<Vec<Row>>),
}

struct Doc {
    path: String,
    kind: DocKind,
    body: Body,
    /// The pieces for the wrap column they were cut at.
    pieces: Option<(usize, Rc<Vec<Piece>>)>,
}

pub struct FilePanel {
    state: Entity<AppState>,
    workspace: Option<WorkspaceId>,
    docs: Vec<Doc>,
    active: usize,
    /// The document area's width at the last paint; zero until then.
    body_width: Pixels,
    _subscriptions: Vec<Subscription>,
}

impl FilePanel {
    pub fn new(state: Entity<AppState>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&state, |this, state, event: &AppEvent, cx| {
                if *event == AppEvent::Workspace {
                    this.workspace = state.read(cx).workspace.clone();
                    this.docs.clear();
                    this.active = 0;
                    cx.notify();
                }
            }),
            // Line width, fit-to-pane and the code font all move the wrap column.
            cx.observe_global::<AppearanceState>(|_, cx| cx.notify()),
        ];
        let workspace = state.read(cx).workspace.clone();
        Self {
            state,
            workspace,
            docs: Vec::new(),
            active: 0,
            body_width: px(0.),
            _subscriptions: subscriptions,
        }
    }

    fn set_body_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        if (width - self.body_width).abs() >= px(0.5) {
            self.body_width = width;
            cx.notify();
        }
    }

    pub fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.open(path, DocKind::File, cx);
    }

    pub fn open_diff(&mut self, path: String, staged: bool, cx: &mut Context<Self>) {
        self.open(path, DocKind::Diff { staged }, cx);
    }

    /// Focuses an open document or opens it; always reloads, the worktree may have changed.
    fn open(&mut self, path: String, kind: DocKind, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let index = match self
            .docs
            .iter()
            .position(|d| d.path == path && d.kind == kind)
        {
            Some(index) => index,
            None => {
                self.docs.push(Doc {
                    path: path.clone(),
                    kind: kind.clone(),
                    body: Body::Loading,
                    pieces: None,
                });
                self.docs.len() - 1
            }
        };
        self.active = index;
        self.docs[index].body = Body::Loading;
        self.docs[index].pieces = None;
        cx.notify();
        let service = self.state.read(cx).services.workspace.clone();
        let request_path = path.clone();
        let request_kind = kind.clone();
        let for_workspace = workspace.clone();
        run(
            cx,
            move || match request_kind {
                DocKind::File => Ok(file_rows(service.read_file(&for_workspace, &request_path)?)),
                DocKind::Diff { staged } => {
                    let text = service.git_diff(
                        &for_workspace,
                        &DiffRequest {
                            path: Some(request_path),
                            staged,
                        },
                    )?;
                    Ok(diff_rows(&text))
                }
            },
            move |this, result, cx| {
                if this.workspace.as_ref() != Some(&workspace) {
                    return;
                }
                let Some(doc) = this
                    .docs
                    .iter_mut()
                    .find(|d| d.path == path && d.kind == kind)
                else {
                    return;
                };
                match result {
                    Ok(rows) => doc.body = Body::Rows(Rc::new(rows)),
                    Err(error) => {
                        doc.body = Body::Rows(Rc::new(Vec::new()));
                        this.state.update(cx, |state, cx| state.report(&error, cx));
                    }
                }
                cx.notify();
            },
        );
    }

    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.docs.len() {
            self.docs.remove(index);
            self.active = self.active.min(self.docs.len().saturating_sub(1));
            cx.notify();
        }
    }
}

impl Render for FilePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let (line_width, bounded) =
            cx.try_global::<AppearanceState>()
                .map_or((LineWidth::DEFAULT, true), |state| {
                    (
                        state.prefs.file_line_width,
                        state.prefs.file_line_width_bounded,
                    )
                });
        let cell = code_cell_width(&text::CODE_DOCUMENT, cx);
        let body_width = self.body_width;
        // Before the first paint the pane width is unknown: wrap at the setting, not at 1.
        let fits = |gutter: Pixels| {
            if body_width > px(0.) {
                ((body_width - gutter).max(px(0.)) / cell).floor() as usize
            } else {
                usize::MAX
            }
        };
        if let Some(doc) = self.docs.get_mut(self.active)
            && let Body::Rows(rows) = &doc.body
        {
            let gutter = gutter_width(matches!(doc.kind, DocKind::Diff { .. }));
            let column = wrap::column(line_width, bounded, fits(gutter));
            if doc.pieces.as_ref().is_none_or(|(at, _)| *at != column) {
                doc.pieces = Some((column, Rc::new(cut(rows, column))));
            }
        }
        let mut strip = TabStrip::new();
        for (index, doc) in self.docs.iter().enumerate() {
            let name = doc.path.rsplit('/').next().unwrap_or(&doc.path).to_owned();
            let icon = if doc.kind == DocKind::File {
                IconName::FileText
            } else {
                IconName::FileDiff
            };
            strip = strip.tab(
                Tab::new(format!("doc-{index}"), name)
                    .icon(icon)
                    .active(index == self.active)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = index;
                        cx.notify();
                    }))
                    .on_close(cx.listener(move |this, _, _, cx| this.close(index, cx))),
            );
        }
        let body: AnyElement = match self.docs.get(self.active) {
            None => div()
                .p(space::S16)
                .child(label(text::METADATA, s::EMPTY).text_color(r.text_subtle))
                .into_any_element(),
            Some(Doc {
                body: Body::Loading,
                ..
            }) => div()
                .p(space::S16)
                .child(label(text::METADATA, s::LOADING).text_color(r.text_subtle))
                .into_any_element(),
            Some(Doc {
                body: Body::Rows(rows),
                path,
                kind,
                pieces,
            }) => {
                let rows = rows.clone();
                let (column, pieces) = pieces.clone().unwrap_or_default();
                let count = pieces.len();
                // A column wider than the pane scrolls sideways (fit-to-pane off, 08 A19).
                let gutter = gutter_width(matches!(kind, DocKind::Diff { .. }));
                let row_width = gutter + cell * column as f32;
                let scrolls = body_width > px(0.) && row_width > body_width;
                let (path, kind) = (path.clone(), kind.clone());
                let open_full = matches!(kind, DocKind::Diff { .. });
                let path_row = h_flex()
                    .h(size::PANEL_HEADER)
                    .flex_none()
                    .px(space::S8)
                    .gap(space::S8)
                    .border_b_1()
                    .border_color(r.border_muted)
                    .child(
                        label(text::CODE_TEXT, path.clone())
                            .flex_1()
                            .truncate()
                            .text_color(r.text_muted),
                    )
                    .when(matches!(kind, DocKind::Diff { staged: true }), |d| {
                        d.child(label(text::METADATA, s::STAGED).text_color(r.text_subtle))
                    })
                    .when(open_full, |d| {
                        d.child(
                            IconButton::new("doc-open-full", IconName::FileText)
                                .tooltip(s::OPEN_FILE)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_file(path.clone(), cx)
                                })),
                        )
                    });
                v_flex()
                    .size_full()
                    .child(path_row)
                    .child(
                        uniform_list("doc-rows", count, move |range, _, cx| {
                            let r = ThinkrailTheme::global(cx).roles;
                            pieces[range]
                                .iter()
                                .map(|piece| {
                                    render_piece(&rows, piece, scrolls.then_some(row_width), &r)
                                })
                                .collect::<Vec<_>>()
                        })
                        .when(scrolls, |list| {
                            list.with_horizontal_sizing_behavior(
                                ListHorizontalSizingBehavior::Unconstrained,
                            )
                        })
                        .flex_1()
                        .min_h(px(0.)),
                    )
                    .into_any_element()
            }
        };
        v_flex()
            .id("file-panel")
            .size_full()
            .bg(r.container_workspace_bg)
            .child(strip)
            .child(div().relative().flex_1().min_h(px(0.)).child(body).child({
                let entity = cx.entity();
                canvas(
                    move |bounds, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.set_body_width(bounds.size.width, cx);
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            }))
    }
}
