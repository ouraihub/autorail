//! Owned diagnostic context for one engine session, explicitly restored at every worker boundary.

use thinkrail_core::StartSession;
use thinkrail_observability::TraceContext;
use tracing::Span;

pub(crate) struct SessionTrace {
    context: Option<TraceContext>,
    span: Span,
    dispatch: tracing::Dispatch,
}

pub(crate) struct TraceGuard {
    _entered: tracing::span::EnteredSpan,
    _dispatch: tracing::dispatcher::DefaultGuard,
}

impl SessionTrace {
    pub(crate) fn new(request: &StartSession) -> Self {
        let (context, root) = match TraceContext::root("agent.session") {
            Ok((context, span)) => (Some(context), span),
            Err(_) => {
                tracing::warn!("agent.trace.unavailable");
                (None, Span::current())
            }
        };
        let span = tracing::info_span!(
            parent: &root,
            "agent.session",
            trace_id = context.as_ref().map(TraceContext::trace_id),
            operation = "agent.session",
            component = "agents",
            chat_id = %request.chat,
            workspace_id = %request.workspace,
            backend = request.backend.label(),
        );
        Self {
            context,
            span,
            dispatch: tracing::dispatcher::get_default(Clone::clone),
        }
    }

    pub(crate) fn span(&self, operation: &'static str) -> Span {
        let _dispatch = tracing::dispatcher::set_default(&self.dispatch);
        let _entered = self.span.enter();
        let child = self
            .context
            .as_ref()
            .map(|context| context.child(operation));
        let parent = child
            .as_ref()
            .filter(|span| !span.is_disabled())
            .unwrap_or(&self.span);
        tracing::info_span!(parent: parent, "agent.boundary",
            trace_id = self.context.as_ref().map(TraceContext::trace_id), operation)
    }

    pub(crate) fn enter(&self, operation: &'static str) -> TraceGuard {
        let dispatch = tracing::dispatcher::set_default(&self.dispatch);
        TraceGuard {
            _entered: self.span(operation).entered(),
            _dispatch: dispatch,
        }
    }
}

#[cfg(test)]
mod tests;
