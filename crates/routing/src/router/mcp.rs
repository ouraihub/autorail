//! MCP server projection: effective servers with their env/header values resolved and bound.

use thinkrail_core::{
    EnvBinding, EnvValue, Error, McpTransport, ProjectedMcpServer, ProjectedMcpTransport, Result,
    RouteRequest,
};

use super::{Router, short_id};

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
pub(super) struct ResolvedMcp {
    pub(super) server: ProjectedMcpServer,
    pub(super) kind: &'static str,
    pub(super) target: String,
    pub(super) args: Vec<String>,
    pub(super) values: Values,
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

impl Router {
    /// Enabled effective servers for the project with their values resolved (04 §4.3 step 5).
    pub(super) fn mcp_servers(&self, request: &RouteRequest) -> Result<Vec<ResolvedMcp>> {
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
}
