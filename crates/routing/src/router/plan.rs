//! Route planning: which (provider, protocol) routes and models an engine gets, and its default model.

use thinkrail_core::{
    Access, BackendKind, DefaultModel, EngineProvider, Error, ModelPrefs, ModelRef, ProjectedModel,
    Protocol, Provider, ProviderModel, Result, ThinkingLevel, clamp_effort,
};
use tracing::warn;

use super::route_name;

const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;
const DEFAULT_MAX_TOKENS: u32 = 16_384;

/// Protocols each engine speaks (04 §4.5).
fn supports(backend: BackendKind, protocol: Protocol) -> bool {
    match (backend, protocol) {
        (
            BackendKind::Pi | BackendKind::Omp | BackendKind::Dsh,
            Protocol::OpenaiChat | Protocol::OpenaiResponses | Protocol::AnthropicMessages,
        ) => true,
    }
}

pub(super) fn no_enabled_key(provider: &Provider) -> Error {
    Error::invalid(format!(
        "提供方 {} 没有启用的 API 密钥，请在设置 → 模型服务中启用一个。",
        provider.input.name
    ))
}

/// One (provider, protocol) pair that has models for the engine (04 §4.3 step 2).
pub(super) struct PlannedRoute<'a> {
    pub(super) provider: &'a Provider,
    pub(super) protocol: Protocol,
    pub(super) endpoint: &'a str,
    pub(super) route: String,
    pub(super) gateway: bool,
    pub(super) models: Vec<ProjectedModel>,
}

/// The override set on the GPT 上下文窗口 settings for `model` of `provider`, if any.
pub(super) fn context_override(
    overrides: &[(ModelRef, u32)],
    provider: &Provider,
    model: &ProviderModel,
) -> Option<u32> {
    overrides
        .iter()
        .find(|(m, _)| m.provider == provider.id && m.model_id == model.model_id)
        .map(|(_, tokens)| *tokens)
}

/// The engine-facing model; the context window is the override, else the provider setting, else
/// [`DEFAULT_CONTEXT_WINDOW`] (agent-settings 08 §6).
pub(super) fn project_model(
    model: &ProviderModel,
    context_override: Option<u32>,
) -> ProjectedModel {
    ProjectedModel {
        id: model.model_id.clone(),
        name: model.name.clone(),
        vision: model.vision,
        reasoning: model.reasoning,
        efforts: if model.reasoning {
            model.efforts.clone()
        } else {
            Vec::new()
        },
        context_window: context_override
            .or(model.context_window)
            .unwrap_or(DEFAULT_CONTEXT_WINDOW),
        max_tokens: model.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
    }
}

/// Steps 1–3 of 04 §4.3: the providers the engine's 编码搭档 choice allows × supported endpoints
/// × enabled models. No choice and 统一网关 take every enabled provider (统一网关 forces the
/// gateway); 官方 projects none; one provider keeps its own access mode and must be enabled.
pub(super) fn plan<'a>(
    providers: &'a [Provider],
    backend: BackendKind,
    choice: Option<&EngineProvider>,
    overrides: &[(ModelRef, u32)],
) -> Result<Vec<PlannedRoute<'a>>> {
    let chosen: Vec<&Provider> = match choice {
        Some(EngineProvider::Official) => Vec::new(),
        None | Some(EngineProvider::Gateway) => {
            providers.iter().filter(|p| p.input.enabled).collect()
        }
        Some(EngineProvider::Provider(id)) => {
            match providers.iter().find(|p| &p.id == id && p.input.enabled) {
                Some(provider) => vec![provider],
                None => {
                    return Err(Error::invalid(format!(
                        "{} 在编码搭档中选择的服务商已删除或停用，请在设置 → 编码搭档中重新选择。",
                        backend.label()
                    )));
                }
            }
        }
    };
    let forced_gateway = choice == Some(&EngineProvider::Gateway);
    let mut routes = Vec::new();
    for provider in chosen {
        // DSH cannot reference header values from the environment (04 §6.4).
        let gateway = forced_gateway
            || provider.input.access == Access::Gateway
            || (backend == BackendKind::Dsh && !provider.input.headers.is_empty());
        for endpoint in &provider.input.endpoints {
            if !supports(backend, endpoint.protocol) {
                continue;
            }
            let models: Vec<ProjectedModel> = provider
                .input
                .models
                .iter()
                .filter(|m| usable(m) && m.protocols.contains(&endpoint.protocol))
                .map(|m| project_model(m, context_override(overrides, provider, m)))
                .collect();
            if models.is_empty() {
                continue;
            }
            routes.push(PlannedRoute {
                provider,
                protocol: endpoint.protocol,
                endpoint: &endpoint.base_url,
                route: route_name(provider.id.as_str(), endpoint.protocol),
                gateway,
                models,
            });
        }
    }
    Ok(routes)
}

/// The route a model is reached through: its first protocol that has a planned route.
pub(super) fn route_for<'a>(
    routes: &'a [PlannedRoute<'_>],
    provider: &Provider,
    model: &ProviderModel,
) -> Option<&'a str> {
    model.protocols.iter().find_map(|protocol| {
        routes
            .iter()
            .find(|r| {
                r.provider.id == provider.id
                    && r.protocol == *protocol
                    && r.models.iter().any(|m| m.id == model.model_id)
            })
            .map(|r| r.route.as_str())
    })
}

/// `wanted` through the planned routes, its effort clamped to what the model declares; `None`
/// when the provider or model is gone, disabled, not a chat model or not routed for this engine.
pub(super) fn resolve_model(
    routes: &[PlannedRoute<'_>],
    providers: &[Provider],
    wanted: &ModelRef,
    effort: Option<ThinkingLevel>,
) -> Option<DefaultModel> {
    providers
        .iter()
        .filter(|p| p.input.enabled && p.id == wanted.provider)
        .find_map(|provider| {
            let model = provider
                .input
                .models
                .iter()
                .find(|m| usable(m) && m.model_id == wanted.model_id)?;
            let route = route_for(routes, provider, model)?;
            let efforts: &[ThinkingLevel] = if model.reasoning { &model.efforts } else { &[] };
            Some(DefaultModel {
                route: route.to_owned(),
                model_id: model.model_id.clone(),
                effort: clamp_effort(effort, efforts),
            })
        })
}

/// The new session's model: the request's pinned model (which must route), else the saved
/// default (which the engine's own default replaces when it cannot route). `effort` replaces the
/// saved default effort when given.
pub(super) fn session_model(
    routes: &[PlannedRoute<'_>],
    providers: &[Provider],
    prefs: &ModelPrefs,
    pinned: Option<&ModelRef>,
    effort: Option<ThinkingLevel>,
) -> Result<Option<DefaultModel>> {
    let effort = effort.or(prefs.default_effort);
    if let Some(wanted) = pinned {
        return resolve_model(routes, providers, wanted, effort)
            .map(Some)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "所选模型「{}」无法通过该引擎的编码搭档路由",
                    wanted.model_id
                ))
            });
    }
    let Some(wanted) = prefs.default_model.as_ref() else {
        return Ok(None);
    };
    let found = resolve_model(routes, providers, wanted, effort);
    if found.is_none() {
        warn!(
            provider = wanted.provider.as_str(),
            code = "NOT_FOUND",
            "routing.model.default_unavailable"
        );
    }
    Ok(found)
}

/// Engines get enabled chat models only; image, embedding and other types stay in settings.
pub(super) fn usable(model: &thinkrail_core::ProviderModel) -> bool {
    model.enabled && model.model_type == thinkrail_core::ModelType::Chat
}
