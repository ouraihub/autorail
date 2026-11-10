//! App-owned reviewer session runner.

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    thread,
    time::{Duration, Instant},
};

use thinkrail_core::{
    AgentRuntime, BackendKind, ChatBindingService, ChatId, ContentBlock, EngineSetup,
    ModelPrefService, ModelRouter, PlanItemId, PlanReviewService, PlanScope, PlanStatus,
    ReviewEvent, ReviewHandle, ReviewOrigin, ReviewOutcome, ReviewResult, ReviewRunner, Role,
    SessionEvent, SessionPhase, SessionRole, SessionSink, StartSession, WorkspaceService,
};
use thinkrail_plan_review::{plan_ops, review, verdict};

use super::Services;

mod dispatch;

const TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
struct SinkState {
    snapshot: Option<thinkrail_core::SessionSnapshot>,
}

struct ReviewerSink {
    state: Mutex<SinkState>,
    wake: Condvar,
    span: tracing::Span,
}

impl SessionSink for ReviewerSink {
    fn publish(&self, event: SessionEvent) {
        let _entered = self.span.enter();
        tracing::debug!("review.session_event");
        if let SessionEvent::Snapshot { snapshot } = event {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.snapshot = Some(*snapshot);
            self.wake.notify_all();
        }
    }
}

struct Running {
    handle: ReviewHandle,
    span: tracing::Span,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

pub struct AppReviewRunner {
    plans: Arc<dyn PlanReviewService>,
    bindings: Arc<dyn ChatBindingService>,
    workspace: Arc<dyn WorkspaceService>,
    prefs: Arc<dyn ModelPrefService>,
    router: Arc<dyn ModelRouter>,
    engines: Arc<dyn EngineSetup>,
    runtimes: [Arc<dyn AgentRuntime>; 3],
    support_dir: PathBuf,
    events: async_channel::Sender<(tracing::Span, ReviewEvent)>,
    self_ref: OnceLock<Weak<AppReviewRunner>>,
    running: Mutex<HashMap<(String, String, PlanItemId), Running>>,
}

impl AppReviewRunner {
    pub fn new(
        services: &Services,
        support_dir: PathBuf,
        events: async_channel::Sender<(tracing::Span, ReviewEvent)>,
    ) -> Arc<Self> {
        let runner = Arc::new(Self {
            plans: services.plans.clone(),
            bindings: services.bindings.clone(),
            workspace: services.workspace.clone(),
            prefs: services.prefs.clone(),
            router: services.router.clone(),
            engines: services.engines.clone(),
            runtimes: [
                services.runtime(BackendKind::Pi),
                services.runtime(BackendKind::Omp),
                services.runtime(BackendKind::Dsh),
            ],
            support_dir,
            events,
            self_ref: OnceLock::new(),
            running: Mutex::new(HashMap::new()),
        });
        let _ = runner.self_ref.set(Arc::downgrade(&runner));
        runner
    }

    fn runtime(&self, backend: BackendKind) -> Arc<dyn AgentRuntime> {
        match backend {
            BackendKind::Pi => self.runtimes[0].clone(),
            BackendKind::Omp => self.runtimes[1].clone(),
            BackendKind::Dsh => self.runtimes[2].clone(),
        }
    }

    fn failure(message: impl Into<String>) -> ReviewResult {
        let message = message.into();
        ReviewResult {
            outcome: ReviewOutcome::Failed {
                message: message.clone(),
            },
            record: None,
            text: format!("Review failed: {message}"),
        }
    }

    fn run(
        &self,
        scope: &PlanScope,
        item_id: &PlanItemId,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> ReviewResult {
        let binding = match self.bindings.bindings(&scope.workspace) {
            Ok(items) => items.into_iter().find(|item| item.chat_id == scope.chat),
            Err(error) => return Self::failure(error.to_string()),
        };
        let Some(binding) = binding else {
            return Self::failure("worker chat is not bound");
        };
        let plan = match self.plans.plan(scope) {
            Ok(plan) => plan,
            Err(error) => return Self::failure(error.to_string()),
        };
        let Some(item) = plan_ops::items(&plan).find(|item| item.id == *item_id) else {
            return Self::failure("plan step disappeared");
        };
        if item.status != PlanStatus::Done || item.changes.is_empty() {
            return Self::failure("step is no longer reviewable");
        }
        let prefs = match self.prefs.prefs() {
            Ok(prefs) => prefs,
            Err(error) => return Self::failure(error.to_string()),
        };
        let project = match self.workspace.projects().and_then(|projects| {
            projects
                .into_iter()
                .find(|project| {
                    self.workspace
                        .workspaces(&project.id)
                        .ok()
                        .is_some_and(|items| items.iter().any(|item| item.id == scope.workspace))
                })
                .map(|project| project.id)
                .ok_or_else(|| thinkrail_core::Error::not_found("工作区项目"))
        }) {
            Ok(project) => project,
            Err(error) => return Self::failure(error.to_string()),
        };
        let reviewer_chat = ChatId::generate();
        let reviewer_span = tracing::info_span!("review.session", reviewer_chat_id = %reviewer_chat,
            chat_id = %scope.chat, workspace_id = %scope.workspace, backend = binding.backend.label());
        let _entered = reviewer_span.enter();
        tracing::info!("review.session_preparing");
        let wanted_model = prefs.review.model.clone().or(prefs.default_model.clone());
        let request = thinkrail_core::RouteRequest {
            chat: reviewer_chat.clone(),
            project,
            backend: binding.backend,
            new_session: true,
            model: wanted_model,
            effort: prefs.review.effort.or(prefs.default_effort),
        };
        let routing = match self.router.prepare(&request) {
            Ok(routing) => routing,
            Err(error) => return Self::failure(error.to_string()),
        };
        let cwd = match self.workspace.workspace_root(&scope.workspace) {
            Ok(path) => path,
            Err(error) => {
                self.router.release(&reviewer_chat);
                return Self::failure(error.to_string());
            }
        };
        let support = self.support_dir.join(reviewer_chat.as_str());
        if let Err(error) = fs::create_dir_all(&support) {
            self.router.release(&reviewer_chat);
            return Self::failure(error.to_string());
        }
        let system_prompt = review::REVIEWER_SYSTEM.to_owned();
        let package =
            review::review_package(review::Subject::Step(item), scope.chat.as_str(), None)
                .unwrap_or_default();
        let sink = Arc::new(ReviewerSink {
            state: Mutex::new(SinkState::default()),
            wake: Condvar::new(),
            span: tracing::Span::current(),
        });
        let launch = match self.engines.status(binding.backend) {
            Ok(Some(install)) => install.launch,
            Ok(None) => return Self::failure("review engine is not installed"),
            Err(error) => return Self::failure(error.to_string()),
        };
        let session_request = StartSession {
            chat: reviewer_chat.clone(),
            workspace: scope.workspace.clone(),
            backend: binding.backend,
            launch,
            config_root: None,
            cwd,
            resume: None,
            host_tools: None,
            routing,
            support_dir: support.clone(),
            role: SessionRole::Reviewer {
                system_prompt: format!("{system_prompt}\n\n{package}"),
            },
        };
        let session = match self
            .runtime(binding.backend)
            .start(session_request, sink.clone())
        {
            Ok(session) => session,
            Err(error) => {
                self.router.release(&reviewer_chat);
                let _ = fs::remove_dir_all(support);
                return Self::failure(error.to_string());
            }
        };
        let deadline = Instant::now() + TIMEOUT;
        let mut was_running = false;
        let final_text = loop {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                break None;
            }
            let mut state = sink
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(snapshot) = &state.snapshot {
                was_running |= snapshot.phase == SessionPhase::Running;
                if snapshot.phase == SessionPhase::Closed {
                    break None;
                }
                if was_running && snapshot.phase == SessionPhase::Idle {
                    break snapshot
                        .messages
                        .iter()
                        .rev()
                        .find(|message| message.role == Role::Assistant)
                        .and_then(|message| {
                            message.content.iter().rev().find_map(|block| match block {
                                ContentBlock::Text { text } => Some(text.clone()),
                                _ => None,
                            })
                        });
                }
                if snapshot.last_error.is_some() {
                    break None;
                }
            }
            if Instant::now() >= deadline {
                tracing::warn!(timeout_ms = TIMEOUT.as_millis() as u64, "review.timeout");
                break None;
            }
            let remain = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(250));
            let (next, _) = sink
                .wake
                .wait_timeout(state, remain)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
        };
        let parsed = final_text.and_then(|text| verdict::parse(&text));
        let result = if let Some(parsed) = parsed {
            let mut applied = None;
            let update = self.plans.update_reviews(scope, &mut |_, steps| {
                applied = Some(verdict::apply(
                    steps,
                    item_id,
                    parsed.clone(),
                    prefs.review.auto_fix,
                    prefs.review.model.clone(),
                    now_ms(),
                ));
                Ok(())
            });
            match (update, applied) {
                (Ok(_), Some(applied)) => ReviewResult {
                    text: review::result_text(
                        &item.title,
                        &applied.outcome,
                        applied
                            .record
                            .as_ref()
                            .and_then(|record| record.summary.as_deref()),
                        &applied.findings,
                    ),
                    outcome: applied.outcome,
                    record: applied.record,
                },
                (Err(error), _) => Self::failure(error.to_string()),
                _ => Self::failure("review result was not applied"),
            }
        } else {
            let _ = self.plans.update_reviews(scope, &mut |_, steps| {
                verdict::fail(steps, item_id);
                Ok(())
            });
            Self::failure("reviewer produced no valid verdict")
        };
        match session.dispose() {
            Ok(()) => tracing::info!("review.session_disposed"),
            Err(error) => tracing::warn!(
                code = thinkrail_core::ErrorCode::code(&error),
                "review.dispose_failed"
            ),
        }
        self.router.release(&reviewer_chat);
        match fs::remove_dir_all(support) {
            Ok(()) => tracing::info!("review.cleanup_finished"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => tracing::warn!("review.cleanup_failed"),
        }
        result
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
