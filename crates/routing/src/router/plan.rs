//! Route planning: which (provider, protocol) routes and models an engine gets, and its default model.

use thinkrail_core::{
    Access, BackendKind, DefaultModel, EngineProvider, Error, ModelPrefs, ProjectedModel, Protocol,
    Provider, ProviderModel, Result,
};
use tracing::info;

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

pub(super) fn project_model(model: &ProviderModel) -> ProjectedModel {
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
        context_window: model.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
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
                .map(project_model)
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

pub(super) fn default_model(
    routes: &[PlannedRoute<'_>],
    providers: &[Provider],
    prefs: &ModelPrefs,
) -> Option<DefaultModel> {
    let wanted = prefs.default_model.as_ref()?;
    let found = providers
        .iter()
        .filter(|p| p.input.enabled && p.id == wanted.provider)
        .find_map(|provider| {
            let model = provider
                .input
                .models
                .iter()
                .find(|m| usable(m) && m.model_id == wanted.model_id)?;
            let route = route_for(routes, provider, model)?;
            let effort = if model.reasoning {
                prefs
                    .default_effort
                    .and_then(|effort| model.efforts.iter().copied().filter(|e| *e <= effort).max())
            } else {
                None
            };
            Some(DefaultModel {
                route: route.to_owned(),
                model_id: model.model_id.clone(),
                effort,
            })
        });
    if found.is_none() {
        info!(
            provider = wanted.provider.as_str(),
            "默认模型不可用，引擎使用自己的默认模型"
        );
    }
    found
}

/// Engines get enabled chat models only; image, embedding and other types stay in settings.
pub(super) fn usable(model: &thinkrail_core::ProviderModel) -> bool {
    model.enabled && model.model_type == thinkrail_core::ModelType::Chat
}
