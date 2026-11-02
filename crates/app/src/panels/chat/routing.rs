//! Routing changes for live chats: signature rechecks, model catalog and model/effort switches.

use std::{collections::HashMap, sync::Arc};

use gpui_kit::Context;
use thinkrail_core::{
    AgentSession, ChatId, ModelChoice, ProjectId, RouteRequest, SessionPhase, ThinkingLevel,
    WorkspaceId,
};

use super::{ChatPanel, engine::dispose_engine};
use crate::state::run;

#[cfg(test)]
mod tests;

type RoutingCheck = (ChatId, RouteRequest, Option<String>, Arc<dyn AgentSession>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingAction {
    Unchanged,
    Pending,
    RestartNow,
}

fn routing_action(
    old_signature: Option<&str>,
    new_signature: Option<&str>,
    phase: SessionPhase,
) -> RoutingAction {
    if new_signature.is_some() && old_signature == new_signature {
        return RoutingAction::Unchanged;
    }
    if phase == SessionPhase::Idle {
        RoutingAction::RestartNow
    } else {
        RoutingAction::Pending
    }
}
impl ChatPanel {
    /// Rechecks live chats with `new_session: false`, so default-model preferences never alter
    /// the signature of an existing native session.
    pub(super) fn on_routing_changed(&mut self, cx: &mut Context<Self>) {
        let services = self.services(cx);
        let project_ids: HashMap<WorkspaceId, ProjectId> = self
            .state
            .read(cx)
            .workspaces
            .iter()
            .map(|w| (w.id.clone(), w.project_id.clone()))
            .collect();
        let requests: Vec<RoutingCheck> = self
            .chats
            .iter()
            .filter_map(|(id, chat)| {
                let backend = chat.binding.as_ref()?.backend;
                let project = project_ids.get(&chat.workspace)?.clone();
                let session = chat.live()?.clone();
                Some((
                    id.clone(),
                    RouteRequest {
                        chat: id.clone(),
                        project,
                        backend,
                        new_session: false,
                    },
                    chat.routing_signature.clone(),
                    session,
                ))
            })
            .collect();
        for (id, request, old_signature, session) in requests {
            self.refresh_catalog(&id, cx);
            let router = services.router.clone();
            run(
                cx,
                move || router.signature(&request),
                move |this, result, cx| {
                    let services = this.services(cx);
                    let Some(chat) = this.chats.get_mut(&id) else {
                        return;
                    };
                    if !chat
                        .session
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &session))
                    {
                        return;
                    }
                    let signature = match result {
                        Ok(signature) => signature,
                        Err(error) => {
                            chat.restart_pending = true;
                            if chat.phase() == SessionPhase::Idle {
                                chat.closing = true;
                                let current = chat.session.take();
                                dispose_engine(&services, id.clone(), current, false, cx);
                            }
                            this.report(&error, cx);
                            return;
                        }
                    };
                    match routing_action(
                        old_signature.as_deref(),
                        Some(signature.as_str()),
                        chat.phase(),
                    ) {
                        RoutingAction::Unchanged => (),
                        RoutingAction::Pending => {
                            chat.restart_pending = true;
                            cx.notify();
                        }
                        RoutingAction::RestartNow => {
                            let current_matches = chat
                                .session
                                .as_ref()
                                .is_some_and(|current| Arc::ptr_eq(current, &session));
                            if !current_matches {
                                return;
                            }
                            chat.restart_pending = false;
                            chat.closing = true;
                            let current = chat.session.take();
                            dispose_engine(&services, id.clone(), current, false, cx);
                        }
                    }
                },
            );
        }
    }

    pub(super) fn refresh_catalog(&mut self, id: &ChatId, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(id) else {
            return;
        };
        let Some(backend) = chat.binding.as_ref().map(|b| b.backend) else {
            return;
        };
        let router = self.services(cx).router;
        let id = id.clone();
        run(
            cx,
            move || router.catalog(backend),
            move |this, result, cx| match result {
                Ok(catalog) => {
                    if let Some(chat) = this.chats.get_mut(&id) {
                        chat.catalog = catalog;
                        cx.notify();
                    }
                }
                Err(error) => this.report(&error, cx),
            },
        );
    }

    pub(super) fn set_model(
        &mut self,
        id: ChatId,
        choice: ModelChoice,
        effort: Option<ThinkingLevel>,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.chats.get(&id) else {
            return;
        };
        let Some(session) = chat.live().cloned() else {
            return;
        };
        if !chat
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.capabilities.model_switch)
            || chat.closing
            || chat.phase() != SessionPhase::Idle
        {
            return;
        }
        let effort = effort.filter(|effort| choice.model.efforts.contains(effort));
        run(
            cx,
            move || session.set_model(&choice.route, &choice.model.id, effort),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }

    pub(super) fn set_effort(&mut self, id: ChatId, effort: ThinkingLevel, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(&id) else {
            return;
        };
        let Some(session) = chat.live().cloned() else {
            return;
        };
        let Some(model) = chat.snapshot.as_ref().and_then(|s| s.model.as_ref()) else {
            return;
        };
        let provider = model.provider.clone().unwrap_or_default();
        let model_id = model.id.clone();
        let Some(choice) = chat
            .catalog
            .iter()
            .find(|choice| choice.route == provider && choice.model.id == model_id)
        else {
            return;
        };
        if !chat
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.capabilities.model_switch)
            || chat.closing
            || chat.phase() != SessionPhase::Idle
            || !choice.model.efforts.contains(&effort)
        {
            return;
        }
        run(
            cx,
            move || session.set_model(&provider, &model_id, Some(effort)),
            |this, result, cx| {
                if let Err(error) = result {
                    this.report(&error, cx);
                }
            },
        );
    }
}
