use std::{cell::RefCell, path::PathBuf, sync::Arc};

use tempfile::TempDir;
use thinkrail_core::{
    Commit, ItemChange, PlanItemId, PlanReviewService, PlanScope, PlanStatus, Result, ReviewRecord,
    Verdict, WorkspaceId, WorkspaceRoots,
};

use crate::{
    JsonPlanReviewStore,
    plan_ops::{self, ItemPatch, NewItem},
    window::{WindowGit, reconcile},
};

struct Fixed(PathBuf);
impl WorkspaceRoots for Fixed {
    fn workspace_root(&self, _: &WorkspaceId) -> Result<PathBuf> {
        Ok(self.0.clone())
    }
}

/// A worktree in memory: dirty paths, a commit list, and commits the host made.
#[derive(Default)]
struct FakeGit {
    dirty: RefCell<Vec<String>>,
    log: RefCell<Vec<(String, String)>>,
    commits: RefCell<Vec<(String, Vec<String>)>>,
    refuse: bool,
}

impl FakeGit {
    fn edit(&self, path: &str) {
        let mut dirty = self.dirty.borrow_mut();
        if !dirty.iter().any(|p| p == path) {
            dirty.push(path.into());
            dirty.sort();
        }
    }

    fn agent_commit(&self, sha: &str, subject: &str) {
        self.log.borrow_mut().push((sha.into(), subject.into()));
    }
}

impl WindowGit for FakeGit {
    fn uncommitted_paths(&self) -> Result<Vec<String>> {
        Ok(self.dirty.borrow().clone())
    }

    fn head(&self) -> Result<Option<String>> {
        Ok(Some(
            self.log
                .borrow()
                .last()
                .map_or("root".into(), |(sha, _)| sha.clone()),
        ))
    }

    fn log_since(&self, base: &str, limit: usize) -> Result<Vec<Commit>> {
        let log = self.log.borrow();
        let start = log
            .iter()
            .position(|(sha, _)| sha == base)
            .map_or(0, |i| i + 1);
        Ok(log[start..]
            .iter()
            .take(limit)
            .map(|(sha, subject)| Commit {
                hash: sha.clone(),
                subject: subject.clone(),
                author: "a".into(),
                timestamp: 0,
            })
            .collect())
    }

    fn commit_paths(&self, subject: &str, paths: &[String]) -> Result<Option<String>> {
        if self.refuse {
            return Ok(None);
        }
        let sha = format!("host{}", self.commits.borrow().len());
        self.commits
            .borrow_mut()
            .push((subject.into(), paths.to_vec()));
        self.dirty.borrow_mut().retain(|p| !paths.contains(p));
        self.agent_commit(&sha, subject);
        Ok(Some(sha))
    }
}

struct Chat {
    store: Arc<JsonPlanReviewStore>,
    scope: PlanScope,
}

impl Chat {
    fn add(&self, title: &str) -> PlanItemId {
        let mut id = None;
        self.store
            .update_plan(&self.scope, &mut |plan| {
                id = Some(
                    plan_ops::add(
                        plan,
                        NewItem {
                            title: title.into(),
                            ..NewItem::default()
                        },
                    )?
                    .id,
                );
                Ok(())
            })
            .unwrap();
        id.unwrap()
    }

    fn set(&self, id: &PlanItemId, status: PlanStatus, subject: Option<&str>) {
        self.store
            .update_plan(&self.scope, &mut |plan| {
                plan_ops::update(
                    plan,
                    id,
                    ItemPatch {
                        status: Some(status),
                        commit_subject: subject.map(|s| Some(s.to_owned())),
                        ..ItemPatch::default()
                    },
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn changes(&self, id: &PlanItemId) -> Vec<ItemChange> {
        let plan = self.store.plan(&self.scope).unwrap();
        plan_ops::items(&plan)
            .find(|i| &i.id == id)
            .unwrap()
            .changes
            .clone()
    }

    fn reconcile(&self, git: &FakeGit) {
        reconcile(self.store.as_ref(), &self.scope, git).unwrap();
    }
}

fn chats(names: &[&str]) -> (TempDir, Vec<Chat>) {
    let data = TempDir::new().unwrap();
    let store = Arc::new(
        JsonPlanReviewStore::open(data.path(), Arc::new(Fixed(data.path().to_path_buf()))).unwrap(),
    );
    let chats = names
        .iter()
        .map(|chat| Chat {
            store: store.clone(),
            scope: PlanScope {
                workspace: "w".into(),
                chat: (*chat).into(),
            },
        })
        .collect();
    (data, chats)
}

fn commit(sha: &str, label: &str) -> ItemChange {
    ItemChange::Commit {
        sha: sha.into(),
        label: label.into(),
    }
}

#[test]
fn a_closed_window_commits_only_its_delta_with_the_commit_subject() {
    let (_data, chats) = chats(&["c"]);
    let chat = &chats[0];
    let git = FakeGit::default();
    let step = chat.add("Add login");
    chat.set(&step, PlanStatus::InProgress, None);
    chat.reconcile(&git);
    let base = &chat.store.step_reviews(&chat.scope).unwrap().baselines[&step];
    assert_eq!((base.paths.len(), base.head.as_deref()), (0, Some("root")));

    git.edit("src/login.rs");
    git.edit(".thinkrail/context/TASK-x.md");
    chat.set(&step, PlanStatus::Done, Some("feat: login\nbody"));
    chat.reconcile(&git);
    assert_eq!(
        *git.commits.borrow(),
        [(
            "feat: login\nbody".to_owned(),
            vec!["src/login.rs".to_owned()]
        )]
    );
    assert_eq!(chat.changes(&step), [commit("host0", "Add login")]);
    assert!(
        chat.store
            .step_reviews(&chat.scope)
            .unwrap()
            .baselines
            .is_empty()
    );
    // Idempotent: a step with a change set and no window is left alone.
    git.edit("later.rs");
    chat.reconcile(&git);
    assert_eq!(git.commits.borrow().len(), 1);
    assert_eq!(chat.changes(&step), [commit("host0", "Add login")]);
}

#[test]
fn user_dirt_or_a_refused_commit_records_paths_and_drops_the_verdict() {
    let (_data, chats) = chats(&["c"]);
    let chat = &chats[0];
    let git = FakeGit::default();
    git.edit("user.txt");
    let step = chat.add("Step");
    chat.set(&step, PlanStatus::InProgress, None);
    chat.reconcile(&git);
    git.edit("a.rs");
    chat.store
        .update_reviews(&chat.scope, &mut |_, steps| {
            steps.verdicts.insert(
                step.clone(),
                ReviewRecord {
                    verdict: Verdict::Approve,
                    summary: None,
                    findings: Vec::new(),
                    reviewed_shas: Vec::new(),
                    model: None,
                    at: 0,
                },
            );
            Ok(())
        })
        .unwrap();
    chat.set(&step, PlanStatus::Done, None);
    chat.reconcile(&git);
    // user.txt is still dirty: the host does not commit, the delta is recorded as paths.
    assert!(git.commits.borrow().is_empty());
    assert_eq!(
        chat.changes(&step),
        [ItemChange::Path {
            path: "a.rs".into()
        }]
    );
    assert!(
        chat.store
            .step_reviews(&chat.scope)
            .unwrap()
            .verdicts
            .is_empty()
    );
}

#[test]
fn a_refused_host_commit_falls_back_to_paths() {
    let (_data, chats) = chats(&["c"]);
    let chat = &chats[0];
    let refusing = FakeGit {
        refuse: true,
        ..FakeGit::default()
    };
    let step = chat.add("Step");
    chat.set(&step, PlanStatus::InProgress, None);
    chat.reconcile(&refusing);
    refusing.edit("b.rs");
    chat.set(&step, PlanStatus::Done, None);
    chat.reconcile(&refusing);
    assert_eq!(
        chat.changes(&step),
        [ItemChange::Path {
            path: "b.rs".into()
        }]
    );
}

#[test]
fn an_exclusive_window_adopts_the_agents_own_commits_once() {
    let (_data, chats) = chats(&["c"]);
    let chat = &chats[0];
    let git = FakeGit::default();
    git.agent_commit("old", "before the window");
    let step = chat.add("Step");
    chat.set(&step, PlanStatus::InProgress, None);
    chat.reconcile(&git);
    git.agent_commit("a1", "agent: part one");
    git.agent_commit("a2", "");
    chat.set(&step, PlanStatus::Done, None);
    chat.reconcile(&git);
    assert_eq!(
        chat.changes(&step),
        [commit("a1", "agent: part one"), commit("a2", "Step")]
    );
    assert!(git.commits.borrow().is_empty(), "nothing left to commit");

    // A fix round: back to in progress, a new window; earlier commits are kept and not re-adopted.
    chat.set(&step, PlanStatus::InProgress, None);
    chat.reconcile(&git);
    git.edit("fix.rs");
    chat.set(&step, PlanStatus::Done, Some("fix: edge"));
    chat.reconcile(&git);
    assert_eq!(
        chat.changes(&step),
        [
            commit("a1", "agent: part one"),
            commit("a2", "Step"),
            commit("host0", "Step"),
        ]
    );
}

#[test]
fn a_shared_worktree_neither_adopts_nor_commits() {
    let (_data, chats) = chats(&["a", "b"]);
    let git = FakeGit::default();
    let first = chats[0].add("A");
    chats[0].set(&first, PlanStatus::InProgress, None);
    chats[0].reconcile(&git);
    let second = chats[1].add("B");
    chats[1].set(&second, PlanStatus::InProgress, None);
    chats[1].reconcile(&git);
    for chat in &chats {
        let steps = chat.store.step_reviews(&chat.scope).unwrap();
        assert!(
            steps.baselines.values().all(|b| b.shared),
            "both sides are shared"
        );
    }
    git.agent_commit("x1", "someone");
    git.edit("a.rs");
    chats[0].set(&first, PlanStatus::Done, None);
    chats[0].reconcile(&git);
    assert!(git.commits.borrow().is_empty());
    assert_eq!(
        chats[0].changes(&first),
        [ItemChange::Path {
            path: "a.rs".into()
        }]
    );
    chats[1].set(&second, PlanStatus::Pending, None);
    chats[1].reconcile(&git);
    assert!(
        chats[1]
            .store
            .step_reviews(&chats[1].scope)
            .unwrap()
            .baselines
            .is_empty()
    );
    // With no other window left, a new window is exclusive again.
    let third = chats[0].add("C");
    chats[0].set(&third, PlanStatus::InProgress, None);
    chats[0].reconcile(&git);
    let steps = chats[0].store.step_reviews(&chats[0].scope).unwrap();
    assert!(!steps.baselines[&third].shared);
}
