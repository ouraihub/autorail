//! Owned diagnostic context for one engine session, explicitly restored at every worker boundary.
//!
//! The session is a correlation context that lives until dispose, not one bounded operation, so
//! its root is closed as `REGISTERED` right away; boundaries below run inside its span and carry
//! the same TraceId. Bounded process, turn, request and tool operations run beneath that context.

use thinkrail_core::StartSession;
use thinkrail_observability::{OperationGuard, TraceContext, operation};
use tracing::Span;
pub(crate) struct SessionTrace {
    /// The context is explicitly carried to every worker boundary.
    pub(crate) context: TraceContext,
    span: Span,
    dispatch: tracing::Dispatch,
}

pub(crate) struct TraceGuard {
    _entered: tracing::span::EnteredSpan,
    _dispatch: tracing::dispatcher::DefaultGuard,
}

impl SessionTrace {
    pub(crate) fn new(request: &StartSession) -> Self {
        let (context, root) = TraceContext::root("agent.session", "agents");
        let span = tracing::info_span!(
            parent: root.span(),
            "agent.session",
            component = "agents",
            chat_id = %request.chat,
            workspace_id = %request.workspace,
            backend = request.backend.label(),
        );
        root.finish("REGISTERED");
        Self {
            context,
            span,
            dispatch: tracing::dispatcher::get_default(Clone::clone),
        }
    }

    /// Starts a bounded lifecycle operation under this session's span.
    pub(crate) fn operation(&self, name: &'static str, layer: &'static str) -> OperationGuard {
        let _dispatch = tracing::dispatcher::set_default(&self.dispatch);
        let _entered = self.span.enter();
        operation(&self.context, name, layer)
    }

    pub(crate) fn span(&self, operation: &'static str) -> Span {
        let _dispatch = tracing::dispatcher::set_default(&self.dispatch);
        tracing::info_span!(parent: &self.span, "agent.boundary", boundary = operation)
    }

    pub(crate) fn enter(&self, operation: &'static str) -> TraceGuard {
        let dispatch = tracing::dispatcher::set_default(&self.dispatch);
        TraceGuard {
            _entered: self.span(operation).entered(),
            _dispatch: dispatch,
        }
    }

    pub(crate) fn enter_operation(&self, operation: &OperationGuard) -> TraceGuard {
        let dispatch = tracing::dispatcher::set_default(&self.dispatch);
        let span = operation.span().clone();
        TraceGuard {
            _entered: span.entered(),
            _dispatch: dispatch,
        }
    }
}

#[cfg(test)]
mod tests;
