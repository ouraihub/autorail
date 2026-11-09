//! `AppState`: the window's shared projection of projects, workspaces and the current selection,
//! plus the error notice. Panels read it and subscribe to [`AppEvent`]; only its methods write it.

use gpui_kit::{Context, EventEmitter};
use thinkrail_core::{Error, ErrorCode, Project, ProjectId, Result, Workspace, WorkspaceId};
use tracing::{debug, warn};

use crate::services::Services;

/// Runs blocking service work on the background executor and hands the result back to the view.
/// Each call is one `ui.background` root: ENTRY when issued, one terminal after the view applied
/// the result (EXIT, or ERROR with the stable code). The job and the completion run inside the
/// root span, so their lines carry the same TraceId; `at` keeps the caller location.
#[track_caller]
pub fn run<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    job: impl FnOnce() -> Result<T> + Send + 'static,
    done: impl FnOnce(&mut V, Result<T>, &mut Context<V>) + 'static,
) {
    let caller = std::panic::Location::caller();
    let (_trace, root) = thinkrail_observability::TraceContext::root("ui.background", "ui");
    let span = root.span().clone();
    let job_span = span.clone();
    let task = cx
        .background_executor()
        .spawn(async move { job_span.in_scope(job) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        let _entered = span.enter();
        let code = result.as_ref().err().map(ErrorCode::code);
        if this.update(cx, |view, cx| done(view, result, cx)).is_err() {
            debug!(at = %caller, "view closed before background job finished");
        }
        match code {
            None => root.finish("SUCCESS"),
            Some(code) => root.fail(code),
        }
    })
    .detach();
}

/// A user-facing failure: the stable code plus the human message (04 §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub code: &'static str,
    pub message: String,
}

impl Notice {
    pub fn from_error(error: &Error) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    Projects,
    /// The selected workspace changed (or was cleared).
    Workspace,
    /// Provider, MCP or model preferences changed; active chats re-check signatures.
    Routing,
}

pub struct AppState {
    pub services: Services,
    pub projects: Vec<Project>,
    pub project: Option<ProjectId>,
    /// Workspaces of every open project, in registry order.
    pub workspaces: Vec<Workspace>,
    pub workspace: Option<WorkspaceId>,
    pub notice: Option<Notice>,
    /// 新建项目 with "让 Agent 起草规格": this project's first chat starts the setup skill.
    pub setup_project: Option<ProjectId>,
}

impl EventEmitter<AppEvent> for AppState {}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("project", &self.project)
            .field("workspace", &self.workspace)
            .finish_non_exhaustive()
    }
}

impl AppState {
    pub fn new(services: Services, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            services,
            projects: Vec::new(),
            project: None,
            workspaces: Vec::new(),
            workspace: None,
            notice: None,
            setup_project: None,
        };
        state.reload(cx);
        state
    }

    pub fn current_workspace(&self) -> Option<&Workspace> {
        let id = self.workspace.as_ref()?;
        self.workspaces.iter().find(|w| &w.id == id)
    }

    pub fn current_project(&self) -> Option<&Project> {
        let id = self.project.as_ref()?;
        self.projects.iter().find(|p| &p.id == id)
    }

    pub fn workspaces_of<'a>(
        &'a self,
        project: &'a ProjectId,
    ) -> impl Iterator<Item = &'a Workspace> + 'a {
        self.workspaces
            .iter()
            .filter(move |w| &w.project_id == project)
    }

    /// Shows a failure in the notice bar and logs its code (never its message text).
    pub fn report(&mut self, error: &Error, cx: &mut Context<Self>) {
        warn!(code = error.code(), "operation failed");
        self.notice = Some(Notice::from_error(error));
        cx.notify();
    }

    pub fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }

    /// Re-reads projects and their workspaces; keeps the selection when it still exists.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let workspace = self.services.workspace.clone();
        run(
            cx,
            move || {
                let projects = workspace.projects()?;
                let mut all = Vec::new();
                for project in &projects {
                    all.extend(workspace.workspaces(&project.id)?);
                }
                Ok((projects, all))
            },
            |this, result, cx| match result {
                Ok((projects, workspaces)) => {
                    this.projects = projects;
                    this.workspaces = workspaces;
                    let project_gone = this
                        .project
                        .as_ref()
                        .is_some_and(|id| !this.projects.iter().any(|p| &p.id == id));
                    if project_gone || this.project.is_none() {
                        this.project = this.projects.first().map(|p| p.id.clone());
                    }
                    let workspace_gone = this
                        .workspace
                        .as_ref()
                        .is_some_and(|id| !this.workspaces.iter().any(|w| &w.id == id));
                    if workspace_gone || this.workspace.is_none() {
                        let first = this
                            .project
                            .as_ref()
                            .and_then(|p| this.workspaces_of(p).next())
                            .map(|w| w.id.clone());
                        this.set_workspace(first, cx);
                    }
                    cx.emit(AppEvent::Projects);
                    cx.notify();
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    pub fn select_workspace(&mut self, id: WorkspaceId, cx: &mut Context<Self>) {
        if let Some(project) = self
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.project_id.clone())
        {
            self.project = Some(project);
        }
        self.set_workspace(Some(id), cx);
        cx.notify();
    }

    fn set_workspace(&mut self, id: Option<WorkspaceId>, cx: &mut Context<Self>) {
        if self.workspace != id {
            self.workspace = id;
            cx.emit(AppEvent::Workspace);
        }
    }

    pub fn open_project(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let workspace = self.services.workspace.clone();
        run(
            cx,
            move || workspace.open_project(&path),
            |this, result, cx| match result {
                Ok(project) => this.opened(project, cx),
                Err(error) => this.report(&error, cx),
            },
        );
    }

    /// Selects a freshly opened (or initialized) project and reloads the tree.
    pub fn opened(&mut self, project: Project, cx: &mut Context<Self>) {
        self.project = Some(project.id);
        self.workspace = None;
        self.reload(cx);
    }

    pub fn close_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        let workspace = self.services.workspace.clone();
        run(
            cx,
            move || workspace.close_project(&id),
            |this, result, cx| match result {
                Ok(()) => this.reload(cx),
                Err(error) => this.report(&error, cx),
            },
        );
    }

    pub fn create_workspace(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let workspace = self.services.workspace.clone();
        run(
            cx,
            move || workspace.create_workspace(&project, thinkrail_core::NewWorkspace::default()),
            |this, result, cx| match result {
                Ok(created) => {
                    this.workspace = None;
                    this.workspaces.push(created.clone());
                    this.select_workspace(created.id, cx);
                    this.reload(cx);
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    pub fn remove_workspace(&mut self, id: WorkspaceId, cx: &mut Context<Self>) {
        let workspace = self.services.workspace.clone();
        run(
            cx,
            move || workspace.remove_workspace(&id),
            |this, result, cx| match result {
                Ok(()) => this.reload(cx),
                Err(error) => this.report(&error, cx),
            },
        );
    }
}
