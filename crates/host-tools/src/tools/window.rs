//! Work-window reconciliation after `todo_*` calls (agent-settings 08 §5.8): `plan_review::window`
//! over the workspace's git, one pass at a time per workspace.

use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;
use thinkrail_core::{
    Commit, ErrorCode as _, PlanReviewService, PlanScope, Result, WorkspaceId, WorkspaceService,
};
use thinkrail_plan_review::window::{self, WindowGit};
use tracing::warn;

/// Serializes reconcile passes per workspace: two chats of one worktree must not commit at once,
/// and `request_review` waits for the pass before it reads a change set.
#[derive(Default)]
pub(crate) struct WindowQueue {
    locks: Mutex<HashMap<WorkspaceId, Arc<Mutex<()>>>>,
}

impl WindowQueue {
    fn lock(&self, workspace: &WorkspaceId) -> Arc<Mutex<()>> {
        self.locks
            .lock()
            .entry(workspace.clone())
            .or_default()
            .clone()
    }

    /// Runs one pass. Git or storage failures are logged and skipped: the step then has no change
    /// set, and the next `todo_*` call tries again.
    pub(crate) fn reconcile(
        &self,
        plans: &dyn PlanReviewService,
        workspaces: &dyn WorkspaceService,
        scope: &PlanScope,
    ) {
        let lock = self.lock(&scope.workspace);
        let _guard = lock.lock();
        let git = Git {
            workspaces,
            id: &scope.workspace,
        };
        if let Err(error) = window::reconcile(plans, scope, &git) {
            warn!(workspace = %scope.workspace, chat = %scope.chat, code = error.code(), %error, "work window reconcile skipped");
        }
    }

    /// Blocks until no pass runs for `workspace`.
    pub(crate) fn settle(&self, workspace: &WorkspaceId) {
        let lock = self.lock(workspace);
        drop(lock.lock());
    }
}

struct Git<'a> {
    workspaces: &'a dyn WorkspaceService,
    id: &'a WorkspaceId,
}

impl WindowGit for Git<'_> {
    fn uncommitted_paths(&self) -> Result<Vec<String>> {
        self.workspaces.git_uncommitted_paths(self.id)
    }

    fn head(&self) -> Result<Option<String>> {
        self.workspaces.git_head(self.id)
    }

    fn log_since(&self, base: &str, limit: usize) -> Result<Vec<Commit>> {
        self.workspaces.git_log_range(self.id, base, limit)
    }

    fn commit_paths(&self, subject: &str, paths: &[String]) -> Result<Option<String>> {
        self.workspaces.git_commit_paths(self.id, subject, paths)
    }
}
