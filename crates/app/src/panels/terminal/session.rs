//! Tab and PTY link lifecycle: workspace tabs, attach/detach, close, and terminal service calls.

use gpui_kit::Context;
use thinkrail_core::{AttachRequest, Attached, TerminalEvent, TerminalRuntimeId, WorkspaceId};
use tracing::debug;

use super::{Link, SHELL, TerminalPanel, parser};
use crate::{state::run, strings::terminal as s};

impl TerminalPanel {
    pub(super) fn set_workspace(&mut self, workspace: Option<WorkspaceId>, cx: &mut Context<Self>) {
        self.workspace = workspace.clone();
        self.tabs.clear();
        self.deselect();
        cx.notify();
        let Some(workspace) = workspace else { return };
        let service = self.state.read(cx).services.terminals.clone();
        let target = workspace.clone();
        run(
            cx,
            move || service.tabs(&target),
            move |this, result, cx| {
                if this.workspace.as_ref() != Some(&workspace) {
                    return;
                }
                match result {
                    Ok(tabs) => {
                        this.tabs = tabs;
                        if let Some(first) = this.tabs.first().map(|t| t.tab_key.clone()) {
                            this.attach(first, cx);
                        }
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn detach(&mut self) {
        self.attempt += 1;
        self.link = Link::Detached;
        self.composition.clear();
        self.composition_selection = 0..0;
    }

    /// No tab is shown any more: drop the link and the previous shell's screen with it.
    fn deselect(&mut self) {
        self.selected = None;
        self.detach();
        self.parser = parser(self.size);
        self.truncated = false;
    }

    pub(super) fn create(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let key = format!("t-{}", uuid_like());
        let title = format!("{} {}", s::TITLE, self.tabs.len() + 1);
        let service = self.state.read(cx).services.terminals.clone();
        let target = workspace.clone();
        run(
            cx,
            move || service.reserve(&target, &key, &title),
            move |this, result, cx| {
                if this.workspace.as_ref() != Some(&workspace) {
                    return;
                }
                match result {
                    Ok(tab) => {
                        let key = tab.tab_key.clone();
                        this.tabs.push(tab);
                        this.attach(key, cx);
                    }
                    Err(error) => this.report(&error, cx),
                }
            },
        );
    }

    pub(super) fn attach(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        self.detach();
        self.selected = Some(key.clone());
        self.link = Link::Attaching;
        let attempt = self.attempt;
        let request = AttachRequest {
            workspace,
            tab_key: key,
            shell: SHELL,
            size: self.size,
        };
        let service = self.state.read(cx).services.terminals.clone();
        run(
            cx,
            move || service.attach(request),
            move |this, result: thinkrail_core::Result<Attached>, cx| {
                if this.attempt != attempt {
                    return;
                }
                match result {
                    Ok(attached) => {
                        this.parser = parser(attached.size);
                        this.size = attached.size;
                        this.parser.process(attached.replay.as_bytes());
                        this.truncated = attached.replay_truncated;
                        this.link = Link::Attached {
                            runtime: attached.runtime,
                            offset: attached.replay_end_offset,
                        };
                        if attached.size != this.size {
                            this.resize(cx);
                        }
                    }
                    Err(error) => {
                        this.link = Link::Detached;
                        this.report(&error, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn close(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        if self.selected.as_ref() == Some(&key) {
            self.deselect();
        }
        self.tabs.retain(|t| t.tab_key != key);
        cx.notify();
        let service = self.state.read(cx).services.terminals.clone();
        run(
            cx,
            move || service.close(&workspace, &key),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }

    pub(super) fn runtime(&self) -> Option<&TerminalRuntimeId> {
        match &self.link {
            Link::Attached { runtime, .. } => Some(runtime),
            Link::Detached | Link::Attaching | Link::Exited(_) => None,
        }
    }

    pub fn on_terminal_event(&mut self, event: TerminalEvent, cx: &mut Context<Self>) {
        match event {
            TerminalEvent::Data {
                runtime,
                byte_offset,
                data,
            } => {
                let Link::Attached {
                    runtime: current,
                    offset,
                } = &mut self.link
                else {
                    return;
                };
                if *current != runtime || byte_offset < *offset {
                    return;
                }
                if byte_offset > *offset {
                    debug!(gap = byte_offset - *offset, "terminal.output.gap");
                    if let Some(key) = self.selected.clone() {
                        self.attach(key, cx);
                    }
                    return;
                }
                self.parser.process(data.as_bytes());
                *offset = byte_offset + data.len() as u64;
                cx.notify();
            }
            TerminalEvent::Exit { runtime, exit_code } => {
                if self.runtime() == Some(&runtime) {
                    self.link = Link::Exited(exit_code);
                    self.composition.clear();
                    cx.notify();
                }
            }
        }
    }

    /// Writes go through the background executor in order (one task per chunk).
    pub(super) fn write(&mut self, data: &str, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime().cloned() else {
            return;
        };
        if data.is_empty() {
            return;
        }
        self.parser.screen_mut().set_scrollback(0);
        let service = self.state.read(cx).services.terminals.clone();
        let data = data.to_owned();
        run(
            cx,
            move || service.write(&runtime, &data),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }

    pub(super) fn resize(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime().cloned() else {
            return;
        };
        let service = self.state.read(cx).services.terminals.clone();
        let size = self.size;
        run(
            cx,
            move || service.resize(&runtime, size),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }
}

/// A short unique tab key (time + counter); tab keys are opaque to the service.
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("{millis:x}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}
