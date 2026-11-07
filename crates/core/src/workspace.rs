use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{ProjectId, Result, WorkspaceId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub slug: String,
    pub last_opened: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceKind {
    /// The project's own checkout; never removable.
    Default,
    /// A worktree created and owned by this app.
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: WorkspaceId,
    pub project_id: ProjectId,
    pub name: String,
    pub branch: String,
    pub base_branch: Option<String>,
    pub worktree_path: PathBuf,
    pub kind: WorkspaceKind,
    pub last_active_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewWorkspace {
    pub name: Option<String>,
    pub base_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Dir,
    File,
}

/// A workspace-relative entry; `path` always uses `/` separators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileContent {
    Text(String),
    /// Binary or non-UTF-8 content: only its size is exposed, never a lossy decode.
    Binary {
        byte_len: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Conflicted,
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub original_path: Option<String>,
    pub status: ChangeStatus,
    pub staged: bool,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffRequest {
    /// Limit to one workspace-relative path; `None` diffs the whole worktree.
    pub path: Option<String>,
    pub staged: bool,
}

/// The one question terminal and review anchoring need answered: where is this workspace on disk.
/// Narrow on purpose so those services cannot reach Git or the project registry.
pub trait WorkspaceRoots: Send + Sync {
    /// Canonical absolute worktree root.
    fn workspace_root(&self, id: &WorkspaceId) -> Result<PathBuf>;
}

/// Projects, worktree-backed workspaces, contained file reads and Git.
///
/// Every path argument is workspace-relative and is rejected if it escapes the worktree, crosses a
/// symlink/junction out of it, or touches `.git`. Implementations are blocking; call them from a
/// background executor, never the UI thread.
pub trait WorkspaceService: WorkspaceRoots {
    fn projects(&self) -> Result<Vec<Project>>;
    /// Whether `path` is inside a Git working tree. No side effects.
    fn is_repository(&self, path: &std::path::Path) -> bool;
    fn open_project(&self, path: &std::path::Path) -> Result<Project>;
    /// Unless `path` is already in a repository: `git init -b main` plus an initial commit of
    /// what is there (empty if nothing), so worktrees work. Then opens it as a project.
    fn init_project(&self, path: &std::path::Path) -> Result<Project>;
    /// 新建项目: makes `parent/name` (which must be missing or empty), initializes it like
    /// [`Self::init_project`] and opens it. A failure removes what was created.
    fn create_project(&self, parent: &std::path::Path, name: &str) -> Result<Project>;
    fn rename_project(&self, id: &ProjectId, name: &str) -> Result<Project>;
    fn close_project(&self, id: &ProjectId) -> Result<()>;

    fn workspaces(&self, project: &ProjectId) -> Result<Vec<Workspace>>;
    fn create_workspace(&self, project: &ProjectId, request: NewWorkspace) -> Result<Workspace>;
    fn remove_workspace(&self, id: &WorkspaceId) -> Result<()>;

    fn read_dir(&self, id: &WorkspaceId, path: &str) -> Result<Vec<DirEntry>>;
    fn read_file(&self, id: &WorkspaceId, path: &str) -> Result<FileContent>;

    fn git_status(&self, id: &WorkspaceId) -> Result<GitStatus>;
    fn git_branches(&self, id: &WorkspaceId) -> Result<Vec<Branch>>;
    fn git_commits(&self, id: &WorkspaceId, limit: usize) -> Result<Vec<Commit>>;
    /// Unified diff text, untracked files included for unstaged diffs.
    fn git_diff(&self, id: &WorkspaceId, request: &DiffRequest) -> Result<String>;
    fn git_checkout(&self, id: &WorkspaceId, branch: &str) -> Result<()>;

    /// Plan-step work windows (agent-settings 08 §5.8). Paths that differ from `HEAD` plus
    /// untracked, not ignored files; sorted.
    fn git_uncommitted_paths(&self, id: &WorkspaceId) -> Result<Vec<String>>;
    /// `None` before the first commit.
    fn git_head(&self, id: &WorkspaceId) -> Result<Option<String>>;
    /// `base..HEAD`, oldest first, at most `limit`.
    fn git_log_range(&self, id: &WorkspaceId, base: &str, limit: usize) -> Result<Vec<Commit>>;
    /// Commits exactly `paths` with the first line of `subject`, running the user's hooks. `None`
    /// when nothing was committed (nothing staged, unresolved merge, a hook refused); the index is
    /// then as it was.
    fn git_commit_paths(
        &self,
        id: &WorkspaceId,
        subject: &str,
        paths: &[String],
    ) -> Result<Option<String>>;
}
