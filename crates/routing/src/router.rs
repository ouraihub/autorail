//! `ModelRouter`: turns the global providers, MCP servers and model preferences into one engine
//! launch's routing (agent-settings 04 §4.3–4.6).

use std::sync::Arc;

use thinkrail_core::{
    Access, BackendKind, ChatId, DefaultModel, EngineProvider, EngineRouting, EnvBinding, EnvValue,
    Error, McpService, McpTransport, ModelChoice, ModelPrefService, ModelPrefs, ModelRouter,
    ProjectedMcpServer, ProjectedMcpTransport, ProjectedModel, ProjectedProvider, Protocol,
    Provider, ProviderModel, ProviderService, Result, RouteRequest, Secret,
};
use tracing::info;

use crate::{
    signature::{self, Signed, SignedMcp, SignedRoute},
    url,
};

/// Key placeholder for providers without any key (local model servers need no auth).
pub(crate) const NO_KEY: &str = "thinkrail-no-key";
const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;
const DEFAULT_MAX_TOKENS: u32 = 16_384;

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

/// Protocols each engine speaks (04 §4.5).
fn supports(backend: BackendKind, protocol: Protocol) -> bool {
    match (backend, protocol) {
        (
            BackendKind::Pi | BackendKind::Omp | BackendKind::Dsh,
            Protocol::OpenaiChat | Protocol::OpenaiResponses | Protocol::AnthropicMessages,
        ) => true,
    }
}

fn no_enabled_key(provider: &Provider) -> Error {
    Error::invalid(format!(
        "提供方 {} 没有启用的 API 密钥，请在设置 → 模型服务中启用一个。",
        provider.input.name
    ))
}

/// One (provider, protocol) pair that has models for the engine (04 §4.3 step 2).
struct PlannedRoute<'a> {
    provider: &'a Provider,
    protocol: Protocol,
    endpoint: &'a str,
    route: String,
    gateway: bool,
    models: Vec<ProjectedModel>,
}

fn project_model(model: &ProviderModel) -> ProjectedModel {
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
fn plan<'a>(
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
fn route_for<'a>(
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

fn default_model(
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

/// `$VAR` / `${VAR}` → the host launch environment; anything else is plaintext.
fn resolve(server: &str, value: &str) -> Result<String> {
    let var = match value
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    {
        Some(name) => Some(name),
        None => value.strip_prefix('$'),
    };
    match var {
        Some(name) if !name.is_empty() => std::env::var(name).map_err(|_| {
            Error::invalid(format!(
                "MCP 服务器 {server} 引用的环境变量 {name} 未设置。"
            ))
        }),
        _ => Ok(value.to_owned()),
    }
}

/// (variable, resolved value) pairs bound for the engine environment.
type Values = Vec<(String, String)>;

/// Projected MCP server plus its resolved values.
struct ResolvedMcp {
    server: ProjectedMcpServer,
    kind: &'static str,
    target: String,
    args: Vec<String>,
    values: Values,
}

fn bind(
    server: &str,
    short: &str,
    letter: char,
    values: &[EnvValue],
) -> Result<(Vec<EnvBinding>, Values)> {
    let mut bindings = Vec::with_capacity(values.len());
    let mut resolved = Vec::with_capacity(values.len());
    for (index, entry) in values.iter().enumerate() {
        let var = format!("THINKRAIL_MCP_{}_{letter}{index}", short.to_uppercase());
        resolved.push((var.clone(), resolve(server, &entry.value)?));
        bindings.push(EnvBinding {
            name: entry.name.clone(),
            var,
        });
    }
    Ok((bindings, resolved))
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

    /// Enabled effective servers for the project with their values resolved (04 §4.3 step 5).
    fn mcp_servers(&self, request: &RouteRequest) -> Result<Vec<ResolvedMcp>> {
        let mut out = Vec::new();
        for entry in self
            .mcp
            .effective(&request.project)?
            .into_iter()
            .filter(|e| e.enabled)
        {
            let server = &entry.server;
            let short = short_id(server.id.as_str());
            let display = server.input.name.as_str();
            let name = format!("thinkrail-{display}");
            let resolved = match &server.input.transport {
                McpTransport::Stdio { command, args, env } => {
                    let (bindings, values) = bind(display, &short, 'E', env)?;
                    let transport = ProjectedMcpTransport::Stdio {
                        command: command.clone(),
                        args: args.clone(),
                        env: bindings,
                    };
                    ResolvedMcp {
                        server: ProjectedMcpServer { name, transport },
                        kind: "stdio",
                        target: command.clone(),
                        args: args.clone(),
                        values,
                    }
                }
                McpTransport::Http { url, headers } => {
                    let (bindings, values) = bind(display, &short, 'H', headers)?;
                    let transport = ProjectedMcpTransport::Http {
                        url: url.clone(),
                        headers: bindings,
                    };
                    ResolvedMcp {
                        server: ProjectedMcpServer { name, transport },
                        kind: "http",
                        target: url.clone(),
                        args: Vec::new(),
                        values,
                    }
                }
            };
            out.push(resolved);
        }
        Ok(out)
    }

    fn build(&self, request: &RouteRequest, mode: Mode) -> Result<EngineRouting> {
        let providers = self.providers.providers()?;
        let prefs = self.prefs.prefs()?;
        let routes = plan(
            &providers,
            request.backend,
            prefs.engine_provider(request.backend),
        )?;
        let default_model = if request.new_session {
            default_model(&routes, &providers, &prefs)
        } else {
            None
        };
        let mcp = self.mcp_servers(request)?;

        let mut env: Vec<(String, Secret)> = Vec::new();
        let mut projected = Vec::with_capacity(routes.len());
        let mut keyed: Vec<&Provider> = Vec::new();
        let mut token: Option<Secret> = None;
        for route in &routes {
            let provider = route.provider;
            let short = short_id(provider.id.as_str()).to_uppercase();
            let key_var = format!("THINKRAIL_PROVIDER_{short}_KEY");
            let first_route_of_provider = !keyed.iter().any(|p| p.id == provider.id);
            if first_route_of_provider {
                keyed.push(provider);
                if !provider.keys.is_empty() && !provider.keys.iter().any(|k| k.enabled) {
                    return Err(no_enabled_key(provider));
                }
            }
            let (base_url, headers) = if route.gateway {
                if mode == Mode::Prepare && first_route_of_provider {
                    let token = token.get_or_insert_with(|| self.gateway.register(&request.chat));
                    env.push((key_var.clone(), token.clone()));
                }
                let base_url = match mode {
                    Mode::Prepare => self.gateway.base_url(&route.route, route.protocol)?,
                    Mode::Signature => String::new(),
                };
                (base_url, Vec::new())
            } else {
                let headers: Vec<EnvBinding> = provider
                    .input
                    .headers
                    .iter()
                    .enumerate()
                    .map(|(index, header)| EnvBinding {
                        name: header.name.clone(),
                        var: format!("THINKRAIL_HDR_{short}_{index}"),
                    })
                    .collect();
                if mode == Mode::Prepare && first_route_of_provider {
                    // One rotation per launch: all protocols of a provider share the selected key.
                    let key = if provider.keys.is_empty() {
                        Secret::new(NO_KEY)
                    } else {
                        self.providers
                            .next_key(&provider.id)?
                            .ok_or_else(|| no_enabled_key(provider))?
                            .value
                    };
                    env.push((key_var.clone(), key));
                    for (binding, header) in headers.iter().zip(&provider.input.headers) {
                        env.push((binding.var.clone(), Secret::new(header.value.clone())));
                    }
                }
                (url::base_url(route.endpoint, route.protocol), headers)
            };
            projected.push(ProjectedProvider {
                route: route.route.clone(),
                display_name: provider.input.name.clone(),
                protocol: route.protocol,
                base_url,
                key_var,
                headers,
                models: route.models.clone(),
            });
        }
        if mode == Mode::Prepare {
            for server in &mcp {
                env.extend(
                    server
                        .values
                        .iter()
                        .map(|(var, value)| (var.clone(), Secret::new(value.clone()))),
                );
            }
        }

        let signed = Signed {
            routes: routes
                .iter()
                .zip(&projected)
                .map(|(route, projected)| SignedRoute {
                    route: &route.route,
                    protocol: route.protocol,
                    base_url: (!route.gateway).then_some(projected.base_url.as_str()),
                    headers: route
                        .provider
                        .input
                        .headers
                        .iter()
                        .map(|h| (h.name.as_str(), h.value.as_str()))
                        .collect(),
                    models: &route.models,
                })
                .collect(),
            keys: keyed
                .iter()
                .map(|p| {
                    (
                        p.id.as_str(),
                        p.revision,
                        p.keys.iter().filter(|k| k.enabled).map(|k| &k.id).collect(),
                    )
                })
                .collect(),
            default_model: default_model.as_ref(),
            mcp: mcp
                .iter()
                .map(|server| SignedMcp {
                    name: &server.server.name,
                    kind: server.kind,
                    target: &server.target,
                    args: &server.args,
                    values: server
                        .values
                        .iter()
                        .map(|(var, value)| (var.as_str(), value.as_str()))
                        .collect(),
                })
                .collect(),
        };
        let signature = signature::compute(&signed);
        Ok(EngineRouting {
            providers: projected,
            default_model,
            mcp: mcp.into_iter().map(|server| server.server).collect(),
            env,
            signature,
        })
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
        let routes = plan(&providers, backend, prefs.engine_provider(backend))?;
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
                    model: project_model(model),
                });
            }
        }
        Ok(out)
    }
}

/// Engines get enabled chat models only; image, embedding and other types stay in settings.
fn usable(model: &thinkrail_core::ProviderModel) -> bool {
    model.enabled && model.model_type == thinkrail_core::ModelType::Chat
}
