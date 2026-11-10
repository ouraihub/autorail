//! Plan-step work windows (agent-settings 08 §5.8, the original `todos/artifacts.ts`): after each
//! `todo_*` call the host opens a window for the step in progress and, when a step is done,
//! attributes what changed in its window to it, committing the delta itself when it safely can.

use std::collections::BTreeSet;

use thinkrail_core::{
    Baseline, Commit, ItemChange, PlanItem, PlanItemId, PlanReviewService, PlanScope, PlanStatus,
    Result,
};
use tracing::debug;

use crate::plan_ops;

/// The app's own folder in a worktree (skills write scratch files there); never a step's change.
const APP_DIR: &str = ".thinkrail";
/// The most commits one window adopts, as in the original.
const WINDOW_COMMITS: usize = 200;

/// The git facts and the one action reconciliation needs, for one workspace.
pub trait WindowGit {
    fn uncommitted_paths(&self) -> Result<Vec<String>>;
    fn head(&self) -> Result<Option<String>>;
    /// `base..HEAD`, oldest first, at most `limit`.
    fn log_since(&self, base: &str, limit: usize) -> Result<Vec<Commit>>;
    /// The new sha, `None` when nothing was committed.
    fn commit_paths(&self, subject: &str, paths: &[String]) -> Result<Option<String>>;
}

fn is_app_state(path: &str) -> bool {
    path == APP_DIR
        || path
            .strip_prefix(APP_DIR)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn changed_paths(git: &dyn WindowGit) -> Result<Vec<String>> {
    let mut paths = git.uncommitted_paths()?;
    paths.retain(|path| !is_app_state(path));
    Ok(paths)
}

fn commit_shas(item: &PlanItem) -> impl Iterator<Item = &str> {
    item.changes.iter().filter_map(|change| match change {
        ItemChange::Commit { sha, .. } => Some(sha.as_str()),
        ItemChange::Path { .. } => None,
    })
}

/// One reconcile pass for a chat. A git failure stops it with nothing written for the step it
/// was on; the next `todo_*` call tries again.
pub fn reconcile(
    store: &dyn PlanReviewService,
    scope: &PlanScope,
    git: &dyn WindowGit,
) -> Result<()> {
    // Windows of steps that are gone or back to pending are dropped.
    let (plan, steps) = store.update_reviews(scope, &mut |plan, steps| {
        let keep: BTreeSet<&PlanItemId> = plan_ops::items(plan)
            .filter(|item| item.status != PlanStatus::Pending)
            .map(|item| &item.id)
            .collect();
        steps.baselines.retain(|id, _| keep.contains(id));
        Ok(())
    })?;
    let items: Vec<&PlanItem> = plan_ops::items(&plan).collect();
    if let Some(active) = items
        .iter()
        .find(|item| item.status == PlanStatus::InProgress)
        && !steps.baselines.contains_key(&active.id)
    {
        let baseline = Baseline {
            paths: changed_paths(git)?,
            head: git.head()?,
            shared: false,
        };
        store.open_window(scope, &active.id, baseline)?;
    }
    let open: Vec<PlanItemId> = steps.baselines.keys().cloned().collect();
    let mut owned: BTreeSet<String> = items
        .iter()
        .flat_map(|item| commit_shas(item).map(str::to_owned))
        .collect();
    for item in items.iter().filter(|item| item.status == PlanStatus::Done) {
        let Some(base) = steps.baselines.get(&item.id) else {
            continue;
        };
        close(store, scope, git, item, base, &open, &mut owned)?;
    }
    Ok(())
}

fn close(
    store: &dyn PlanReviewService,
    scope: &PlanScope,
    git: &dyn WindowGit,
    item: &PlanItem,
    base: &Baseline,
    open: &[PlanItemId],
    owned: &mut BTreeSet<String>,
) -> Result<()> {
    let now = changed_paths(git)?;
    let delta: Vec<String> = now
        .iter()
        .filter(|path| !base.paths.contains(path))
        .cloned()
        .collect();
    let exclusive = !base.shared && !store.windows_elsewhere(scope)?;
    let sole = open.iter().all(|id| id == &item.id);
    let mut adopted = Vec::new();
    if let Some(head) = base.head.as_deref()
        && exclusive
        && sole
    {
        for commit in git.log_since(head, WINDOW_COMMITS)? {
            if owned.insert(commit.hash.clone()) {
                let label = if commit.subject.is_empty() {
                    item.title.clone()
                } else {
                    commit.subject
                };
                adopted.push(ItemChange::Commit {
                    sha: commit.hash,
                    label,
                });
            }
        }
    }
    let committed =
        if !delta.is_empty() && exclusive && base.paths.iter().all(|path| !now.contains(path)) {
            let subject = item
                .commit_subject
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&item.title);
            git.commit_paths(subject, &delta)?
        } else {
            None
        };
    if let Some(sha) = &committed {
        owned.insert(sha.clone());
    }
    let (adopted_count, delta_count) = (adopted.len(), delta.len());
    let mut found = false;
    store.update_reviews(scope, &mut |plan, steps| {
        // The step left `Done` or its window changed meanwhile: leave both to the next pass.
        if steps.baselines.get(&item.id) != Some(base) {
            return Ok(());
        }
        let Some(stored) = plan_ops::items_mut(plan)
            .find(|stored| stored.id == item.id && stored.status == PlanStatus::Done)
        else {
            return Ok(());
        };
        found = true;
        steps.baselines.remove(&item.id);
        if adopted.is_empty() && delta.is_empty() {
            return Ok(());
        }
        stored
            .changes
            .retain(|change| matches!(change, ItemChange::Commit { .. }));
        stored.changes.extend(adopted.iter().cloned());
        match &committed {
            Some(sha) => stored.changes.push(ItemChange::Commit {
                sha: sha.clone(),
                label: stored.title.clone(),
            }),
            None if !delta.is_empty() => {
                stored.changes.extend(
                    delta
                        .iter()
                        .map(|path| ItemChange::Path { path: path.clone() }),
                );
                // A path list has no watermark: the step is reviewed afresh. The fix round stays.
                steps.verdicts.remove(&item.id);
            }
            None => {}
        }
        Ok(())
    })?;
    if found {
        debug!(
            workspace = %scope.workspace,
            chat = %scope.chat,
            item = %item.id,
            adopted = adopted_count,
            delta = delta_count,
            committed = committed.is_some(),
            "review.window.closed"
        );
    }
    Ok(())
}
