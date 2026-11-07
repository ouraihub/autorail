//! Review thread ownership and traced lifecycle; business execution stays in the parent module.

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

impl ReviewRunner for AppReviewRunner {
    fn start(
        &self,
        scope: &PlanScope,
        item: &PlanItemId,
        origin: ReviewOrigin,
    ) -> thinkrail_core::Result<thinkrail_core::StartedReview> {
        let key = (
            scope.workspace.to_string(),
            scope.chat.to_string(),
            item.clone(),
        );
        let mut running = self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = running.get(&key) {
            existing.span.in_scope(|| tracing::debug!("review.joined"));
            return Ok(thinkrail_core::StartedReview {
                handle: existing.handle.clone(),
                joined: true,
            });
        }
        let runner = self
            .self_ref
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| thinkrail_core::Error::external("review runner unavailable"))?;
        let context = thinkrail_observability::TraceContext::root("review.run");
        let (trace, root) = match context {
            Ok((trace, span)) => (Some(trace), span),
            Err(_) => {
                tracing::warn!("review.trace_unavailable");
                (None, tracing::Span::current())
            }
        };
        let span = tracing::info_span!(parent: &root, "review", chat_id = %scope.chat,
            workspace_id = %scope.workspace, item_id = %item);
        let handle = ReviewHandle::default();
        let cancelled = Arc::new(AtomicBool::new(false));
        let active = Running {
            handle: handle.clone(),
            span: span.clone(),
            cancelled: cancelled.clone(),
        };
        let worker_scope = scope.clone();
        let worker_item = item.clone();
        let worker_handle = handle.clone();
        let worker_key = key.clone();
        thread::Builder::new()
            .name("plan-review".into())
            .spawn(move || {
                let _entered = span.enter();
                tracing::info!("review.started");
                let _ = runner.events.send_blocking((
                    span.clone(),
                    ReviewEvent::Started {
                        scope: worker_scope.clone(),
                        item: worker_item.clone(),
                    },
                ));
                let operation = trace
                    .as_ref()
                    .map(|trace| thinkrail_observability::operation(trace, "review.run", "review"));
                let result = runner.run(&worker_scope, &worker_item, &cancelled);
                if let Some(operation) = operation {
                    if cancelled.load(Ordering::Acquire) {
                        operation.cancel("user");
                    } else {
                        match &result.outcome {
                            ReviewOutcome::Failed { .. } => operation.fail("review_failed"),
                            ReviewOutcome::Approved
                            | ReviewOutcome::ApproveBlocked { .. }
                            | ReviewOutcome::ChangesFix
                            | ReviewOutcome::ChangesStop => operation.finish("ok"),
                        }
                    }
                }
                tracing::info!(
                    cancelled = cancelled.load(Ordering::Acquire),
                    failed = matches!(result.outcome, ReviewOutcome::Failed { .. }),
                    "review.finished"
                );
                worker_handle.finish(result.clone());
                if !matches!(origin, ReviewOrigin::Tool { waits: true }) {
                    let _ = runner.events.send_blocking((
                        span.clone(),
                        ReviewEvent::Finished {
                            scope: worker_scope,
                            item: worker_item,
                            result,
                        },
                    ));
                }
                runner
                    .running
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&worker_key);
            })
            .map_err(|error| thinkrail_core::Error::external(error.to_string()))?;
        running.insert(key, active);
        Ok(thinkrail_core::StartedReview {
            handle,
            joined: false,
        })
    }

    fn cancel(&self, scope: &PlanScope) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for ((workspace, chat, _), review) in running.iter() {
            if workspace == scope.workspace.as_str() && chat == scope.chat.as_str() {
                review.span.in_scope(|| {
                    review.cancelled.store(true, Ordering::Release);
                    tracing::info!("review.cancel_requested");
                    review.handle.finish(Self::failure("review cancelled"));
                });
            }
        }
    }
}
