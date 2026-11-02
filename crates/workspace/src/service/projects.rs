//! Project registry operations: list, init, create, open, rename and close projects.

use std::{fs, path::Path};

use thinkrail_core::{Error, Project, ProjectId, Result, Workspace, WorkspaceId, WorkspaceKind};
use tracing::info;

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
        git::toplevel(path).is_ok()
    }

    pub(super) fn init_project(&self, path: &Path) -> Result<Project> {
        let requested = fs::canonicalize(path).map_err(|_| Error::not_found("文件夹"))?;
        // Already inside a repository: open it rather than nest a second one.
        if git::toplevel(&requested).is_err() {
            git::bootstrap(&requested)?;
            info!(path = %requested.display(), "git repository initialized");
        }
        self.open_project(&requested)
    }

    pub(super) fn create_project(&self, parent: &Path, name: &str) -> Result<Project> {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0', ':']) {
            return Err(Error::invalid(format!("项目文件夹名不合法：{name}")));
        }
        fs::create_dir_all(parent)
            .map_err(|e| Error::invalid(format!("无法创建位置 {}：{e}", parent.display())))?;
        let path = parent.join(name);
        let created = match fs::read_dir(&path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(Error::conflict(format!(
                        "文件夹已存在且不为空：{}",
                        path.display()
                    )));
                }
                false
            }
            Err(_) => {
                fs::create_dir(&path)
                    .map_err(|e| Error::invalid(format!("无法创建 {}：{e}", path.display())))?;
                true
            }
        };
        let opened = git::bootstrap(&path).and_then(|()| self.open_project(&path));
        if opened.is_err() {
            let _ = fs::remove_dir_all(path.join(".git"));
            if created {
                let _ = fs::remove_dir(&path);
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
        info!(project = %project.id, "project opened");
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
