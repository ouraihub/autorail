//! Engine start, launch and disposal, run off the UI thread.

use std::sync::Arc;

use gpui_kit::Context;
use thinkrail_core::{
    AgentSession, BackendKind, ChatBinding, ChatId, Error, Readiness, Result, RouteRequest,
    SessionRole, StartSession,
};
use tracing::{debug, info};

use super::{ChatPanel, StartInput, Started};
use crate::{services::Services, state::run};

/// `Ok(Err(readiness))` when no usable engine was found (07 §5.1): guidance, not an error.
pub(super) fn start_engine(input: StartInput) -> Result<std::result::Result<Started, Readiness>> {
    // A bound chat keeps its backend; a new one takes the default Agent (07 §5.3).
    let (backend, launch) = match &input.binding {
        Some(b) => match input.services.launch(b.backend)? {
            Some(launch) => (b.backend, launch),
            None => return Ok(Err(Readiness::DefaultMissing(b.backend))),
        },
        None => match input.services.readiness()? {
            (Readiness::Ready(backend), Some(launch)) => (backend, launch),
            (readiness, _) => return Ok(Err(readiness)),
        },
    };
    launch_engine(input, backend, launch).map(Ok)
}

fn launch_engine(
    input: StartInput,
    backend: BackendKind,
    launch: thinkrail_core::Launch,
) -> Result<Started> {
    let StartInput {
        services,
        chat,
        workspace,
        project,
        cwd,
        binding,
        sink,
        prompt,
    } = input;
    let route_request = RouteRequest {
        chat: chat.clone(),
        project,
        backend,
        new_session: binding.is_none(),
        model: None,
        effort: None,
    };
    let routing = match services.router.prepare(&route_request) {
        Ok(routing) => routing,
        Err(error) => {
            services.router.release(&chat);
            return Err(error);
        }
    };
    let routing_signature = match services.router.signature(&RouteRequest {
        new_session: false,
        ..route_request.clone()
    }) {
        Ok(signature) => signature,
        Err(error) => {
            services.router.release(&chat);
            return Err(error);
        }
    };
    let support_dir = match services.support_dir(&chat) {
        Ok(path) => path,
        Err(error) => {
            services.router.release(&chat);
            return Err(error);
        }
    };
    let host_tool_options = match services.host_tool_options(backend) {
        Ok(options) => options,
        Err(error) => {
            services.router.release(&chat);
            return Err(error);
        }
    };
    let host_tools = services.host_tools(&workspace, &chat, host_tool_options);
    let request = StartSession {
        chat: chat.clone(),
        workspace: workspace.clone(),
        backend,
        launch,
        config_root: None,
        cwd,
        resume: binding.as_ref().map(|b| b.native_session_key.clone()),
        host_tools,
        routing,
        support_dir,
        role: SessionRole::Worker,
    };
    let session = match services.runtime(backend).start(request, sink) {
        Ok(session) => session,
        Err(error) => {
            services.router.release(&chat);
            let _ = services.remove_support_dir(&chat);
            return Err(error);
        }
    };
    let binding = match binding {
        Some(binding) => binding,
        None => {
            let key = match session.snapshot().native_session_key {
                Some(key) => key,
                None => {
                    let _ = session.dispose();
                    services.router.release(&chat);
                    let _ = services.remove_support_dir(&chat);
                    return Err(Error::protocol("Agent 没有返回会话 locator"));
                }
            };
            let binding = ChatBinding {
                chat_id: chat.clone(),
                workspace_id: workspace,
                backend,
                native_session_key: key,
            };
            if let Err(error) = services.bindings.bind(binding.clone()) {
                let _ = session.dispose();
                services.router.release(&chat);
                let _ = services.remove_support_dir(&chat);
                return Err(error);
            }
            binding
        }
    };
    let prompt_error = prompt.and_then(|text| session.prompt(&text).err());
    Ok(Started {
        session,
        binding,
        routing_signature,
        host_tool_options,
        prompt_error,
    })
}

/// Disposes an engine, revokes routing and removes generated files. `unbind` only when closing.
pub(super) fn dispose_engine(
    services: &Services,
    chat: ChatId,
    session: Option<Arc<dyn AgentSession>>,
    unbind: bool,
    cx: &mut Context<ChatPanel>,
) {
    let bindings = services.bindings.clone();
    let router = services.router.clone();
    let support = services.clone();
    let callback_chat = chat.clone();
    run(
        cx,
        move || {
            if let Some(session) = session
                && let Err(error) = session.dispose()
            {
                debug!(code = %error, "dispose of a closed chat failed");
            }
            router.release(&chat);
            let cleanup = support.remove_support_dir(&chat);
            if unbind {
                bindings.unbind(&chat)?;
            }
            info!(chat = %chat, "chat engine disposed");
            cleanup
        },
        move |this, result, cx| {
            if let Some(chat) = this.chats.get_mut(&callback_chat) {
                chat.closing = false;
            }
            if let Err(error) = result {
                this.report(&error, cx);
            }
            cx.notify();
        },
    );
}
