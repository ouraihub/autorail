//! `ModelRouter`: turns the global providers, MCP servers and model preferences into one engine
//! launch's routing (agent-settings 04 §4.3–4.6).

mod build;
mod mcp;
mod plan;
mod settings;

use std::sync::Arc;

use thinkrail_core::{
    BackendKind, ChatId, EngineNote, EngineRouting, McpService, ModelChoice, ModelPrefService,
    ModelRef, ModelRouter, ModelStatus, Protocol, ProviderService, Result, RouteRequest, Secret,
};

use plan::{context_override, plan, project_model, route_for, usable};

/// Key placeholder for providers without any key (local model servers need no auth).
pub(crate) const NO_KEY: &str = "thinkrail-no-key";

/// The part of the loopback gateway the router depends on (04 §4.1); `Gateway` implements it.
pub trait GatewayRoutes: Send + Sync {
    /// Gateway address for `route`; binds the listener on first use.
    fn base_url(&self, route: &str, protocol: Protocol) -> Result<String>;
    /// A fresh session token for `chat`, replacing its previous one.
    fn register(&self, chat: &ChatId) -> Secret;
    fn release(&self, chat: &ChatId);
}

/// First 8 characters of an id, the stable part of route and variable names.
pub(crate) fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

pub(crate) fn protocol_suffix(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiChat => "oc",
        Protocol::OpenaiResponses => "or",
        Protocol::AnthropicMessages => "am",
    }
}

/// `thinkrail-<short id>-<oc|or|am>`.
pub(crate) fn route_name(provider: &str, protocol: Protocol) -> String {
    format!(
        "thinkrail-{}-{}",
        short_id(provider),
        protocol_suffix(protocol)
    )
}

/// Builds launch routing from the settings services.
pub struct Router {
    providers: Arc<dyn ProviderService>,
    mcp: Arc<dyn McpService>,
    prefs: Arc<dyn ModelPrefService>,
    gateway: Arc<dyn GatewayRoutes>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Advances key rotation and registers the chat's gateway token.
    Prepare,
    /// Read-only: only the signature is used.
    Signature,
}

impl Router {
    pub fn new(
        providers: Arc<dyn ProviderService>,
        mcp: Arc<dyn McpService>,
        prefs: Arc<dyn ModelPrefService>,
        gateway: Arc<dyn GatewayRoutes>,
    ) -> Self {
        Self {
            providers,
            mcp,
            prefs,
            gateway,
        }
    }
}

impl ModelRouter for Router {
    fn prepare(&self, request: &RouteRequest) -> Result<EngineRouting> {
        self.build(request, Mode::Prepare)
    }

    fn signature(&self, request: &RouteRequest) -> Result<String> {
        Ok(self.build(request, Mode::Signature)?.signature)
    }

    fn release(&self, chat: &ChatId) {
        self.gateway.release(chat);
    }

    fn catalog(&self, backend: BackendKind) -> Result<Vec<ModelChoice>> {
        let providers = self.providers.providers()?;
        let prefs = self.prefs.prefs()?;
        let routes = plan(
            &providers,
            backend,
            prefs.engine_provider(backend),
            &prefs.context_overrides,
        )?;
        let mut out = Vec::new();
        for provider in providers.iter().filter(|p| p.input.enabled) {
            for model in provider.input.models.iter().filter(|m| usable(m)) {
                let hidden = prefs
                    .hidden
                    .iter()
                    .any(|h| h.provider == provider.id && h.model_id == model.model_id);
                if hidden {
                    continue;
                }
                // A model reachable over several protocols is listed once, under its first route.
                let Some(route) = route_for(&routes, provider, model) else {
                    continue;
                };
                out.push(ModelChoice {
                    provider: provider.id.clone(),
                    provider_name: provider.input.name.clone(),
                    route: route.to_owned(),
                    model: project_model(
                        model,
                        context_override(&prefs.context_overrides, provider, model),
                    ),
                });
            }
        }
        Ok(out)
    }

    fn settings_catalog(&self) -> Result<Vec<ModelChoice>> {
        settings::catalog(&self.providers.providers()?, &self.prefs.prefs()?)
    }

    fn model_status(&self, model: &ModelRef) -> Result<ModelStatus> {
        settings::status(&self.providers.providers()?, model)
    }

    fn engine_notes(&self) -> Result<Vec<(BackendKind, EngineNote)>> {
        Ok(settings::notes(
            &self.providers.providers()?,
            &self.prefs.prefs()?,
        ))
    }
}
