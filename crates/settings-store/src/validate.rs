//! Input validation with user-facing Chinese messages (agent-settings 04 §2.3, §3.3).
//!
//! Each function returns the normalized value that is stored: names, URLs, keys and labels are
//! trimmed, endpoints are put in canonical protocol order. Messages never echo key or header values.

use std::collections::HashSet;

use thinkrail_core::{
    Error, EnvValue, McpScope, McpServerInput, McpTransport, ProjectId, Protocol, ProviderInput, Result, Secret,
};

const MAX_NAME: usize = 64;
const MAX_LABEL: usize = 64;
const MAX_MODELS: usize = 500;
const MAX_MCP_NAME: usize = 32;
/// Header names the host sets itself; users cannot override them.
const RESERVED_HEADERS: &[&str] = &["authorization", "x-api-key", "host", "content-length"];

/// Canonical endpoint order, so stored and returned endpoints compare equal to the validated input.
pub(crate) fn protocol_rank(protocol: Protocol) -> u8 {
    match protocol {
        Protocol::OpenaiChat => 0,
        Protocol::OpenaiResponses => 1,
        Protocol::AnthropicMessages => 2,
    }
}

pub(crate) fn provider(mut input: ProviderInput) -> Result<ProviderInput> {
    input.name = input.name.trim().to_owned();
    let length = input.name.chars().count();
    if length == 0 || length > MAX_NAME || input.name.chars().any(char::is_control) {
        return Err(Error::invalid("请填写提供方名称（最多 64 个字符）。"));
    }
    if input.endpoints.is_empty() {
        return Err(Error::invalid("请至少填写一个 API 地址。"));
    }
    let mut protocols = HashSet::new();
    for endpoint in &mut input.endpoints {
        if !protocols.insert(endpoint.protocol) {
            return Err(Error::invalid("每种端点类型只能填写一个 API 地址。"));
        }
        endpoint.base_url = endpoint.base_url.trim().to_owned();
        http_url(&endpoint.base_url, "API 地址")?;
    }
    input.endpoints.sort_by_key(|endpoint| protocol_rank(endpoint.protocol));
    if !protocols.contains(&input.default_protocol) {
        return Err(Error::invalid("默认协议没有对应的 API 地址。"));
    }
    for header in &input.headers {
        if !is_header_name(&header.name) {
            return Err(Error::invalid(format!("请求头名称 {} 不合法。", header.name.escape_debug())));
        }
        if RESERVED_HEADERS.contains(&header.name.to_ascii_lowercase().as_str()) {
            return Err(Error::invalid(format!("请求头 {} 由 ThinkRail 管理，不能自定义。", header.name)));
        }
    }
    if input.models.len() > MAX_MODELS {
        return Err(Error::invalid("每个提供方最多 500 个模型。"));
    }
    let mut model_ids = HashSet::new();
    for model in &mut input.models {
        model.model_id = model.model_id.trim().to_owned();
        if model.model_id.is_empty() {
            return Err(Error::invalid("模型 ID 不能为空。"));
        }
        if !model_ids.insert(model.model_id.clone()) {
            return Err(Error::invalid(format!("模型 {} 重复。", model.model_id)));
        }
        if model.protocols.is_empty() || model.protocols.iter().any(|protocol| !protocols.contains(protocol)) {
            return Err(Error::invalid(format!("模型 {} 的端点类型没有对应的 API 地址。", model.model_id)));
        }
        if !model.reasoning && !model.efforts.is_empty() {
            return Err(Error::invalid(format!("模型 {} 未开启推理，不能设置推理档位。", model.model_id)));
        }
    }
    Ok(input)
}

/// A key value, trimmed. Uniqueness is checked against the database by the caller.
pub(crate) fn key(key: &Secret) -> Result<String> {
    let value = key.expose().trim();
    if value.is_empty() {
        return Err(Error::invalid("API 密钥不能为空"));
    }
    Ok(value.to_owned())
}

/// A key label, trimmed; blank means no label.
pub(crate) fn label(label: Option<String>) -> Result<Option<String>> {
    let Some(label) = label else { return Ok(None) };
    let label = label.trim();
    if label.chars().count() > MAX_LABEL {
        return Err(Error::invalid("标签最多 64 个字符。"));
    }
    Ok((!label.is_empty()).then(|| label.to_owned()))
}

pub(crate) fn project(project: &ProjectId) -> Result<()> {
    if project.as_str().trim().is_empty() {
        return Err(Error::invalid("项目 id 不能为空。"));
    }
    Ok(())
}

pub(crate) fn mcp(mut input: McpServerInput) -> Result<McpServerInput> {
    let name = &input.name;
    if name.is_empty()
        || name.len() > MAX_MCP_NAME
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::invalid("MCP 服务器名称只能包含字母、数字、- 和 _，最多 32 个字符。"));
    }
    if let McpScope::Project { project: id } = &input.scope {
        project(id)?;
    }
    match &mut input.transport {
        McpTransport::Stdio { command, env, .. } => {
            *command = command.trim().to_owned();
            if command.is_empty() {
                return Err(Error::invalid("请填写 MCP 服务器的启动命令。"));
            }
            env_names(env)?;
        }
        McpTransport::Http { url, headers } => {
            *url = url.trim().to_owned();
            http_url(url, "MCP 地址")?;
            env_names(headers)?;
        }
    }
    input.description = input.description.map(|text| text.trim().to_owned()).filter(|text| !text.is_empty());
    Ok(input)
}

fn env_names(values: &[EnvValue]) -> Result<()> {
    if values.iter().any(|value| value.name.trim().is_empty()) {
        return Err(Error::invalid("环境变量和请求头的名称不能为空。"));
    }
    Ok(())
}

/// `http://` or `https://`, a non-empty host, and no user name or password.
fn http_url(url: &str, what: &str) -> Result<()> {
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://")) else {
        return Err(Error::invalid(format!("{what}需要以 http:// 或 https:// 开头。")));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(Error::invalid(format!("{what}不能包含用户名或密码。")));
    }
    if authority.is_empty() {
        return Err(Error::invalid(format!("{what}缺少主机名。")));
    }
    Ok(())
}

/// RFC 9110 `token`: one or more visible ASCII characters other than delimiters.
fn is_header_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
