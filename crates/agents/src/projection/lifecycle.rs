//! Turn lifecycle transitions, with spans owned until the actual settle or close boundary.

use thinkrail_core::SessionError;

use super::Projection;

impl Projection {
    pub(crate) fn set_running(&mut self, running: bool) {
        if self.running == running {
            return;
        }
        let _session = self
            .trace
            .as_ref()
            .map(|trace| trace.enter("agent.turn.transition"));
        self.running = running;
        if running {
            self.turn_span = self.trace.as_ref().map(|trace| trace.span("agent.turn"));
            let _entered = self.turn_span.as_ref().map(tracing::Span::enter);
            tracing::info!(chat_id = %self.snapshot.id, "agent.turn.started");
            self.turn_started = Some(std::time::Instant::now());
        } else if let Some(started) = self.turn_started.take() {
            let span = self.turn_span.take();
            let _entered = span.as_ref().map(tracing::Span::enter);
            tracing::info!(chat_id = %self.snapshot.id,
                duration_ms = started.elapsed().as_millis() as u64, "agent.turn.settled");
        }
        self.publish();
    }

    /// Final transition: the engine is gone. Pending interactions die with it.
    pub(crate) fn close(&mut self, error: Option<SessionError>) {
        if self.closed {
            return;
        }
        let _session = self
            .trace
            .as_ref()
            .map(|trace| trace.enter("agent.turn.close"));
        self.closed = true;
        self.running = false;
        if let Some(started) = self.turn_started.take() {
            let span = self.turn_span.take();
            let _entered = span.as_ref().map(tracing::Span::enter);
            tracing::warn!(chat_id = %self.snapshot.id,
                duration_ms = started.elapsed().as_millis() as u64, "agent.turn.interrupted");
        }
        self.snapshot.interactions.clear();
        if let Some(error) = error {
            self.error(error);
        }
        self.publish();
    }
}
