//! Project registry operations: list, init, create, open, rename and close projects.

use std::{fs, path::Path};

use thinkrail_core::{
    Error, ErrorCode, ErrorKind, IoContext, Project, ProjectId, Result, Workspace, WorkspaceId,
    WorkspaceKind,
};
use tracing::{debug, warn};

use super::{GitWorkspaceService, display_name, now, same_path, slug};
use crate::{git, registry::StoredProject};

impl GitWorkspaceService {
    pub(super) fn projects(&self) -> Result<Vec<Project>> {
        let mut projects: Vec<Project> = self
            .store
            .read()?
            .projects
            .into_iter()
            .filter(|p| !p.closed)
            .map(|p| p.project)
            .collect();
        projects.sort_by_key(|p| std::cmp::Reverse(p.last_opened));
        Ok(projects)
    }

    pub(super) fn is_repository(&self, path: &Path) -> bool {
        match git::toplevel(path) {
            Ok(_) => true,
            Err(error) if error.kind() == Some(ErrorKind::Invalid) => false,
            Err(error) => {
                warn!(code = error.code(), "workspace.repository.probe_failed");
                false
            }
        }
    }

    pub(super) fn init_project(&self, path: &Path) -> Result<Project> {
        let requested = fs::canonicalize(path).map_err(|_| Error::not_found("文件夹"))?;
        // Already inside a repository: open it rather than nest a second one.
        match git::toplevel(&requested) {
            Ok(_) => {}
            Err(error) if error.kind() == Some(ErrorKind::Invalid) => {
                git::bootstrap(&requested)?;
                debug!("workspace.repository.initialized");
            }
            Err(error) => return Err(error),
        }
        self.open_project(&requested)
    }

    pub(super) fn create_project(&self, parent: &Path, name: &str) -> Result<Project> {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0', ':']) {
            return Err(Error::invalid("项目文件夹名不合法"));
        }
        fs::create_dir_all(parent).context("创建项目位置")?;
        let path = parent.join(name);
        let created = match fs::read_dir(&path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(Error::conflict("文件夹已存在且不为空"));
                }
                false
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).context("创建项目文件夹")?;
                true
            }
            Err(error) => return Err(error).context("读取项目文件夹"),
        };
        let opened = git::bootstrap(&path).and_then(|()| self.open_project(&path));
        if opened.is_err() {
            if let Err(error) = fs::remove_dir_all(path.join(".git"))
                && error.kind() != std::io::ErrorKind::NotFound
            {
                warn!(code = "IO", io_kind = ?error.kind(), "workspace.project.repository_cleanup_failed");
            }
            if created && let Err(error) = fs::remove_dir(&path) {
                warn!(code = "IO", io_kind = ?error.kind(), "workspace.project.directory_cleanup_failed");
            }
        }
        opened
    }

    pub(super) fn open_project(&self, path: &Path) -> Result<Project> {
        let requested = fs::canonicalize(path).map_err(|_| Error::not_found("文件夹"))?;
        let top = git::toplevel(&requested)?;
        let branch = git::current_branch(&top)?;
        let base = git::default_branch(&top)?;
        let project = self.mutate(|registry| {
            if let Some(stored) = registry
                .projects
                .iter_mut()
                .find(|p| same_path(&p.project.path, &top))
            {
                stored.closed = false;
                stored.project.last_opened = now();
                return Ok(stored.project.clone());
            }
            if registry
                .workspaces
                .iter()
                .any(|w| same_path(&w.worktree_path, &top))
            {
                return Err(Error::conflict("该文件夹已是某个项目的工作区"));
            }
            let id = ProjectId::generate();
            let name = top
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Project")
                .to_owned();
            let short: String = id.as_str().chars().take(8).collect();
            let project = Project {
                id: id.clone(),
                slug: format!("{}-{short}", slug(&name)),
                name,
                path: top.clone(),
                last_opened: now(),
            };
            registry.projects.push(StoredProject {
                project: project.clone(),
                closed: false,
            });
            registry.workspaces.push(Workspace {
                id: WorkspaceId::generate(),
                project_id: id,
                name: "默认工作区".into(),
                branch: branch.clone(),
                base_branch: Some(base.clone()),
                worktree_path: top.clone(),
                kind: WorkspaceKind::Default,
                last_active_at: now(),
            });
            Ok(project)
        })?;
        debug!(project = %project.id, "workspace.project.opened");
        Ok(project)
    }

    pub(super) fn rename_project(&self, id: &ProjectId, name: &str) -> Result<Project> {
        let name = display_name(name, 100)?;
        self.mutate(|registry| {
            let stored = registry
                .projects
                .iter_mut()
                .find(|p| &p.project.id == id && !p.closed)
                .ok_or_else(|| Error::not_found("项目"))?;
            stored.project.name = name;
            Ok(stored.project.clone())
        })
    }

    pub(super) fn close_project(&self, id: &ProjectId) -> Result<()> {
        self.mutate(|registry| {
            let stored = registry
                .projects
                .iter_mut()
                .find(|p| &p.project.id == id)
                .ok_or_else(|| Error::not_found("项目"))?;
            stored.closed = true;
            Ok(())
        })
    }
}
