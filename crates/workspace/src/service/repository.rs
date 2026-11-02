//! Git queries and actions inside one workspace: status, branches, commits, diff, checkout.

use std::{fs, path::Path};

use thinkrail_core::{
    Branch, Commit, DiffRequest, Error, GitStatus, IoContext, Result, WorkspaceId,
};
use tracing::info;

use super::GitWorkspaceService;
use crate::{
    git::{self, MAX_OUTPUT},
    paths::{resolve_contained, safe_relative, utf8, validate_missing_leaf},
};

impl GitWorkspaceService {
    pub(super) fn git_status(&self, id: &WorkspaceId) -> Result<GitStatus> {
        git::status(&self.root(id)?)
    }

    pub(super) fn git_branches(&self, id: &WorkspaceId) -> Result<Vec<Branch>> {
        git::branches(&self.root(id)?)
    }

    pub(super) fn git_commits(&self, id: &WorkspaceId, limit: usize) -> Result<Vec<Commit>> {
        git::commits(&self.root(id)?, limit)
    }

    pub(super) fn git_diff(&self, id: &WorkspaceId, request: &DiffRequest) -> Result<String> {
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

    pub(super) fn git_checkout(&self, id: &WorkspaceId, branch: &str) -> Result<()> {
        let root = self.root(id)?;
        git::validate_branch(&root, branch)?;
        if !git::branch_exists(&root, branch)? {
            return Err(Error::not_found(format!("分支 {branch}")));
        }
        git::ok(&root, &["switch", "--no-guess", "--", branch])?;
        info!(workspace = %id, "branch switched");
        Ok(())
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
