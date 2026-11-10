//! Workspace listing plus creation and guarded removal of app-managed Git worktrees.

use std::{fs, path::Path};

use thinkrail_core::{
    Error, IoContext, NewWorkspace, ProjectId, Result, Workspace, WorkspaceId, WorkspaceKind,
};
use tracing::debug;

use super::{GitWorkspaceService, display_name, now, same_path, slug};
use crate::{git, paths::reject_links, registry::Ownership};

impl GitWorkspaceService {
    pub(super) fn workspaces(&self, project: &ProjectId) -> Result<Vec<Workspace>> {
        let registry = self.store.read()?;
        registry.project(project)?;
        let mut workspaces: Vec<Workspace> = registry
            .workspaces
            .iter()
            .filter(|w| &w.project_id == project)
            .cloned()
            .collect();
        // Branch is read live: users switch branches outside the app.
        for workspace in &mut workspaces {
            if let Ok(root) = Self::root_of(&registry, &workspace.id) {
                workspace.branch = git::current_branch(&root)?;
            }
        }
        Ok(workspaces)
    }

    pub(super) fn create_workspace(
        &self,
        project: &ProjectId,
        request: NewWorkspace,
    ) -> Result<Workspace> {
        let registry = self.store.read()?;
        let project = registry.project(project)?.clone();
        let project_path =
            fs::canonicalize(&project.path).map_err(|_| Error::not_found("项目目录"))?;
        let name = request
            .name
            .as_deref()
            .map(|s| display_name(s, 60))
            .transpose()?;
        let base = match request.base_ref {
            Some(base) => {
                git::safe_ref(&base)?;
                base
            }
            None => git::current_branch(&project_path)?,
        };
        let commit = git::ok(
            &project_path,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{base}^{{commit}}"),
            ],
        )?;
        let parent = self.data_dir.join("worktrees").join(&project.slug);
        fs::create_dir_all(&parent).context("创建托管工作区目录")?;
        reject_links(&self.data_dir, &parent)?;
        let stem = name
            .as_deref()
            .map(slug)
            .unwrap_or_else(|| "workspace".into());
        // Named: `stem`, `stem-2`, ...; unnamed: `workspace-1`, `workspace-2`, ...
        let candidates = (if name.is_some() { 1 } else { 2 }..=1001).map(|n| {
            if n == 1 {
                stem.clone()
            } else {
                format!("{stem}-{}", if name.is_some() { n } else { n - 1 })
            }
        });
        let mut branch = None;
        for candidate in candidates {
            git::validate_branch(&project_path, &candidate)?;
            if !parent.join(&candidate).exists() && !git::branch_exists(&project_path, &candidate)?
            {
                branch = Some(candidate);
                break;
            }
        }
        let branch = branch.ok_or_else(|| Error::conflict("无法为新工作区分配分支名"))?;
        let path = parent.join(&branch);
        let path_arg = git::path_arg(&path)?;
        git::ok(
            &project_path,
            &[
                "worktree",
                "add",
                "--no-track",
                "-b",
                &branch,
                "--",
                &path_arg,
                commit.trim(),
            ],
        )?;
        let common = git::common_dir(&path)?;
        let workspace = Workspace {
            id: WorkspaceId::generate(),
            project_id: project.id.clone(),
            name: name.unwrap_or_else(|| branch.clone()),
            branch: branch.clone(),
            base_branch: Some(base),
            worktree_path: path.clone(),
            kind: WorkspaceKind::Managed,
            last_active_at: now(),
        };
        let saved = self.mutate(|registry| {
            registry.project(&project.id)?;
            registry.managed.insert(
                workspace.id.clone(),
                Ownership {
                    path: path.clone(),
                    project_id: project.id.clone(),
                    branch: branch.clone(),
                    common_dir: common.clone(),
                },
            );
            registry.workspaces.push(workspace.clone());
            Ok(workspace.clone())
        });
        match saved {
            Ok(workspace) => {
                debug!(workspace = %workspace.id, "workspace.managed.created");
                Ok(workspace)
            }
            Err(error) => {
                // Keep the branch (it may hold commits); remove only the empty worktree we just made.
                if let Err(cleanup) =
                    git::ok(&project_path, &["worktree", "remove", "--", &path_arg])
                {
                    tracing::warn!(workspace = %workspace.id, code = %thinkrail_core::ErrorCode::code(&cleanup), "workspace.managed.cleanup_failed");
                }
                Err(error)
            }
        }
    }

    pub(super) fn remove_workspace(&self, id: &WorkspaceId) -> Result<()> {
        let registry = self.store.read()?;
        let workspace = registry.workspace(id)?.clone();
        if workspace.kind != WorkspaceKind::Managed {
            return Err(Error::forbidden("默认工作区不能删除"));
        }
        let owner = registry
            .managed
            .get(id)
            .ok_or_else(|| Error::forbidden("工作区不是本应用创建的"))?
            .clone();
        let project = registry.project(&workspace.project_id)?.clone();
        let project_path =
            fs::canonicalize(&project.path).map_err(|_| Error::not_found("项目目录"))?;
        git::validate_branch(&project_path, &owner.branch)?;
        if owner.branch.contains('/') {
            return Err(Error::forbidden("托管工作区分支名无效"));
        }
        let expected = self
            .data_dir
            .join("worktrees")
            .join(&project.slug)
            .join(&owner.branch);
        let path = Self::root_of(&registry, id)?;
        if path != expected
            || owner.path != expected
            || owner.project_id != workspace.project_id
            || workspace.worktree_path != expected
        {
            return Err(Error::forbidden("托管工作区的归属记录与实际路径不一致"));
        }
        reject_links(&self.data_dir, &expected)?;
        let common = git::common_dir(&path)?;
        if common != git::common_dir(&project_path)? || common != owner.common_dir {
            return Err(Error::forbidden("托管工作区不属于该项目仓库"));
        }
        let listing = git::ok(&project_path, &["worktree", "list", "--porcelain", "-z"])?;
        let registered = listing
            .split('\0')
            .filter_map(|line| line.strip_prefix("worktree "))
            .any(|p| same_path(Path::new(p), &path));
        if !registered || path == project_path {
            return Err(Error::forbidden("不是已注册的托管 worktree"));
        }
        // Git refuses dirty or locked trees; never force, never recursively delete.
        git::ok(
            &project_path,
            &["worktree", "remove", "--", &git::path_arg(&path)?],
        )?;
        self.mutate(|registry| {
            registry.workspaces.retain(|w| &w.id != id);
            registry.managed.remove(id);
            Ok(())
        })?;
        debug!(workspace = %id, "workspace.managed.removed");
        Ok(())
    }
}
