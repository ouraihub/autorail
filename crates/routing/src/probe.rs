//! `ProviderProbe`: model discovery and key checks straight against the upstream, never through
//! the gateway (agent-settings 04 §5.3).

use std::{sync::Arc, time::{Duration, Instant}};

use serde_json::Value;
use thinkrail_core::{
    DiscoveredModel, Error, KeyCheck, ProbeTarget, Protocol, ProviderId, ProviderProbe, ProviderService,
    Result,
};

use crate::{runtime, url};

const TIMEOUT: Duration = Duration::from_secs(15);
const ANTHROPIC_VERSION: &str = "2023-06-01";
const SNIPPET: usize = 200;

/// Probes provider endpoints with explicit keys; does not touch key rotation.
pub struct Probe {
    providers: Arc<dyn ProviderService>,
    timeout: Duration,
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe").field("timeout", &self.timeout).finish_non_exhaustive()
    }
}

/// Upstream reply: status and body text.
struct Reply {
    status: u16,
    body: String,
}

fn rejected() -> Error {
    Error::external("服务器拒绝了该 API 密钥。")
}

fn classify(error: &reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::timeout("服务器未及时响应。")
    } else if error.is_builder() {
        Error::invalid("API 地址无效。")
    } else {
        Error::external("无法连接到服务器。")
    }
}

fn snippet(body: &str, target: &ProbeTarget) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            let error = v.get("error")?;
            error.get("message").and_then(Value::as_str).or_else(|| error.as_str()).map(str::to_owned)
        })
        .unwrap_or_else(|| body.to_owned());
    let mut message = message;
    for secret in target.key.iter().map(|key| key.expose()).chain(target.headers.iter().map(|header| header.value.as_str())) {
        if !secret.is_empty() {
            message = message.replace(secret, "***");
        }
    }
    message.trim().chars().take(SNIPPET).collect()
}

fn status_error(reply: &Reply, target: &ProbeTarget) -> Error {
    if reply.status == 401 || reply.status == 403 {
        return rejected();
    }
    let detail = snippet(&reply.body, target);
    if detail.is_empty() {
        Error::external(format!("服务器返回 HTTP {}。", reply.status))
    } else {
        Error::external(format!("服务器返回 HTTP {}。{detail}", reply.status))
    }
}

fn parse_models(body: &str) -> Result<Vec<DiscoveredModel>> {
    let not_a_list = || Error::protocol("这不是兼容 OpenAI 或 Anthropic 的模型列表响应。");
    let value: Value = serde_json::from_str(body).map_err(|_| not_a_list())?;
    let data = value.get("data").and_then(Value::as_array).ok_or_else(not_a_list)?;
    let models = data
        .iter()
        .map(|item| {
            let id = item.get("id").and_then(Value::as_str).ok_or_else(not_a_list)?;
            Ok(DiscoveredModel {
                id: id.to_owned(),
                name: item.get("display_name").and_then(Value::as_str).map(str::to_owned),
                context_window: item.get("context_length").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if models.is_empty() {
        return Err(Error::external("服务器未返回任何模型。"));
    }
    Ok(models)
}

impl Probe {
    pub fn new(providers: Arc<dyn ProviderService>) -> Self {
        Self { providers, timeout: TIMEOUT }
    }

    #[cfg(test)]
    pub(crate) fn with_timeout(providers: Arc<dyn ProviderService>, timeout: Duration) -> Self {
        Self { providers, timeout }
    }

    fn send(&self, target: &ProbeTarget, path: &str, body: Option<Value>) -> Result<Reply> {
        let runtime = runtime::get()?;
        let protocol = target.endpoint.protocol;
        let address = format!("{}{path}", url::base_url(&target.endpoint.base_url, protocol));
        let client = &runtime.client;
        let mut request = match &body {
            Some(body) => client.post(&address).json(body),
            None => client.get(&address),
        };
        request = request.timeout(self.timeout);
        if let Some(key) = &target.key {
            request = request.bearer_auth(key.expose());
            if protocol == Protocol::AnthropicMessages {
                request = request.header("x-api-key", key.expose());
            }
        }
        if protocol == Protocol::AnthropicMessages {
            request = request.header("anthropic-version", ANTHROPIC_VERSION);
        }
        for header in &target.headers {
            request = request.header(header.name.as_str(), header.value.as_str());
        }
        runtime.tokio.block_on(async {
            let response = request.send().await.map_err(|e| classify(&e))?;
            let status = response.status().as_u16();
            let body = response.text().await.map_err(|e| classify(&e))?;
            Ok(Reply { status, body })
        })
    }
}

impl ProviderProbe for Probe {
    fn list_models(&self, target: &ProbeTarget) -> Result<Vec<DiscoveredModel>> {
        let path = match target.endpoint.protocol {
            Protocol::OpenaiChat | Protocol::OpenaiResponses => "/models",
            Protocol::AnthropicMessages => "/v1/models",
        };
        let reply = self.send(target, path, None)?;
        if !(200..300).contains(&reply.status) {
            return Err(status_error(&reply, target));
        }
        parse_models(&reply.body)
    }

    fn check(&self, target: &ProbeTarget, model_id: &str) -> Result<u64> {
        let (path, body) = match target.endpoint.protocol {
            Protocol::OpenaiChat => (
                "/chat/completions",
                serde_json::json!({"model": model_id, "messages": [{"role": "user", "content": "hi"}], "max_tokens": 1}),
            ),
            Protocol::OpenaiResponses => {
                ("/responses", serde_json::json!({"model": model_id, "input": "hi", "max_output_tokens": 16}))
            }
            Protocol::AnthropicMessages => (
                "/v1/messages",
                serde_json::json!({"model": model_id, "max_tokens": 1, "messages": [{"role": "user", "content": "hi"}]}),
            ),
        };
        let started = Instant::now();
        let reply = self.send(target, path, Some(body))?;
        if !(200..300).contains(&reply.status) {
            return Err(status_error(&reply, target));
        }
        Ok(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    /// Serial, in key order; each key is read with `reveal_key`, so rotation is untouched.
    fn check_keys(&self, provider: &ProviderId, model_id: &str) -> Result<Vec<KeyCheck>> {
        let provider = self.providers.provider(provider)?;
        let model = provider
            .input
            .models
            .iter()
            .find(|m| m.model_id == model_id)
            .ok_or_else(|| Error::not_found(format!("模型 {model_id}")))?;
        let protocol = model.protocols.first().copied().ok_or_else(|| {
            Error::invalid(format!("模型 {model_id} 的端点类型没有对应的 API 地址。"))
        })?;
        let endpoint = provider
            .input
            .endpoints
            .iter()
            .find(|e| e.protocol == protocol)
            .ok_or_else(|| Error::invalid(format!("模型 {model_id} 的端点类型没有对应的 API 地址。")))?;
        let mut out = Vec::new();
        for summary in provider.keys.iter().filter(|k| k.enabled) {
            let target = ProbeTarget {
                endpoint: endpoint.clone(),
                headers: provider.input.headers.clone(),
                key: Some(self.providers.reveal_key(&summary.id)?),
            };
            let started = Instant::now();
            let result = self.check(&target, model_id);
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            out.push(match result {
                Ok(elapsed_ms) => KeyCheck { key: summary.id.clone(), ok: true, elapsed_ms, error: None },
                Err(error) => KeyCheck { key: summary.id.clone(), ok: false, elapsed_ms, error: Some(error.to_string()) },
            });
        }
        Ok(out)
    }
}
