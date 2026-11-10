//! Turn lifecycle transitions, with spans owned until the actual settle or close boundary.

use thinkrail_core::SessionError;

use super::Projection;

impl Projection {
    pub(crate) fn set_running(&mut self, running: bool) {
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
        self.closed = true;
        self.running = false;
        if self.turn_started.take().is_some()
            && let Some(operation) = self.turn_operation.take()
        {
            if error.is_some() {
                operation.fail("SESSION_CLOSED");
            } else {
                operation.cancel("session_closed");
            }
            tracing::warn!(chat_id = %self.snapshot.id, "agent.turn.interrupted");
        }
        self.snapshot.interactions.clear();
        if let Some(error) = error {
            self.error(error);
        }
        self.publish();
    }
}
