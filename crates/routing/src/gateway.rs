//! Loopback gateway: engines send same-protocol requests with a per-chat token; the gateway swaps
//! in the real key, provider headers and upstream address and streams the response back
//! (agent-settings 04 §5.2).

use std::{
    collections::HashMap, convert::Infallible, io, net::SocketAddr, sync::Arc, time::Instant,
};

use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{self, HeaderMap, HeaderName, HeaderValue},
    service::service_fn,
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto,
};
use parking_lot::Mutex;
use thinkrail_core::{
    ChatId, Error, ErrorKind, Protocol, Provider, ProviderService, Result, Secret,
};
use tracing::{info, warn};

use crate::{
    router::{GatewayRoutes, protocol_suffix, short_id},
    runtime, url,
};

type Body = BoxBody<Bytes, io::Error>;

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

struct Shared {
    providers: Arc<dyn ProviderService>,
    /// token → chat. Tokens live only in memory.
    sessions: Mutex<HashMap<String, ChatId>>,
}

/// The host's loopback gateway; listens on `127.0.0.1:<random>` from the first `base_url` call.
pub struct Gateway {
    shared: Arc<Shared>,
    addr: Mutex<Option<SocketAddr>>,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway")
            .field("addr", &*self.addr.lock())
            .finish_non_exhaustive()
    }
}

impl Gateway {
    pub fn new(providers: Arc<dyn ProviderService>) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(Shared {
                providers,
                sessions: Mutex::new(HashMap::new()),
            }),
            addr: Mutex::new(None),
        })
    }

    /// The bound address, binding and starting the server on first call.
    fn addr(&self) -> Result<SocketAddr> {
        let mut slot = self.addr.lock();
        if let Some(addr) = *slot {
            return Ok(addr);
        }
        let runtime = runtime::get()?;
        let unavailable = |e: io::Error| {
            Error::new(
                ErrorKind::BackendUnavailable,
                format!("无法在 127.0.0.1 上启动网关：{e}"),
            )
        };
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(unavailable)?;
        std_listener.set_nonblocking(true).map_err(unavailable)?;
        let addr = std_listener.local_addr().map_err(unavailable)?;
        let listener = {
            let _guard = runtime.tokio.enter();
            tokio::net::TcpListener::from_std(std_listener).map_err(unavailable)?
        };
        let shared = Arc::clone(&self.shared);
        runtime.tokio.spawn(serve(listener, shared));
        info!(%addr, "网关已启动");
        *slot = Some(addr);
        Ok(addr)
    }
}

impl GatewayRoutes for Gateway {
    /// OpenAI-style: `http://127.0.0.1:<port>/p/<route>/v1`; Anthropic: `…/p/<route>` (the SDK
    /// appends `/v1/messages`). Engine request paths are therefore always `/p/<route>/v1/...`.
    fn base_url(&self, route: &str, protocol: Protocol) -> Result<String> {
        let addr = self.addr()?;
        Ok(match protocol {
            Protocol::OpenaiChat | Protocol::OpenaiResponses => {
                format!("http://{addr}/p/{route}/v1")
            }
            Protocol::AnthropicMessages => format!("http://{addr}/p/{route}"),
        })
    }

    /// 32 random bytes (two v4 uuids, hex); replaces the chat's previous token.
    fn register(&self, chat: &ChatId) -> Secret {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut sessions = self.shared.sessions.lock();
        sessions.retain(|_, owner| owner != chat);
        sessions.insert(token.clone(), chat.clone());
        Secret::new(token)
    }

    fn release(&self, chat: &ChatId) {
        self.shared.sessions.lock().retain(|_, owner| owner != chat);
    }
}

async fn serve(listener: tokio::net::TcpListener, shared: Arc<Shared>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                warn!(%error, "网关接受连接失败");
                continue;
            }
        };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let shared = Arc::clone(&shared);
                async move { Ok::<_, Infallible>(handle(&shared, request).await) }
            });
            if let Err(error) = auto::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await
            {
                info!(%error, "网关连接结束");
            }
        });
    }
}

fn full(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into())
        .map_err(|never| match never {})
        .boxed()
}

fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    let body = serde_json::json!({ "error": { "message": message } }).to_string();
    let mut response = Response::new(full(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

fn token(headers: &HeaderMap) -> Option<&str> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    bearer
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
        .map(str::trim)
}

fn protocol_of(suffix: &str) -> Option<Protocol> {
    [
        Protocol::OpenaiChat,
        Protocol::OpenaiResponses,
        Protocol::AnthropicMessages,
    ]
    .into_iter()
    .find(|p| protocol_suffix(*p) == suffix)
}

/// Paths an engine may reach per protocol (04 §5.2 step 2).
fn allowed(protocol: Protocol, rest: &str) -> bool {
    match protocol {
        Protocol::OpenaiChat => matches!(rest, "v1/chat/completions" | "v1/models"),
        Protocol::OpenaiResponses => matches!(rest, "v1/responses" | "v1/models"),
        Protocol::AnthropicMessages => matches!(
            rest,
            "v1/messages" | "v1/messages/count_tokens" | "v1/models"
        ),
    }
}

/// `thinkrail-<short id>-<suffix>` → (short id, protocol).
fn parse_route(route: &str) -> Option<(&str, Protocol)> {
    let (short, suffix) = route.strip_prefix("thinkrail-")?.rsplit_once('-')?;
    Some((short, protocol_of(suffix)?))
}

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

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| Error::external("网关后台任务异常结束"))?
}

async fn handle(shared: &Arc<Shared>, request: Request<Incoming>) -> Response<Body> {
    // 1. Token.
    let known = token(request.headers()).is_some_and(|t| shared.sessions.lock().contains_key(t));
    if !known {
        return error_response(StatusCode::UNAUTHORIZED, "invalid ThinkRail gateway token");
    }
    // 2. Route and path whitelist.
    let path = request.uri().path().to_owned();
    let Some((route, rest)) = path.strip_prefix("/p/").and_then(|p| p.split_once('/')) else {
        return error_response(StatusCode::NOT_FOUND, "unknown ThinkRail gateway path");
    };
    let route = route.to_owned();
    let Some((short, protocol)) = parse_route(&route) else {
        return error_response(StatusCode::NOT_FOUND, "unknown ThinkRail gateway route");
    };
    if !allowed(protocol, rest) {
        return error_response(
            StatusCode::NOT_FOUND,
            "path not forwarded by ThinkRail gateway",
        );
    }
    let short = short.to_owned();
    let lookup = Arc::clone(&shared.providers);
    let provider: Option<Provider> = match blocking(move || {
        Ok(lookup
            .providers()?
            .into_iter()
            .find(|p| p.input.enabled && short_id(p.id.as_str()) == short))
    })
    .await
    {
        Ok(found) => found,
        Err(error) => return error_response(StatusCode::BAD_GATEWAY, &error.to_string()),
    };
    let Some((provider, endpoint)) = provider.as_ref().and_then(|p| {
        p.input
            .endpoints
            .iter()
            .find(|e| e.protocol == protocol)
            .map(|e| (p, e))
    }) else {
        return error_response(StatusCode::NOT_FOUND, "unknown ThinkRail gateway route");
    };
    forward(
        shared,
        provider,
        &endpoint.base_url,
        protocol,
        &route,
        rest,
        request,
    )
    .await
}

async fn forward(
    shared: &Arc<Shared>,
    provider: &Provider,
    endpoint: &str,
    protocol: Protocol,
    route: &str,
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
        let providers = Arc::clone(&shared.providers);
        let id = provider.id.clone();
        match blocking(move || providers.next_key(&id)).await {
            Ok(Some(key)) => Some(key),
            Ok(None) => {
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    "提供方没有启用的 API 密钥，请在设置中启用一个",
                );
            }
            Err(error) => return error_response(StatusCode::BAD_GATEWAY, &error.to_string()),
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
            return error_response(StatusCode::BAD_GATEWAY, "API 密钥包含不能放进请求头的字符");
        };
        headers.insert(header::AUTHORIZATION, bearer);
        if protocol == Protocol::AnthropicMessages {
            let Ok(raw) = HeaderValue::from_str(key.value.expose()) else {
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
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("提供方请求头 {} 无效", extra.name),
            );
        };
        headers.insert(name, value);
    }
    // 6. Body as is; response streamed back.
    let body = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "无法读取请求体"),
    };
    let key_label = key
        .as_ref()
        .map(|k| k.label.clone().unwrap_or_else(|| k.id.to_string()));
    let Ok(runtime) = runtime::get() else {
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
            warn!(route, key = key_label.as_deref(), reason, "网关上游不可达");
            return error_response(StatusCode::BAD_GATEWAY, &format!("上游不可达：{reason}"));
        }
    };
    let status = upstream.status();
    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        if !HOP_BY_HOP.contains(&name.as_str()) && name != header::CONTENT_LENGTH {
            response_headers.append(name.clone(), value.clone());
        }
    }
    // 7. Log without values.
    info!(
        route,
        key = key_label.as_deref(),
        status = status.as_u16(),
        elapsed_ms = started.elapsed().as_millis() as u64
    );
    let stream = upstream
        .bytes_stream()
        .map_ok(Frame::data)
        .map_err(io::Error::other);
    let mut response = Response::new(BodyExt::boxed(StreamBody::new(stream)));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    response
}
