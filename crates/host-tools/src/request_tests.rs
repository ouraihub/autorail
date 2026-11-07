use std::{path::PathBuf, sync::Arc};

use parking_lot::Mutex;
use serde_json::json;
use tempfile::TempDir;
use thinkrail_core::{
    BackendKind, EngineProvider, FindingState, HostToolHandler, HostToolOptions, ItemChange,
    ModelPrefService, ModelPrefs, ModelRef, PlanItemId, PlanReviewService, PlanScope, PlanStatus,
    Result, ReviewFinding, ReviewHandle, ReviewOrigin, ReviewOutcome, ReviewRecord, ReviewResult,
    ReviewRunner, ReviewSwitch, StartedReview, ThinkingLevel, Verdict, WorkspaceId, WorkspaceRoots,
};
use thinkrail_plan_review::{
    JsonPlanReviewStore,
    plan_ops::{self, ItemPatch, NewItem},
};

use crate::HostTools;

struct Root(PathBuf);
impl WorkspaceRoots for Root {
    fn workspace_root(&self, _: &WorkspaceId) -> Result<PathBuf> {
        Ok(self.0.clone())
    }
}

/// Only the 评审 switch matters to `request_review`.
struct Prefs(Mutex<ModelPrefs>);
impl ModelPrefService for Prefs {
    fn prefs(&self) -> Result<ModelPrefs> {
        Ok(self.0.lock().clone())
    }
    fn set_review_switch(&self, _: ReviewSwitch, on: bool) -> Result<ModelPrefs> {
        self.0.lock().review.agent_review = on;
        self.prefs()
    }
    fn set_default(&self, _: Option<ModelRef>, _: Option<ThinkingLevel>) -> Result<ModelPrefs> {
        unreachable!()
    }
    fn set_hidden(&self, _: &ModelRef, _: bool) -> Result<ModelPrefs> {
        unreachable!()
    }
    fn set_engine_provider(&self, _: BackendKind, _: EngineProvider) -> Result<ModelPrefs> {
        unreachable!()
    }
    fn set_default_agent(&self, _: Option<BackendKind>) -> Result<ModelPrefs> {
        unreachable!()
    }
    fn set_context_overrides(&self, _: &[(ModelRef, Option<u32>)]) -> Result<ModelPrefs> {
        unreachable!()
    }
    fn set_review_model(
        &self,
        _: Option<ModelRef>,
        _: Option<ThinkingLevel>,
    ) -> Result<ModelPrefs> {
        unreachable!()
    }
}

/// Finishes a waiting review at once with `outcome`; a second start of the same step joins.
#[derive(Default)]
struct Runner {
    starts: Mutex<Vec<(PlanItemId, ReviewOrigin)>>,
    running: Mutex<Option<(PlanItemId, ReviewHandle)>>,
    outcome: Mutex<Option<ReviewOutcome>>,
    cancels: Mutex<usize>,
}

impl ReviewRunner for Runner {
    fn start(
        &self,
        _: &PlanScope,
        item: &PlanItemId,
        origin: ReviewOrigin,
    ) -> Result<StartedReview> {
        self.starts.lock().push((item.clone(), origin));
        if let Some((running, handle)) = self.running.lock().as_ref()
            && running == item
        {
            return Ok(StartedReview {
                handle: handle.clone(),
                joined: true,
            });
        }
        let handle = ReviewHandle::default();
        match self.outcome.lock().clone() {
            Some(outcome) => handle.finish(ReviewResult {
                text: format!("{outcome:?}"),
                outcome,
                record: None,
            }),
            None => *self.running.lock() = Some((item.clone(), handle.clone())),
        }
        Ok(StartedReview {
            handle,
            joined: false,
        })
    }

    fn cancel(&self, _: &PlanScope) {
        *self.cancels.lock() += 1;
    }
}

struct Fixture {
    _dirs: (TempDir, TempDir),
    store: Arc<JsonPlanReviewStore>,
    prefs: Arc<Prefs>,
    runner: Arc<Runner>,
    tools: HostTools,
    scope: PlanScope,
}

const SYNC: HostToolOptions = HostToolOptions {
    request_review: true,
    async_review: false,
};
const ASYNC: HostToolOptions = HostToolOptions {
    request_review: true,
    async_review: true,
};

fn fixture() -> Fixture {
    let (data, work) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let roots: Arc<dyn WorkspaceRoots> = Arc::new(Root(work.path().to_path_buf()));
    let store = Arc::new(JsonPlanReviewStore::open(data.path(), roots.clone()).unwrap());
    let mut prefs = ModelPrefs::default();
    prefs.review.agent_review = true;
    let prefs = Arc::new(Prefs(Mutex::new(prefs)));
    let runner = Arc::new(Runner::default());
    let tools = HostTools::new(store.clone(), roots).with_reviews(runner.clone(), prefs.clone());
    Fixture {
        _dirs: (data, work),
        store,
        prefs,
        runner,
        tools,
        scope: PlanScope {
            workspace: "w".into(),
            chat: "c".into(),
        },
    }
}

impl Fixture {
    /// A step with `status` and, when `committed`, one commit in its change set.
    fn step(&self, status: PlanStatus, committed: bool) -> PlanItemId {
        let mut id = None;
        self.store
            .update_plan(&self.scope, &mut |plan| {
                let item = plan_ops::add(
                    plan,
                    NewItem {
                        title: "Step".into(),
                        ..NewItem::default()
                    },
                )?
                .id;
                plan_ops::update(
                    plan,
                    &item,
                    ItemPatch {
                        status: Some(status),
                        ..ItemPatch::default()
                    },
                )?;
                if committed {
                    let stored = plan.loose.iter_mut().find(|i| i.id == item).unwrap();
                    stored.changes.push(ItemChange::Commit {
                        sha: "abc".into(),
                        label: "Step".into(),
                    });
                }
                id = Some(item);
                Ok(())
            })
            .unwrap();
        id.unwrap()
    }

    fn review(&self, options: HostToolOptions, id: &PlanItemId) -> (bool, String) {
        let result = self
            .tools
            .call(
                &self.scope,
                options,
                "request_review",
                json!({"itemId": id.as_str()}),
            )
            .unwrap();
        (result.is_error, result.text)
    }
}

fn names(tools: &HostTools, options: HostToolOptions) -> Vec<String> {
    tools.specs(options).into_iter().map(|s| s.name).collect()
}

#[test]
fn the_tool_is_listed_only_when_offered_with_the_right_wording() {
    let f = fixture();
    assert!(!names(&f.tools, HostToolOptions::default()).contains(&"request_review".into()));
    let review = |options| {
        f.tools
            .specs(options)
            .into_iter()
            .find(|s| s.name == "request_review")
            .unwrap()
    };
    assert!(review(SYNC).description.contains("next action"));
    assert!(review(ASYNC).description.contains("【评审结论】"));
    assert_eq!(review(SYNC).input_schema["required"], json!(["itemId"]));
    // Without a runner the switch cannot list it.
    let bare = HostTools::new(f.store.clone(), Arc::new(Root(PathBuf::new())));
    assert!(!names(&bare, SYNC).contains(&"request_review".into()));
}

#[test]
fn request_review_checks_the_switch_and_the_step_before_starting() {
    let f = fixture();
    let pending = f.step(PlanStatus::Pending, true);
    let empty = f.step(PlanStatus::Done, false);
    let ready = f.step(PlanStatus::Done, true);
    let refused = |options, id: &PlanItemId, needle: &str| {
        let (error, text) = f.review(options, id);
        assert!(error && text.contains(needle), "{text}");
    };
    refused(HostToolOptions::default(), &ready, "not available");
    refused(SYNC, &"nope".into(), "No plan step");
    refused(SYNC, &pending, "is not done");
    refused(SYNC, &empty, "no reviewable changes");
    // Turned off since the engine started: refused until it restarts.
    f.prefs
        .set_review_switch(ReviewSwitch::AgentReview, false)
        .unwrap();
    refused(SYNC, &ready, "turned off");
    assert!(f.runner.starts.lock().is_empty());
}

#[test]
fn a_waiting_call_returns_the_verdict_and_a_failure_is_a_tool_error() {
    let f = fixture();
    let ready = f.step(PlanStatus::Done, true);
    *f.runner.outcome.lock() = Some(ReviewOutcome::ChangesFix);
    assert_eq!(f.review(SYNC, &ready), (false, "ChangesFix".into()));
    assert_eq!(
        f.runner.starts.lock()[0],
        (ready.clone(), ReviewOrigin::Tool { waits: true })
    );
    *f.runner.outcome.lock() = Some(ReviewOutcome::Failed {
        message: "timeout".into(),
    });
    assert!(f.review(SYNC, &ready).0);
}

#[test]
fn a_message_call_returns_at_once_and_joins_a_running_review() {
    let f = fixture();
    let ready = f.step(PlanStatus::Done, true);
    let (error, first) = f.review(ASYNC, &ready);
    assert!(!error && first.starts_with("The review has started."));
    let (_, second) = f.review(ASYNC, &ready);
    assert!(second.starts_with("A review of this step is already running."));
    assert_eq!(
        f.runner.starts.lock()[0].1,
        ReviewOrigin::Tool { waits: false }
    );
    f.tools.cancel(&f.scope);
    assert_eq!(*f.runner.cancels.lock(), 1);
}

#[test]
fn resolve_comment_resolves_open_reviewer_findings() {
    let f = fixture();
    let ready = f.step(PlanStatus::Done, true);
    f.store
        .update_reviews(&f.scope, &mut |_, steps| {
            steps.verdicts.insert(
                ready.clone(),
                ReviewRecord {
                    verdict: Verdict::RequestChanges,
                    summary: None,
                    findings: ["rf_a", "rf_b"]
                        .map(|id| ReviewFinding {
                            id: id.into(),
                            path: None,
                            start_line: None,
                            end_line: None,
                            body: "BUG".into(),
                            state: FindingState::Open,
                        })
                        .to_vec(),
                    reviewed_shas: vec!["abc".into()],
                    model: None,
                    at: 0,
                },
            );
            Ok(())
        })
        .unwrap();
    let resolve = |id: &str| {
        f.tools
            .call(&f.scope, SYNC, "resolve_comment", json!({"commentId": id}))
            .unwrap()
    };
    assert!(!resolve("rf_a").is_error);
    assert!(resolve("rf_a").is_error, "already resolved");
    assert!(resolve("rf_zzz").is_error);
    assert!(!resolve("rf_b").is_error);
    // The last open finding resolved: the step is unreviewed again.
    assert!(f.store.step_reviews(&f.scope).unwrap().verdicts.is_empty());
}
