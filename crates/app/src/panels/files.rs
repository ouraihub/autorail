//! File and diff viewer (04 §5, §9.2): read-only, opened from the Changes panel. Large files are
//! virtualised (one uniform row per line).

mod diff;

use std::rc::Rc;

use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Rgba, Styled as _,
    Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    div, prelude::FluentBuilder as _, px, uniform_list,
};
use thinkrail_core::{DiffRequest, FileContent, WorkspaceId};
use thinkrail_ui::{
    Roles, ThinkrailTheme,
    components::{IconButton, Tab, TabStrip, TextStyled as _, label},
    metrics::{size, space, text},
};

use crate::{
    state::{AppEvent, AppState, run},
    strings::files as s,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum DocKind {
    File,
    Diff { staged: bool },
}

/// One rendered row of a document.
#[derive(Debug, Clone)]
enum Row {
    Code { number: u32, text: String },
    Diff(diff::Line),
    Hunk(String),
    FileHeader(String),
    Note(String),
}

enum Body {
    Loading,
    Rows(Rc<Vec<Row>>),
}

struct Doc {
    path: String,
    kind: DocKind,
    body: Body,
}

pub struct FilePanel {
    state: Entity<AppState>,
    workspace: Option<WorkspaceId>,
    docs: Vec<Doc>,
    active: usize,
    _subscriptions: Vec<Subscription>,
}

impl FilePanel {
    pub fn new(state: Entity<AppState>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.subscribe(&state, |this, state, event: &AppEvent, cx| {
            if *event == AppEvent::Workspace {
                this.workspace = state.read(cx).workspace.clone();
                this.docs.clear();
                this.active = 0;
                cx.notify();
            }
        })];
        let workspace = state.read(cx).workspace.clone();
        Self { state, workspace, docs: Vec::new(), active: 0, _subscriptions: subscriptions }
    }

    pub fn open_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.open(path, DocKind::File, cx);
    }

    pub fn open_diff(&mut self, path: String, staged: bool, cx: &mut Context<Self>) {
        self.open(path, DocKind::Diff { staged }, cx);
    }

    /// Focuses an open document or opens it; always reloads, the worktree may have changed.
    fn open(&mut self, path: String, kind: DocKind, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else { return };
        let index = match self.docs.iter().position(|d| d.path == path && d.kind == kind) {
            Some(index) => index,
            None => {
                self.docs.push(Doc { path: path.clone(), kind: kind.clone(), body: Body::Loading });
                self.docs.len() - 1
            }
        };
        self.active = index;
        self.docs[index].body = Body::Loading;
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
                    let text = service.git_diff(&for_workspace, &DiffRequest { path: Some(request_path), staged })?;
                    Ok(diff_rows(&text))
                }
            },
            move |this, result, cx| {
                if this.workspace.as_ref() != Some(&workspace) {
                    return;
                }
                let Some(doc) = this.docs.iter_mut().find(|d| d.path == path && d.kind == kind) else { return };
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

fn file_rows(content: FileContent) -> Vec<Row> {
    match content {
        FileContent::Text(text) => {
            text.lines().enumerate().map(|(i, line)| Row::Code { number: i as u32 + 1, text: line.to_owned() }).collect()
        }
        FileContent::Binary { byte_len } => vec![Row::Note(format!("{}（{byte_len} {}）", s::BINARY, s::BYTES))],
    }
}

fn diff_rows(text: &str) -> Vec<Row> {
    let files = diff::parse(text);
    if files.is_empty() || files.iter().all(|f| f.hunks.is_empty() && !f.binary && f.old_path.is_none()) {
        return vec![Row::Note(s::NO_DIFF.to_owned())];
    }
    let several = files.len() > 1;
    let mut rows = Vec::new();
    for file in files {
        if several {
            rows.push(Row::FileHeader(file.path.clone()));
        }
        if let Some(old) = file.old_path.as_ref().filter(|old| **old != file.path) {
            rows.push(Row::Note(format!("{} {old}", s::RENAMED_FROM)));
        }
        if file.binary {
            rows.push(Row::Note(s::BINARY_DIFF.to_owned()));
        }
        for hunk in file.hunks {
            rows.push(Row::Hunk(hunk.header));
            rows.extend(hunk.lines.into_iter().map(Row::Diff));
        }
    }
    rows
}

fn number(n: Option<u32>) -> String {
    n.map(|n| n.to_string()).unwrap_or_default()
}

fn render_row(row: &Row, r: &Roles) -> AnyElement {
    let gutter = |old: String, new: String| {
        h_flex()
            .flex_none()
            .gap(space::S4)
            .pr(space::S8)
            .text_color(r.text_subtle)
            .child(div().w(px(40.)).flex().justify_end().child(old))
            .child(div().w(px(40.)).flex().justify_end().child(new))
    };
    let line = |content: String, color: Rgba| div().flex_1().min_w(px(0.)).overflow_hidden().whitespace_nowrap().text_color(color).child(content);
    let base = h_flex().w_full().h(size::ROW).px(space::S8).text_style(text::CODE_DOCUMENT);
    match row {
        Row::Code { number: n, text: t } => base
            .child(div().flex_none().w(px(56.)).pr(space::S12).flex().justify_end().text_color(r.text_subtle).child(n.to_string()))
            .child(line(t.clone(), r.text_default))
            .into_any_element(),
        Row::Diff(l) => {
            let (sign, bg, fg) = match l.kind {
                diff::Kind::Added => ("+", Some(r.feedback_success_subtle), r.feedback_success),
                diff::Kind::Removed => ("−", Some(r.feedback_error_subtle), r.feedback_error),
                diff::Kind::Context => (" ", None, r.text_muted),
                diff::Kind::Note => (" ", None, r.text_subtle),
            };
            base.when_some(bg, |d, bg| d.bg(bg))
                .child(gutter(number(l.old), number(l.new)))
                .child(div().flex_none().w(space::S16).text_color(fg).child(sign))
                .child(line(l.text.clone(), if l.kind == diff::Kind::Context { r.text_default } else { fg }))
                .into_any_element()
        }
        Row::Hunk(header) => base.bg(r.container_content_bg).child(line(header.clone(), r.text_subtle)).into_any_element(),
        Row::FileHeader(path) => base
            .bg(r.container_header_bg)
            .child(div().text_style(text::TITLE_COMPACT).text_color(r.text_default).child(path.clone()))
            .into_any_element(),
        Row::Note(note) => base.child(line(note.clone(), r.text_subtle)).into_any_element(),
    }
}

impl Render for FilePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let mut strip = TabStrip::new();
        for (index, doc) in self.docs.iter().enumerate() {
            let name = doc.path.rsplit('/').next().unwrap_or(&doc.path).to_owned();
            let icon = if doc.kind == DocKind::File { IconName::FileText } else { IconName::FileDiff };
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
            None => div().p(space::S16).child(label(text::METADATA, s::EMPTY).text_color(r.text_subtle)).into_any_element(),
            Some(Doc { body: Body::Loading, .. }) => {
                div().p(space::S16).child(label(text::METADATA, s::LOADING).text_color(r.text_subtle)).into_any_element()
            }
            Some(Doc { body: Body::Rows(rows), path, kind }) => {
                let rows = rows.clone();
                let count = rows.len();
                let (path, kind) = (path.clone(), kind.clone());
                let open_full = matches!(kind, DocKind::Diff { .. });
                let path_row = h_flex()
                    .h(size::PANEL_HEADER)
                    .flex_none()
                    .px(space::S8)
                    .gap(space::S8)
                    .border_b_1()
                    .border_color(r.border_muted)
                    .child(label(text::CODE_TEXT, path.clone()).flex_1().truncate().text_color(r.text_muted))
                    .when(matches!(kind, DocKind::Diff { staged: true }), |d| {
                        d.child(label(text::METADATA, s::STAGED).text_color(r.text_subtle))
                    })
                    .when(open_full, |d| {
                        d.child(IconButton::new("doc-open-full", IconName::FileText).tooltip(s::OPEN_FILE).on_click(
                            cx.listener(move |this, _, _, cx| this.open_file(path.clone(), cx)),
                        ))
                    });
                v_flex()
                    .size_full()
                    .child(path_row)
                    .child(
                        uniform_list("doc-rows", count, move |range, _, cx| {
                            let r = ThinkrailTheme::global(cx).roles;
                            rows[range].iter().map(|row| render_row(row, &r)).collect::<Vec<_>>()
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
            .child(div().flex_1().min_h(px(0.)).child(body))
    }
}
