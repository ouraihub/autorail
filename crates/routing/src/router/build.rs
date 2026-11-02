//! Launch routing assembly: projected providers, launch environment and the routing signature.

use thinkrail_core::{
    EngineRouting, EnvBinding, ProjectedProvider, Provider, Result, RouteRequest, Secret,
};

use super::{
    Mode, NO_KEY, Router,
    plan::{default_model, no_enabled_key, plan},
    short_id,
};
use crate::{
    signature::{self, Signed, SignedMcp, SignedRoute},
    url,
};

impl Router {
    pub(super) fn build(&self, request: &RouteRequest, mode: Mode) -> Result<EngineRouting> {
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
