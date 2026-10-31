//! Changes panel (04 §9.2): the worktree's Git status as a list or a directory tree. Clicking a
//! file opens its diff in the file panel.

use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{Change, ChangeStatus, GitStatus, WorkspaceId};
use thinkrail_ui::{
    Roles, ThinkrailTheme,
    components::{IconButton, PanelHeader, Segment, TreeRow, label},
    metrics::{size, space, text},
};

use super::files::FilePanel;
use crate::{
    state::{AppEvent, AppState, run},
    strings::changes as s,
};

/// One row of the tree view: a directory header or a change at `depth`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TreeItem {
    Dir { path: String, name: String, depth: usize },
    File { index: usize, name: String, depth: usize },
}

/// Groups changes by directory, directories before their files, both sorted by path.
pub(crate) fn tree(changes: &[Change]) -> Vec<TreeItem> {
    let mut order: Vec<usize> = (0..changes.len()).collect();
    order.sort_by(|a, b| changes[*a].path.cmp(&changes[*b].path));
    let mut items = Vec::new();
    let mut open: Vec<String> = Vec::new();
    for index in order {
        let parts: Vec<&str> = changes[index].path.split('/').collect();
        let (dirs, name) = parts.split_at(parts.len().saturating_sub(1));
        let common = open.iter().zip(dirs).take_while(|(a, b)| a == *b).count();
        open.truncate(common);
        for (depth, dir) in dirs.iter().enumerate().skip(common) {
            open.push((*dir).to_owned());
            items.push(TreeItem::Dir { path: dirs[..=depth].join("/"), name: (*dir).to_owned(), depth });
        }
        items.push(TreeItem::File { index, name: name.first().copied().unwrap_or_default().to_owned(), depth: dirs.len() });
    }
    items
}

fn status_mark(status: ChangeStatus, r: &Roles) -> (&'static str, gpui_kit::Rgba) {
    match status {
        ChangeStatus::Added => ("A", r.feedback_success),
        ChangeStatus::Untracked => ("U", r.text_subtle),
        ChangeStatus::Deleted => ("D", r.feedback_error),
        ChangeStatus::Modified | ChangeStatus::TypeChanged => ("M", r.feedback_warning),
        ChangeStatus::Renamed => ("R", r.feedback_info),
        ChangeStatus::Copied => ("C", r.feedback_info),
        ChangeStatus::Conflicted => ("!", r.feedback_error),
    }
}

pub struct ChangesPanel {
    state: Entity<AppState>,
    files: Entity<FilePanel>,
    workspace: Option<WorkspaceId>,
    status: Option<GitStatus>,
    tree_mode: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChangesPanel {
    pub fn new(state: Entity<AppState>, files: Entity<FilePanel>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.subscribe(&state, |this, state, event: &AppEvent, cx| {
            let workspace = state.read(cx).workspace.clone();
            if *event == AppEvent::WorkspaceChanged || workspace != this.workspace {
                this.workspace = workspace;
                this.status = None;
                this.refresh(cx);
            }
        })];
        let workspace = state.read(cx).workspace.clone();
        let mut panel = Self { state, files, workspace, status: None, tree_mode: false, _subscriptions: subscriptions };
        panel.refresh(cx);
        panel
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            cx.notify();
            return;
        };
        let service = self.state.read(cx).services.workspace.clone();
        let target = workspace.clone();
        run(cx, move || service.git_status(&target), move |this, result, cx| {
            if this.workspace.as_ref() != Some(&workspace) {
                return;
            }
            match result {
                Ok(status) => this.status = Some(status),
                Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
            }
            cx.notify();
        });
    }

    fn change_row(&self, index: usize, change: &Change, name: String, depth: usize, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let (mark, color) = status_mark(change.status, &r);
        let counts = match (change.additions, change.deletions) {
            (None, None) => None,
            (add, del) => Some((add.unwrap_or(0), del.unwrap_or(0))),
        };
        let (path, staged) = (change.path.clone(), change.staged);
        TreeRow::new(format!("change-{index}"), name)
            .depth(depth)
            .trailing(h_flex().gap(space::S4).when_some(counts, |d, (add, del)| {
                d.child(label(text::METADATA, format!("+{add}")).text_color(r.feedback_success))
                    .child(label(text::METADATA, format!("−{del}")).text_color(r.feedback_error))
            }))
            .trailing(label(text::METADATA, mark).w(size::HEADER_CONTROL).text_color(color))
            .on_click(cx.listener(move |this, _, _, cx| {
                let path = path.clone();
                this.files.update(cx, |files, cx| files.open_diff(path, staged, cx));
            }))
            .into_any_element()
    }
}

impl Render for ChangesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let header = PanelHeader::new(s::TITLE)
            .action(Segment::new("changes-mode", [s::LIST, s::TREE], usize::from(self.tree_mode)).on_select(cx.listener(
                |this, index: &usize, _, cx| {
                    this.tree_mode = *index == 1;
                    cx.notify();
                },
            )))
            .action(IconButton::new("changes-refresh", IconName::RefreshCw).tooltip(s::REFRESH).on_click(cx.listener(
                |this, _, _, cx| this.refresh(cx),
            )));
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut branch_line = None;
        if let Some(status) = self.status.clone() {
            let upstream = match &status.upstream {
                Some(upstream) => format!("{upstream} · {} {} · {} {}", s::AHEAD, status.ahead, s::BEHIND, status.behind),
                None => s::NO_UPSTREAM.to_owned(),
            };
            branch_line = Some(format!("{} · {upstream}", status.branch.clone().unwrap_or_default()));
            if self.tree_mode {
                for item in tree(&status.changes) {
                    match item {
                        TreeItem::Dir { path, name, depth } => rows.push(
                            TreeRow::new(format!("change-dir-{path}"), name)
                                .depth(depth)
                                .icon(IconName::Folder)
                                .into_any_element(),
                        ),
                        TreeItem::File { index, name, depth } => {
                            rows.push(self.change_row(index, &status.changes[index], name, depth, cx));
                        }
                    }
                }
            } else {
                for (index, change) in status.changes.iter().enumerate() {
                    rows.push(self.change_row(index, change, change.path.clone(), 0, cx));
                }
            }
        }
        let empty = match (&self.workspace, &self.status) {
            (None, _) => Some(s::NO_WORKSPACE),
            (Some(_), Some(status)) if status.changes.is_empty() => Some(s::CLEAN),
            _ => None,
        };
        v_flex()
            .size_full()
            .bg(r.container_sidebar_bg)
            .border_t_1()
            .border_color(r.border_muted)
            .child(header)
            .when_some(branch_line, |d, line| {
                d.child(label(text::METADATA, line).px(size::PANEL_PADDING).py(space::S4).truncate().text_color(r.text_subtle))
            })
            .child(
                v_flex()
                    .id("changes-list")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p(space::S4)
                    .gap(space::S2)
                    .children(rows)
                    .when_some(empty, |d, message| {
                        d.child(label(text::METADATA, message).p(size::PANEL_PADDING).text_color(r.text_subtle))
                    }),
            )
            .child(div())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str) -> Change {
        Change { path: path.into(), original_path: None, status: ChangeStatus::Modified, staged: false, additions: None, deletions: None }
    }

    #[test]
    fn tree_groups_files_under_shared_directories() {
        let changes = vec![change("src/b.rs"), change("README.md"), change("src/a/x.rs"), change("src/a.rs")];
        let items = tree(&changes);
        let names: Vec<String> = items
            .iter()
            .map(|i| match i {
                TreeItem::Dir { name, depth, .. } => format!("{depth}d:{name}"),
                TreeItem::File { name, depth, .. } => format!("{depth}f:{name}"),
            })
            .collect();
        assert_eq!(names, ["0f:README.md", "0d:src", "1f:a.rs", "1d:a", "2f:x.rs", "1f:b.rs"]);
    }
}
