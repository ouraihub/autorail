//! Loopback gateway: engines send same-protocol requests with a per-chat token; the gateway swaps
//! in the real key, provider headers and upstream address and streams the response back
//! (agent-settings 04 §5.2).
mod forward;

use std::{collections::HashMap, convert::Infallible, io, net::SocketAddr, sync::Arc};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    Request, Response, StatusCode,
    body::Incoming,
    header::{self, HeaderMap, HeaderValue},
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
use tracing::{Instrument, info, warn};

use crate::{
    router::{GatewayRoutes, protocol_suffix, short_id},
    runtime,
};

type Body = BoxBody<Bytes, io::Error>;

struct Shared {
    providers: Arc<dyn ProviderService>,
    /// Token → chat and registration span. This is session/setup context, not the current turn;
    /// engines do not send a trusted per-turn trace identity. Tokens live only in memory.
    sessions: Mutex<HashMap<String, (ChatId, tracing::Span)>>,
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
        let unavailable =
            |_: io::Error| Error::new(ErrorKind::BackendUnavailable, "无法在本机启动网关");
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(unavailable)?;
        std_listener.set_nonblocking(true).map_err(unavailable)?;
        let addr = std_listener.local_addr().map_err(unavailable)?;
        let listener = {
            let _guard = runtime.tokio.enter();
            tokio::net::TcpListener::from_std(std_listener).map_err(unavailable)?
        };
        let shared = Arc::clone(&self.shared);
        runtime
            .tokio
            .spawn(serve(listener, shared).in_current_span());
        info!(port = addr.port(), "routing.gateway.started");
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
        sessions.retain(|_, (owner, _)| owner != chat);
        sessions.insert(token.clone(), (chat.clone(), tracing::Span::current()));
        Secret::new(token)
    }

    fn release(&self, chat: &ChatId) {
        self.shared
            .sessions
            .lock()
            .retain(|_, (owner, _)| owner != chat);
    }
}

async fn serve(listener: tokio::net::TcpListener, shared: Arc<Shared>) {
    let mut accept_failures = 0u64;
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => {
                if accept_failures > 0 {
                    warn!(
                        code = "IO",
                        count = accept_failures,
                        "routing.gateway.accept_recovered"
                    );
                    accept_failures = 0;
                }
                stream
            }
            Err(error) => {
                accept_failures = accept_failures.saturating_add(1);
                if accept_failures == 1 {
                    warn!(code = "IO", io_kind = ?error.kind(), "routing.gateway.accept_failed");
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let service = service_fn(move |request| {
                let shared = Arc::clone(&shared);
                let span = token(request.headers()).and_then(|token| shared.sessions.lock().get(token).map(|(_, span)| span.clone())).unwrap_or_else(tracing::Span::current);
                async move { Ok::<_, Infallible>(handle(&shared, request).instrument(span).await) }
            });
            if auto::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await
                .is_err()
            {
                warn!(code = "PROTOCOL", "routing.gateway.connection_failed");
            }
        }.in_current_span());
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

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || span.in_scope(work))
        .await
        .unwrap_or_else(|error| {
            if error.is_panic() {
                std::panic::resume_unwind(error.into_panic());
            }
            Err(Error::external("网关后台任务异常结束"))
        })
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
        Err(error) => {
            warn!(code = %thinkrail_core::ErrorCode::code(&error), "routing.gateway.provider_lookup_failed");
            return error_response(StatusCode::BAD_GATEWAY, "无法读取网关提供方配置");
        }
    };
    let Some((provider, endpoint)) = provider.as_ref().and_then(|p| {
        p.input
            .endpoints
            .iter()
            .find(|e| e.protocol == protocol)
            .map(|e| (p, e))
    }) else {
        warn!(code = "NOT_FOUND", "routing.gateway.route_unavailable");
        return error_response(StatusCode::NOT_FOUND, "unknown ThinkRail gateway route");
    };
    forward::forward(
        &shared.providers,
        provider,
        &endpoint.base_url,
        protocol,
        rest,
        request,
    )
    .await
}
