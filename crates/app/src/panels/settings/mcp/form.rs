//! MCP server form values and their validation into a server input.

use thinkrail_core::{EnvValue, McpScope, McpServerInput, McpTransport};

use super::TransportKind;
use crate::strings::settings_mcp as s;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Form {
    pub name: String,
    pub transport: TransportKind,
    pub command: String,
    pub args: String,
    pub url: String,
    pub env: Vec<EnvValue>,
    pub headers: Vec<EnvValue>,
    pub description: String,
    pub enabled: bool,
    pub scope: McpScope,
}

fn validate_pairs(values: &[EnvValue], error: &'static str) -> Result<Vec<EnvValue>, &'static str> {
    if values.iter().any(|pair| pair.name.trim().is_empty()) {
        return Err(error);
    }
    Ok(values
        .iter()
        .map(|pair| EnvValue {
            name: pair.name.trim().to_owned(),
            value: pair.value.clone(),
        })
        .collect())
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub(super) fn to_input(form: &Form) -> Result<McpServerInput, &'static str> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err(s::NAME_REQUIRED);
    }
    if !valid_name(name) {
        return Err(s::NAME_INVALID);
    }
    let transport = match form.transport {
        TransportKind::Stdio => {
            let command = form.command.trim();
            if command.is_empty() {
                return Err(s::COMMAND_REQUIRED);
            }
            McpTransport::Stdio {
                command: command.to_owned(),
                args: form
                    .args
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(ToOwned::to_owned)
                    .collect(),
                env: validate_pairs(&form.env, s::ENV_INVALID)?,
            }
        }
        TransportKind::Http => {
            let url = form.url.trim();
            validate_url(url)?;
            McpTransport::Http {
                url: url.to_owned(),
                headers: validate_pairs(&form.headers, s::HEADERS_INVALID)?,
            }
        }
    };
    Ok(McpServerInput {
        name: name.to_owned(),
        scope: form.scope.clone(),
        transport,
        enabled: form.enabled,
        description: (!form.description.trim().is_empty())
            .then(|| form.description.trim().to_owned()),
    })
}

pub(super) fn validate_url(url: &str) -> Result<(), &'static str> {
    let parsed = url::Url::parse(url).map_err(|_| s::URL_REQUIRED)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(s::URL_REQUIRED);
    }
    Ok(())
}
