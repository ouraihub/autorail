use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use thinkrail_core::{
    Branch, Commit, DiffRequest, DirEntry, EntryKind, Error, FileContent, GitStatus, IoContext,
    NewWorkspace, Project, ProjectId, Result, Workspace, WorkspaceId, WorkspaceKind,
    WorkspaceRoots, WorkspaceService,
};
use tracing::info;

use crate::{
    git::{self, MAX_OUTPUT},
    paths::{
        bounded_read, forbidden_component, is_link, reject_links, resolve_contained, safe_relative,
        utf8, validate_missing_leaf,
    },
    registry::{Ownership, Registry, Store, StoredProject},
};

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

    fn untracked_diff(&self, root: &Path, only: Option<&str>, diff: &mut String) -> Result<()> {
        for change in git::status(root)?.changes {
            if change.status != thinkrail_core::ChangeStatus::Untracked
                || only.is_some_and(|p| p != change.path)
            {
                continue;
            }
            let relative = safe_relative(&change.path)?;
            validate_missing_leaf(root, &relative)?;
            let lexical = root.join(&relative);
            // A symlink's content in Git is its target text; never read through it.
            if fs::symlink_metadata(&lexical)
                .context("读取未跟踪文件")?
                .file_type()
                .is_symlink()
            {
                let target = fs::read_link(&lexical).context("读取符号链接")?;
                let target = utf8(&target)?;
                let path = &change.path;
                diff.push_str(&format!(
                    "diff --git a/{path} b/{path}\nnew file mode 120000\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+{target}\n\\ No newline at end of file\n"
                ));
            } else if resolve_contained(root, &relative)?.is_file() {
                let args = [
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--",
                    git::null_device(),
                    &relative,
                ];
                diff.push_str(&git::run(root, &args, &[1])?.stdout);
            }
            if diff.len() > MAX_OUTPUT {
                return Err(Error::invalid("diff 超过 16 MiB 上限"));
            }
        }
        Ok(())
    }
}

impl WorkspaceRoots for GitWorkspaceService {
    fn workspace_root(&self, id: &WorkspaceId) -> Result<PathBuf> {
        self.root(id)
    }
}

impl WorkspaceService for GitWorkspaceService {
    fn projects(&self) -> Result<Vec<Project>> {
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

    fn is_repository(&self, path: &Path) -> bool {
        git::toplevel(path).is_ok()
    }

    fn init_project(&self, path: &Path) -> Result<Project> {
        let requested = fs::canonicalize(path).map_err(|_| Error::not_found("文件夹"))?;
        // Already inside a repository: open it rather than nest a second one.
        if git::toplevel(&requested).is_err() {
            git::bootstrap(&requested)?;
            info!(path = %requested.display(), "git repository initialized");
        }
        self.open_project(&requested)
    }

    fn create_project(&self, parent: &Path, name: &str) -> Result<Project> {
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

    fn open_project(&self, path: &Path) -> Result<Project> {
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

    fn rename_project(&self, id: &ProjectId, name: &str) -> Result<Project> {
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

    fn close_project(&self, id: &ProjectId) -> Result<()> {
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

    fn workspaces(&self, project: &ProjectId) -> Result<Vec<Workspace>> {
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

    fn create_workspace(&self, project: &ProjectId, request: NewWorkspace) -> Result<Workspace> {
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
                info!(workspace = %workspace.id, "managed workspace created");
                Ok(workspace)
            }
            Err(error) => {
                // Keep the branch (it may hold commits); remove only the empty worktree we just made.
                if let Err(cleanup) =
                    git::ok(&project_path, &["worktree", "remove", "--", &path_arg])
                {
                    tracing::warn!(code = %thinkrail_core::ErrorCode::code(&cleanup), "orphan worktree left after failed save");
                }
                Err(error)
            }
        }
    }

    fn remove_workspace(&self, id: &WorkspaceId) -> Result<()> {
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
        info!(workspace = %id, "managed workspace removed");
        Ok(())
    }

    fn read_dir(&self, id: &WorkspaceId, path: &str) -> Result<Vec<DirEntry>> {
        let root = self.root(id)?;
        let directory = resolve_contained(&root, path)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(directory).context("读取目录")? {
            let entry = entry.context("读取目录")?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if forbidden_component(&name) {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&root)
                .map_err(|_| Error::forbidden("路径超出工作区"))?
                .to_path_buf();
            let relative = utf8(&relative)?.replace('\\', "/");
            // Escaping links are not navigable entries.
            let Ok(resolved) = resolve_contained(&root, &relative) else {
                continue;
            };
            let metadata = fs::metadata(&resolved).context("读取目录项")?;
            let kind = if metadata.is_dir() {
                EntryKind::Dir
            } else if metadata.is_file() {
                EntryKind::File
            } else {
                continue;
            };
            entries.push(DirEntry {
                name,
                path: relative,
                kind,
            });
        }
        entries.sort_by(|a, b| {
            (a.kind != EntryKind::Dir)
                .cmp(&(b.kind != EntryKind::Dir))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    }

    fn read_file(&self, id: &WorkspaceId, path: &str) -> Result<FileContent> {
        let root = self.root(id)?;
        let file = resolve_contained(&root, path)?;
        let metadata = fs::symlink_metadata(&file).context("读取文件")?;
        if !metadata.is_file() || is_link(&metadata) {
            return Err(Error::invalid("不是普通文件"));
        }
        let bytes = bounded_read(File::open(&file).context("读取文件")?, MAX_OUTPUT)?;
        if bytes.contains(&0) {
            return Ok(FileContent::Binary {
                byte_len: bytes.len() as u64,
            });
        }
        Ok(match String::from_utf8(bytes) {
            Ok(text) => FileContent::Text(text),
            Err(error) => FileContent::Binary {
                byte_len: error.as_bytes().len() as u64,
            },
        })
    }

    fn git_status(&self, id: &WorkspaceId) -> Result<GitStatus> {
        git::status(&self.root(id)?)
    }

    fn git_branches(&self, id: &WorkspaceId) -> Result<Vec<Branch>> {
        git::branches(&self.root(id)?)
    }

    fn git_commits(&self, id: &WorkspaceId, limit: usize) -> Result<Vec<Commit>> {
        git::commits(&self.root(id)?, limit)
    }

    fn git_diff(&self, id: &WorkspaceId, request: &DiffRequest) -> Result<String> {
        let root = self.root(id)?;
        let only = match request.path.as_deref() {
            Some(input) => {
                let relative = safe_relative(input)?;
                if relative.is_empty() {
                    return Err(Error::invalid("diff 路径不能为空"));
                }
                validate_missing_leaf(&root, &relative)?;
                Some(relative)
            }
            None => None,
        };
        let mut args = vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--binary",
        ];
        if request.staged {
            args.push("--cached");
        }
        args.push("--");
        if let Some(path) = &only {
            args.push(path);
        }
        let mut diff = git::ok(&root, &args)?;
        if !request.staged {
            self.untracked_diff(&root, only.as_deref(), &mut diff)?;
        }
        Ok(diff)
    }

    fn git_checkout(&self, id: &WorkspaceId, branch: &str) -> Result<()> {
        let root = self.root(id)?;
        git::validate_branch(&root, branch)?;
        if !git::branch_exists(&root, branch)? {
            return Err(Error::not_found(format!("分支 {branch}")));
        }
        git::ok(&root, &["switch", "--no-guess", "--", branch])?;
        info!(workspace = %id, "branch switched");
        Ok(())
    }
}
