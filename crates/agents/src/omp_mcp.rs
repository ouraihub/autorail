//! OMP MCP servers (agent-settings 04 §6.3). OMP's RPC mode has no way to inject servers, so the
//! host merges its own entries into `<config root>/mcp.json`: only keys starting with `thinkrail-`
//! are replaced, every other entry and top-level field is kept, values are `${VAR}` references
//! to the variables the host injects, and the file is rewritten only when its content changes.

use std::{fs, io, path::Path};

use serde_json::{Map, Value, json};
use thinkrail_core::{
    EnvBinding, Error, ErrorKind, IoContext, ProjectedMcpServer, ProjectedMcpTransport, Result,
};

use crate::host;

/// Prefix of every server the host owns in `mcp.json`.
const PREFIX: &str = "thinkrail-";
const FILE: &str = "mcp.json";

fn references(bindings: &[EnvBinding]) -> Value {
    Value::Object(
        bindings
            .iter()
            .map(|b| (b.name.clone(), Value::from(format!("${{{}}}", b.var))))
            .collect(),
    )
}

/// The `mcpServers` entry of one projected server.
fn entry(server: &ProjectedMcpServer) -> Value {
    match &server.transport {
        ProjectedMcpTransport::Stdio { command, args, env } => {
            json!({"type": "stdio", "command": command, "args": args, "env": references(env)})
        }
        ProjectedMcpTransport::Http { url, headers } => {
            json!({"type": "http", "url": url, "headers": references(headers)})
        }
    }
}

fn not_json(path: &Path) -> Error {
    Error::invalid(format!(
        "{} 不是有效的 JSON，ThinkRail 不会覆盖它。",
        path.display()
    ))
}

/// The new text of `path` (currently `current`; `None` = missing) with `servers` as its only
/// `thinkrail-` entries, or `None` when the content would not change.
pub(crate) fn merge(
    path: &Path,
    current: Option<&str>,
    servers: &[ProjectedMcpServer],
) -> Result<Option<String>> {
    let before = match current {
        Some(text) => serde_json::from_str::<Value>(text).map_err(|_| not_json(path))?,
        None => Value::Object(Map::new()),
    };
    let mut after = before.clone();
    let Value::Object(root) = &mut after else {
        return Err(not_json(path));
    };
    if !root.contains_key("mcpServers") && servers.is_empty() {
        return Ok(None);
    }
    let Value::Object(entries) = root
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()))
    else {
        return Err(Error::invalid(format!(
            "{} 中的 mcpServers 不是对象，ThinkRail 不会覆盖它。",
            path.display()
        )));
    };
    entries.retain(|name, _| !name.starts_with(PREFIX));
    entries.extend(
        servers
            .iter()
            .map(|server| (server.name.clone(), entry(server))),
    );
    if after == before {
        return Ok(None);
    }
    serde_json::to_string_pretty(&after)
        .map(|text| Some(text + "\n"))
        .map_err(|e| Error::protocol(format!("无法编码 {}：{e}", path.display())))
}

/// Brings `<root>/mcp.json` in line with `servers`. Without servers an unparseable file is left
/// alone silently: there is nothing to project, and the launch must behave as before routing.
pub(crate) fn sync(root: &Path, servers: &[ProjectedMcpServer]) -> Result<()> {
    let path = root.join(FILE);
    let current = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context(format!("无法读取 {}", path.display())),
    };
    match merge(&path, current.as_deref(), servers) {
        Ok(Some(text)) => host::write_support(root, FILE, &text).map(drop),
        Ok(None) => Ok(()),
        Err(error) if servers.is_empty() && error.kind() == Some(ErrorKind::Invalid) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
