//! Authenticated upstream forwarding and response streaming; no token or listener ownership.
use super::{Body, blocking, error_response};
use crate::{runtime, url};
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{self, HeaderMap, HeaderName, HeaderValue},
};
use std::{io, sync::Arc, time::Instant};
use thinkrail_core::{Protocol, Provider, ProviderService};
use tracing::{debug, warn};

/// Headers never forwarded in either direction (RFC 9110 §7.6.1).
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];
/// Request headers the gateway owns.
const MANAGED: [&str; 4] = ["authorization", "x-api-key", "host", "content-length"];
/// Upstream failures without URL or secrets.
fn describe(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "连接超时"
    } else if error.is_connect() {
        "无法连接到服务器"
    } else {
        "请求失败"
    }
}
pub(super) async fn forward(
    services: &Arc<dyn ProviderService>,
    provider: &Provider,
    endpoint: &str,
    protocol: Protocol,
    rest: &str,
    request: Request<Incoming>,
) -> Response<Body> {
    let started = Instant::now();
    // 3. Upstream address.
    let base = url::base_url(endpoint, protocol);
    let tail = match protocol {
        Protocol::OpenaiChat | Protocol::OpenaiResponses => {
            rest.strip_prefix("v1/").unwrap_or(rest)
        }
        Protocol::AnthropicMessages => rest,
    };
    let mut target = format!("{base}/{tail}");
    if let Some(query) = request.uri().query() {
        target.push('?');
        target.push_str(query);
    }
    // 4. Key: one rotation per request.
    let key = if provider.keys.is_empty() {
        None
    } else {
        let providers = Arc::clone(services);
        let id = provider.id.clone();
        match blocking(move || providers.next_key(&id)).await {
            Ok(Some(key)) => Some(key),
            Ok(None) => {
                warn!(provider_id = %provider.id, code = "INVALID", "routing.gateway.no_enabled_key");
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "提供方没有启用的 API 密钥，请在设置中启用一个",
                );
            }
            Err(error) => {
                warn!(code = %thinkrail_core::ErrorCode::code(&error), "routing.gateway.key_lookup_failed");
                return error_response(StatusCode::BAD_GATEWAY, "无法读取网关 API 密钥");
            }
        }
    };
    // 5. Headers.
    let (parts, body) = request.into_parts();
    let mut headers = HeaderMap::new();
    for (name, value) in &parts.headers {
        let lower = name.as_str();
        if !HOP_BY_HOP.contains(&lower) && !MANAGED.contains(&lower) {
            headers.append(name.clone(), value.clone());
        }
    }
    if let Some(key) = &key {
        let Ok(bearer) = HeaderValue::from_str(&format!("Bearer {}", key.value.expose())) else {
            warn!(provider_id = %provider.id, code = "INVALID", "routing.gateway.invalid_key_header");
            return error_response(StatusCode::BAD_GATEWAY, "API 密钥包含不能放进请求头的字符");
        };
        headers.insert(header::AUTHORIZATION, bearer);
        if protocol == Protocol::AnthropicMessages {
            let Ok(raw) = HeaderValue::from_str(key.value.expose()) else {
                warn!(provider_id = %provider.id, code = "INVALID", "routing.gateway.invalid_key_header");
                return error_response(StatusCode::BAD_GATEWAY, "API 密钥包含不能放进请求头的字符");
            };
            headers.insert("x-api-key", raw);
        }
    }
    for extra in &provider.input.headers {
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(extra.name.as_bytes()),
            HeaderValue::from_str(&extra.value),
        ) else {
            warn!(provider_id = %provider.id, code = "INVALID", "routing.gateway.invalid_provider_header");
            return error_response(StatusCode::BAD_GATEWAY, "提供方请求头无效");
        };
        headers.insert(name, value);
    }
    // 6. Body as is; response streamed back.
    let body = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            warn!(provider_id = %provider.id, code = "PROTOCOL", "routing.gateway.request_read_failed");
            return error_response(StatusCode::BAD_REQUEST, "无法读取请求体");
        }
    };
    let key_id = key.as_ref().map(|key| key.id.as_str());
    let Ok(runtime) = runtime::get() else {
        warn!(provider_id = %provider.id, code = "BACKEND_UNAVAILABLE", "routing.gateway.runtime_unavailable");
        return error_response(StatusCode::BAD_GATEWAY, "网络运行时不可用");
    };
    let upstream = runtime
        .client
        .request(parts.method, &target)
        .headers(headers)
        .body(body)
        .send()
        .await;
    let upstream = match upstream {
        Ok(upstream) => upstream,
        Err(error) => {
            let reason = describe(&error);
            warn!(provider_id = %provider.id, key_id, code = if error.is_timeout() { "TIMEOUT" } else { "EXTERNAL" }, "routing.gateway.upstream_failed");
            return error_response(StatusCode::BAD_GATEWAY, &format!("上游不可达：{reason}"));
        }
    };
    let status = upstream.status();
    if !status.is_success() {
        warn!(provider_id = %provider.id, key_id, status = status.as_u16(), code = "EXTERNAL", "routing.gateway.upstream_rejected");
    }
    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        if !HOP_BY_HOP.contains(&name.as_str()) && name != header::CONTENT_LENGTH {
            response_headers.append(name.clone(), value.clone());
        }
    }
    // Response headers are not a successful terminal state: the body is still streaming.
    debug!(provider_id = %provider.id, key_id, status = status.as_u16(),
        elapsed_ms = started.elapsed().as_millis() as u64, "routing.gateway.response_headers");
    let span = tracing::Span::current();
    let stream = upstream
        .bytes_stream()
        .map_ok(Frame::data)
        .map_err(move |_| {
            span.in_scope(|| {
                warn!(code = "EXTERNAL", "routing.gateway.response_read_failed");
                io::Error::other("网关上游响应读取失败")
            })
        });
    let mut response = Response::new(BodyExt::boxed(StreamBody::new(stream)));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    response
}
