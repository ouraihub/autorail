//! `GitWorkspaceService`: shared state and helpers; routes `WorkspaceService` to its child modules.

mod files;
mod projects;
mod repository;
mod workspaces;

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use thinkrail_core::{
    Branch, Commit, DiffRequest, DirEntry, Error, FileContent, GitStatus, IoContext, NewWorkspace,
    Project, ProjectId, Result, Workspace, WorkspaceId, WorkspaceRoots, WorkspaceService,
};

use crate::registry::{Registry, Store};

/// [`WorkspaceService`] over the local `git` executable and `<data>/workbench.json`.
#[derive(Debug)]
pub struct GitWorkspaceService {
    data_dir: PathBuf,
    store: Store,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn display_name(raw: &str, max: usize) -> Result<String> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > max || name.chars().any(char::is_control) {
        return Err(Error::invalid(format!("名称须为 1–{max} 个可见字符")));
    }
    Ok(name)
}

fn slug(name: &str) -> String {
    let mut result = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
        } else if !result.is_empty() && !result.ends_with('-') {
            result.push('-');
        }
        if result.len() >= 60 {
            break;
        }
    }
    let trimmed = result.trim_matches('-');
    if trimmed.is_empty() {
        "workspace".into()
    } else {
        trimmed.into()
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    fs::canonicalize(a).is_ok_and(|a| a == b)
}

impl GitWorkspaceService {
    /// Opens (or starts) the registry in `data_dir`; damaged metadata fails with `Corrupt`.
    pub fn open(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir).context("创建数据目录")?;
        let data_dir = fs::canonicalize(data_dir).context("解析数据目录")?;
        let store = Store::new(&data_dir);
        store.read()?;
        Ok(Self { data_dir, store })
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut Registry) -> Result<T>) -> Result<T> {
        let lock = self.store.lock()?;
        let mut registry = self.store.read()?;
        let value = change(&mut registry)?;
        self.store.write(&lock, &registry)?;
        Ok(value)
    }

    fn root_of(registry: &Registry, id: &WorkspaceId) -> Result<PathBuf> {
        let workspace = registry.workspace(id)?;
        let path = fs::canonicalize(&workspace.worktree_path)
            .map_err(|_| Error::not_found("工作区目录"))?;
        if !path.is_dir() {
            return Err(Error::not_found("工作区目录"));
        }
        Ok(path)
    }

    fn root(&self, id: &WorkspaceId) -> Result<PathBuf> {
        Self::root_of(&self.store.read()?, id)
    }
}

impl WorkspaceRoots for GitWorkspaceService {
    fn workspace_root(&self, id: &WorkspaceId) -> Result<PathBuf> {
        self.root(id)
    }
}

// Each method runs the same-named inherent method in the child module that owns it.
impl WorkspaceService for GitWorkspaceService {
    fn projects(&self) -> Result<Vec<Project>> {
        Self::projects(self)
    }

    fn is_repository(&self, path: &Path) -> bool {
        Self::is_repository(self, path)
    }

    fn init_project(&self, path: &Path) -> Result<Project> {
        Self::init_project(self, path)
    }

    fn create_project(&self, parent: &Path, name: &str) -> Result<Project> {
        Self::create_project(self, parent, name)
    }

    fn open_project(&self, path: &Path) -> Result<Project> {
        Self::open_project(self, path)
    }

    fn rename_project(&self, id: &ProjectId, name: &str) -> Result<Project> {
        Self::rename_project(self, id, name)
    }

    fn close_project(&self, id: &ProjectId) -> Result<()> {
        Self::close_project(self, id)
    }

    fn workspaces(&self, project: &ProjectId) -> Result<Vec<Workspace>> {
        Self::workspaces(self, project)
    }

    fn create_workspace(&self, project: &ProjectId, request: NewWorkspace) -> Result<Workspace> {
        Self::create_workspace(self, project, request)
    }

    fn remove_workspace(&self, id: &WorkspaceId) -> Result<()> {
        Self::remove_workspace(self, id)
    }

    fn read_dir(&self, id: &WorkspaceId, path: &str) -> Result<Vec<DirEntry>> {
        Self::read_dir(self, id, path)
    }

    fn read_file(&self, id: &WorkspaceId, path: &str) -> Result<FileContent> {
        Self::read_file(self, id, path)
    }

    fn git_status(&self, id: &WorkspaceId) -> Result<GitStatus> {
        Self::git_status(self, id)
    }

    fn git_branches(&self, id: &WorkspaceId) -> Result<Vec<Branch>> {
        Self::git_branches(self, id)
    }

    fn git_commits(&self, id: &WorkspaceId, limit: usize) -> Result<Vec<Commit>> {
        Self::git_commits(self, id, limit)
    }

    fn git_diff(&self, id: &WorkspaceId, request: &DiffRequest) -> Result<String> {
        Self::git_diff(self, id, request)
    }

    fn git_checkout(&self, id: &WorkspaceId, branch: &str) -> Result<()> {
        Self::git_checkout(self, id, branch)
    }
}
