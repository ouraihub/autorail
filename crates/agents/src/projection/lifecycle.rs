//! Turn lifecycle transitions, with spans owned until the actual settle or close boundary.

use thinkrail_core::SessionError;

use super::Projection;

impl Projection {
    /// Async engine errors have no Result consumer; duplicate frames share one failed turn.
    pub(super) fn log_error_boundary(&self, error: &SessionError) {
        if !self.running || self.turn_started.is_some() {
            let _entered = self
                .trace
                .as_ref()
                .map(|trace| trace.enter("agent.session.error"));
            tracing::error!(chat_id = %self.snapshot.id, code = %error.code,
                "agent.session.failed");
        }
    }

    pub(crate) fn set_running(&mut self, running: bool) {
        let _entered = self
            .trace
            .as_ref()
            .map(|trace| trace.enter("agent.turn.state"));
        if self.running == running {
            return;
        }
        self.running = running;
        if running {
            if let Some(trace) = &self.trace {
                self.turn_operation = Some(trace.operation("agent.turn", "service"));
            }
            tracing::info!(chat_id = %self.snapshot.id, "agent.turn.started");
            self.turn_started = Some(std::time::Instant::now());
        } else if let Some(started) = self.turn_started.take() {
            let completed = if let Some(operation) = self.turn_operation.take() {
                operation.finish("SUCCESS");
                true
            } else {
                false
            };
            if completed {
                tracing::info!(chat_id = %self.snapshot.id,
                    duration_ms = started.elapsed().as_millis() as u64, "agent.turn.settled");
            }
        }
        self.publish();
    }

    /// Final transition: the engine is gone. Pending interactions die with it.
    pub(crate) fn close(&mut self, error: Option<SessionError>) {
        if self.closed {
            return;
        }
        if let Some(error) = error {
            self.error(error);
        }
        self.closed = true;
        self.running = false;
        if self.turn_started.take().is_some()
            && let Some(operation) = self.turn_operation.take()
        {
            operation.cancel("session_closed");
            tracing::debug!(chat_id = %self.snapshot.id, "agent.turn.interrupted");
        }
        self.snapshot.interactions.clear();
        self.publish();
    }
}
