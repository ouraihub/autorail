use std::{
    io,
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::{Request, Response, body::{Frame, Incoming}, service::service_fn};
use hyper_util::{rt::{TokioExecutor, TokioIo}, server::conn::auto};
use parking_lot::Mutex;
use tempfile::TempDir;
use thinkrail_core::{
    Access, BackendKind, ChatId, Endpoint, EngineRouting, EnvValue, ErrorKind, Header, McpScope, McpServerInput,
    McpService, McpTransport, ModelPrefService, ModelRef, ModelRouter, ProbeTarget, ProjectId, ProjectedMcpTransport,
    Protocol, Provider, ProviderInput, ProviderKeyInput, ProviderKeyUpdate, ProviderModel, ProviderProbe, ModelType,
    ProviderService, Result, RouteRequest, Secret, ThinkingLevel,
};
use thinkrail_settings_store::SqliteSettingsStore;

use crate::{Gateway, GatewayRoutes, Probe, Router, preview, runtime, url::base_url};

const KEYS: [&str; 3] = ["sk-test-a", "sk-test-b", "sk-test-c"];
const HEADER_VALUE: &str = "trace-secret-value";

// ---------- settings fixtures ----------

struct Fixture {
    _dir: TempDir,
    store: Arc<SqliteSettingsStore>,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(SqliteSettingsStore::open(&dir.path().join("thinkrail.db")).unwrap());
    Fixture { _dir: dir, store }
}

fn model(id: &str, protocols: &[Protocol]) -> ProviderModel {
    ProviderModel {
        model_id: id.into(),
        model_type: ModelType::Chat,
        name: id.to_uppercase(),
        protocols: protocols.to_vec(),
        context_window: None,
        max_tokens: Some(8_000),
        vision: true,
        reasoning: true,
        tools: true,
        efforts: vec![ThinkingLevel::Low, ThinkingLevel::High],
        enabled: true,
    }
}

fn endpoints(openai: &str, anthropic: &str) -> Vec<Endpoint> {
    vec![
        Endpoint { protocol: Protocol::OpenaiChat, base_url: openai.into() },
        Endpoint { protocol: Protocol::AnthropicMessages, base_url: anthropic.into() },
    ]
}

fn input(name: &str, access: Access) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        endpoints: endpoints("https://relay.example/v1", "https://relay.example/v1"),
        default_protocol: Protocol::OpenaiChat,
        headers: vec![Header { name: "X-Trace".into(), value: HEADER_VALUE.into() }],
        access,
        enabled: true,
        models: vec![model("gpt-test", &[Protocol::OpenaiChat]), model("claude-test", &[Protocol::AnthropicMessages])],
    }
}

fn create(fx: &Fixture, input: ProviderInput, keys: &[&str]) -> Provider {
    let mut provider = ProviderService::create(&*fx.store, input).unwrap();
    for key in keys {
        let key = ProviderKeyInput { label: None, key: Secret::new(*key), enabled: true };
        provider = fx.store.add_key(&provider.id, key).unwrap();
    }
    provider
}

fn set_key_enabled(fx: &Fixture, provider: &Provider, index: usize, enabled: bool) -> Provider {
    let update = ProviderKeyUpdate { label: None, key: None, enabled };
    fx.store.update_key(&provider.id, &provider.keys[index].id, update).unwrap()
}

fn short(provider: &Provider) -> String {
    provider.id.as_str().chars().take(8).collect()
}

#[track_caller]
fn kind<T: std::fmt::Debug>(result: Result<T>, expected: ErrorKind) -> String {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), Some(expected), "{error}");
    error.to_string()
}

// ---------- gateway test double ----------

#[derive(Debug, Default)]
struct FakeGateway {
    base_urls: Mutex<Vec<(String, Protocol)>>,
    registered: Mutex<Vec<ChatId>>,
    released: Mutex<Vec<ChatId>>,
}

impl GatewayRoutes for FakeGateway {
    fn base_url(&self, route: &str, protocol: Protocol) -> Result<String> {
        self.base_urls.lock().push((route.to_owned(), protocol));
        Ok(format!("http://gateway.test/p/{route}"))
    }
    fn register(&self, chat: &ChatId) -> Secret {
        let mut registered = self.registered.lock();
        registered.push(chat.clone());
        Secret::new(format!("gw-token-{}", registered.len()))
    }
    fn release(&self, chat: &ChatId) {
        self.released.lock().push(chat.clone());
    }
}

fn router(fx: &Fixture, gateway: Arc<dyn GatewayRoutes>) -> Router {
    Router::new(fx.store.clone(), fx.store.clone(), fx.store.clone(), gateway)
}

fn request(backend: BackendKind, new_session: bool) -> RouteRequest {
    RouteRequest { chat: ChatId::from("chat-1"), project: ProjectId::from("project-1"), backend, new_session }
}

fn env<'a>(routing: &'a EngineRouting, var: &str) -> Option<&'a str> {
    routing.env.iter().find(|(name, _)| name == var).map(|(_, value)| value.expose())
}

/// Everything except `env`, serialized: must never carry a secret.
#[track_caller]
fn assert_no_secret_outside_env(routing: &EngineRouting) {
    let public = serde_json::json!({
        "providers": routing.providers,
        "defaultModel": routing.default_model,
        "mcp": routing.mcp,
        "signature": routing.signature,
    })
    .to_string();
    for secret in KEYS.iter().copied().chain([HEADER_VALUE, "mcp-secret-value"]) {
        assert!(!public.contains(secret), "{secret} leaked outside env");
    }
    assert!(!format!("{routing:?}").contains("sk-test"));
}

// ---------- T3: url ----------

#[test]
fn base_url_normalization_matrix() {
    use Protocol::{AnthropicMessages as Am, OpenaiChat as Oc, OpenaiResponses as Or};
    let cases: [(&str, &str, &str); 7] = [
        ("https://a.com", "https://a.com/v1", "https://a.com"),
        ("https://a.com/", "https://a.com/v1", "https://a.com"),
        ("https://a.com/v1", "https://a.com/v1", "https://a.com"),
        ("https://a.com/v1/", "https://a.com/v1", "https://a.com"),
        ("https://a.com/api/v3", "https://a.com/api/v3", "https://a.com/api"),
        ("https://a.com/v1#", "https://a.com/v1", "https://a.com/v1"),
        ("https://a.com/anthropic", "https://a.com/anthropic/v1", "https://a.com/anthropic"),
    ];
    for (raw, openai, anthropic) in cases {
        assert_eq!(base_url(raw, Oc), openai, "{raw} openai-chat");
        assert_eq!(base_url(raw, Or), openai, "{raw} openai-responses");
        assert_eq!(base_url(raw, Am), anthropic, "{raw} anthropic");
    }
    assert_eq!(base_url("  https://a.com/v1beta/ ", Oc), "https://a.com/v1beta");
    assert_eq!(base_url("https://a.com/v2alpha", Am), "https://a.com");
}

#[test]
fn preview_shows_the_full_request_address() {
    assert_eq!(preview("https://a.com", Protocol::OpenaiChat), "https://a.com/v1/chat/completions");
    assert_eq!(preview("https://a.com/v1/", Protocol::OpenaiResponses), "https://a.com/v1/responses");
    assert_eq!(preview("https://a.com/v1", Protocol::AnthropicMessages), "https://a.com/v1/messages");
    assert_eq!(preview("https://a.com/custom/chat#", Protocol::OpenaiChat), "https://a.com/custom/chat/chat/completions");
    assert_eq!(preview("https://a.com/anthropic#", Protocol::AnthropicMessages), "https://a.com/anthropic/v1/messages");
}

// ---------- T3: prepare ----------

#[test]
fn empty_store_gives_empty_routing() {
    let fx = fixture();
    let gateway = Arc::new(FakeGateway::default());
    let routing = router(&fx, gateway.clone()).prepare(&request(BackendKind::Pi, true)).unwrap();
    assert!(routing.is_empty());
    assert!(!routing.signature.is_empty());
    assert!(gateway.registered.lock().is_empty());
}

#[test]
fn one_provider_with_two_protocols_projects_two_routes() {
    let fx = fixture();
    let provider = create(&fx, input("Relay", Access::Direct), &KEYS[..1]);
    let short = short(&provider);
    let upper = short.to_uppercase();
    let routing = router(&fx, Arc::new(FakeGateway::default())).prepare(&request(BackendKind::Pi, true)).unwrap();

    assert_eq!(routing.providers.len(), 2);
    let oc = &routing.providers[0];
    assert_eq!(oc.route, format!("thinkrail-{short}-oc"));
    assert_eq!(oc.protocol, Protocol::OpenaiChat);
    assert_eq!(oc.base_url, "https://relay.example/v1");
    assert_eq!(oc.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["gpt-test"]);
    assert_eq!((oc.models[0].context_window, oc.models[0].max_tokens), (128_000, 8_000));
    let am = &routing.providers[1];
    assert_eq!(am.route, format!("thinkrail-{short}-am"));
    assert_eq!(am.base_url, "https://relay.example");
    assert_eq!(am.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["claude-test"]);

    let key_var = format!("THINKRAIL_PROVIDER_{upper}_KEY");
    assert!(routing.providers.iter().all(|p| p.key_var == key_var && p.display_name == "Relay"));
    assert_eq!(oc.headers[0].name, "X-Trace");
    assert_eq!(oc.headers[0].var, format!("THINKRAIL_HDR_{upper}_0"));
    assert_eq!(env(&routing, &key_var), Some(KEYS[0]));
    assert_eq!(env(&routing, &format!("THINKRAIL_HDR_{upper}_0")), Some(HEADER_VALUE));
    assert_eq!(routing.env.len(), 2);
    assert_no_secret_outside_env(&routing);
}

#[test]
fn direct_rotation_keeps_the_signature_until_keys_change() {
    let fx = fixture();
    let provider = create(&fx, input("Relay", Access::Direct), &KEYS[..2]);
    let key_var = format!("THINKRAIL_PROVIDER_{}_KEY", short(&provider).to_uppercase());
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let req = request(BackendKind::Omp, true);

    let first = router.prepare(&req).unwrap();
    let second = router.prepare(&req).unwrap();
    assert_eq!(env(&first, &key_var), Some(KEYS[0]));
    assert_eq!(env(&second, &key_var), Some(KEYS[1]));
    let signature = router.signature(&req).unwrap();
    assert_eq!(router.signature(&req).unwrap(), signature);
    assert_eq!(first.signature, signature);
    assert_eq!(second.signature, signature);
    // `signature` is read-only: rotation continues where `prepare` left it.
    assert_eq!(env(&router.prepare(&req).unwrap(), &key_var), Some(KEYS[0]));

    set_key_enabled(&fx, &provider, 1, false);
    assert_ne!(router.signature(&req).unwrap(), signature);
    assert_no_secret_outside_env(&first);
}

#[test]
fn editing_a_direct_key_invalidates_the_launch_without_rotating_it() {
    let fx = fixture();
    let provider = create(&fx, input("Relay", Access::Direct), &KEYS[..1]);
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let req = request(BackendKind::Pi, false);
    let before = router.prepare(&req).unwrap().signature;
    fx.store.update_key(&provider.id, &provider.keys[0].id, ProviderKeyUpdate {
        label: None, key: Some(Secret::new(KEYS[1])), enabled: true,
    }).unwrap();
    assert_ne!(router.signature(&req).unwrap(), before);
    let routing = router.prepare(&req).unwrap();
    let key_var = format!("THINKRAIL_PROVIDER_{}_KEY", short(&provider).to_uppercase());
    assert_eq!(env(&routing, &key_var), Some(KEYS[1]));
}

#[test]
fn all_keys_disabled_is_invalid_and_no_keys_uses_the_placeholder() {
    let fx = fixture();
    let local = create(&fx, input("Local", Access::Direct), &[]);
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let req = request(BackendKind::Pi, true);
    let routing = router.prepare(&req).unwrap();
    let key_var = format!("THINKRAIL_PROVIDER_{}_KEY", short(&local).to_uppercase());
    assert_eq!(env(&routing, &key_var), Some("thinkrail-no-key"));

    let relay = create(&fx, input("Relay", Access::Direct), &KEYS[..1]);
    set_key_enabled(&fx, &relay, 0, false);
    let message = kind(router.prepare(&req), ErrorKind::Invalid);
    assert!(message.contains("提供方 Relay 没有启用的 API 密钥"), "{message}");
    kind(router.signature(&req), ErrorKind::Invalid);
}

#[test]
fn gateway_mode_uses_gateway_addresses_and_tokens() {
    let fx = fixture();
    let provider = create(&fx, input("Relay", Access::Gateway), &KEYS[..2]);
    let short = short(&provider);
    let gateway = Arc::new(FakeGateway::default());
    let router = router(&fx, gateway.clone());
    let req = request(BackendKind::Pi, true);
    let routing = router.prepare(&req).unwrap();

    assert_eq!(routing.providers[0].base_url, format!("http://gateway.test/p/thinkrail-{short}-oc"));
    assert_eq!(routing.providers[1].base_url, format!("http://gateway.test/p/thinkrail-{short}-am"));
    assert!(routing.providers.iter().all(|p| p.headers.is_empty()));
    assert_eq!(
        *gateway.base_urls.lock(),
        [(format!("thinkrail-{short}-oc"), Protocol::OpenaiChat), (format!("thinkrail-{short}-am"), Protocol::AnthropicMessages)]
    );
    // One token per chat, shared by all routes; the real key never enters the environment.
    assert_eq!(*gateway.registered.lock(), std::slice::from_ref(&req.chat));
    let key_var = format!("THINKRAIL_PROVIDER_{}_KEY", short.to_uppercase());
    assert_eq!(env(&routing, &key_var), Some("gw-token-1"));
    assert_eq!(routing.env.len(), 1);
    assert!(routing.env.iter().all(|(_, v)| !KEYS.contains(&v.expose())));
    assert_no_secret_outside_env(&routing);

    // Signature never registers a token.
    assert_eq!(router.signature(&req).unwrap(), routing.signature);
    assert_eq!(gateway.registered.lock().len(), 1);
    router.release(&req.chat);
    assert_eq!(*gateway.released.lock(), [req.chat]);
}

#[test]
fn dsh_with_extra_headers_switches_a_direct_provider_to_the_gateway() {
    let fx = fixture();
    let provider = create(&fx, input("Relay", Access::Direct), &KEYS[..1]);
    let mut plain = input("Plain", Access::Direct);
    plain.headers.clear();
    create(&fx, plain, &KEYS[1..2]);
    let gateway = Arc::new(FakeGateway::default());
    let router = router(&fx, gateway.clone());

    let routing = router.prepare(&request(BackendKind::Dsh, true)).unwrap();
    let short = short(&provider);
    let relay: Vec<_> = routing.providers.iter().filter(|p| p.route.contains(&short)).collect();
    assert!(relay.iter().all(|p| p.base_url.starts_with("http://gateway.test/") && p.headers.is_empty()));
    let others: Vec<_> = routing.providers.iter().filter(|p| !p.route.contains(&short)).collect();
    assert!(others.iter().all(|p| p.base_url.starts_with("https://relay.example")));
    assert_eq!(env(&routing, &format!("THINKRAIL_PROVIDER_{}_KEY", short.to_uppercase())), Some("gw-token-1"));
    assert_no_secret_outside_env(&routing);

    // pi keeps the same provider direct.
    let pi = router.prepare(&request(BackendKind::Pi, true)).unwrap();
    assert!(pi.providers.iter().all(|p| !p.base_url.starts_with("http://gateway.test/")));
}

#[test]
fn default_model_only_for_new_sessions_with_clamped_effort() {
    let fx = fixture();
    let mut relay = input("Relay", Access::Direct);
    relay.models.push(ProviderModel { reasoning: false, efforts: vec![], ..model("plain", &[Protocol::OpenaiChat]) });
    relay.models.push(model("both", &[Protocol::AnthropicMessages, Protocol::OpenaiChat]));
    let provider = create(&fx, relay, &KEYS[..1]);
    let short = short(&provider);
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let pick = |id: &str, effort| {
        let model = ModelRef { provider: provider.id.clone(), model_id: id.into() };
        fx.store.set_default(Some(model), Some(effort)).unwrap();
    };

    pick("both", ThinkingLevel::Max);
    let routing = router.prepare(&request(BackendKind::Pi, true)).unwrap();
    let default = routing.default_model.unwrap();
    assert_eq!(default.route, format!("thinkrail-{short}-am"));
    assert_eq!((default.model_id.as_str(), default.effort), ("both", Some(ThinkingLevel::High)));
    assert!(router.prepare(&request(BackendKind::Pi, false)).unwrap().default_model.is_none());

    pick("both", ThinkingLevel::Medium);
    let effort = router.prepare(&request(BackendKind::Pi, true)).unwrap().default_model.unwrap().effort;
    assert_eq!(effort, Some(ThinkingLevel::Low));

    pick("plain", ThinkingLevel::High);
    let default = router.prepare(&request(BackendKind::Pi, true)).unwrap().default_model.unwrap();
    assert_eq!((default.route, default.effort), (format!("thinkrail-{short}-oc"), None));

    // The model is deleted: the preference stays, the launch has no default.
    let current = fx.store.provider(&provider.id).unwrap();
    let mut edited = current.input.clone();
    edited.models.retain(|m| m.model_id != "plain");
    ProviderService::update(&*fx.store, &provider.id, current.revision, edited).unwrap();
    assert!(router.prepare(&request(BackendKind::Pi, true)).unwrap().default_model.is_none());
}

fn mcp_input(name: &str, transport: McpTransport) -> McpServerInput {
    McpServerInput { name: name.into(), scope: McpScope::Global, transport, enabled: true, description: None }
}

fn value(name: &str, value: &str) -> EnvValue {
    EnvValue { name: name.into(), value: value.into() }
}

#[test]
fn mcp_values_resolve_into_env_under_id_derived_names() {
    let fx = fixture();
    let docs = McpService::create(
        &*fx.store,
        mcp_input(
            "docs",
            McpTransport::Stdio {
                command: "docs-mcp".into(),
                args: vec!["--stdio".into()],
                env: vec![value("TOKEN", "mcp-secret-value"), value("SEARCH", "$PATH"), value("ALSO", "${PATH}")],
            },
        ),
    )
    .unwrap();
    let web = McpService::create(
        &*fx.store,
        mcp_input("web", McpTransport::Http { url: "https://mcp.example/".into(), headers: vec![value("X-Key", "abc")] }),
    )
    .unwrap();
    let router = router(&fx, Arc::new(FakeGateway::default()));
    let routing = router.prepare(&request(BackendKind::Omp, true)).unwrap();

    let docs_short = docs.id.as_str().chars().take(8).collect::<String>().to_uppercase();
    let web_short = web.id.as_str().chars().take(8).collect::<String>().to_uppercase();
    assert_eq!(routing.mcp.len(), 2);
    assert_eq!(routing.mcp[0].name, "thinkrail-docs");
    let ProjectedMcpTransport::Stdio { command, args, env: bindings } = &routing.mcp[0].transport else { panic!() };
    assert_eq!((command.as_str(), args.as_slice()), ("docs-mcp", ["--stdio".to_owned()].as_slice()));
    let vars: Vec<_> = bindings.iter().map(|b| (b.name.as_str(), b.var.clone())).collect();
    assert_eq!(
        vars,
        [
            ("TOKEN", format!("THINKRAIL_MCP_{docs_short}_E0")),
            ("SEARCH", format!("THINKRAIL_MCP_{docs_short}_E1")),
            ("ALSO", format!("THINKRAIL_MCP_{docs_short}_E2")),
        ]
    );
    let path = std::env::var("PATH").unwrap();
    assert_eq!(env(&routing, &format!("THINKRAIL_MCP_{docs_short}_E0")), Some("mcp-secret-value"));
    assert_eq!(env(&routing, &format!("THINKRAIL_MCP_{docs_short}_E1")), Some(path.as_str()));
    assert_eq!(env(&routing, &format!("THINKRAIL_MCP_{docs_short}_E2")), Some(path.as_str()));
    let ProjectedMcpTransport::Http { headers, .. } = &routing.mcp[1].transport else { panic!() };
    assert_eq!(headers[0].var, format!("THINKRAIL_MCP_{web_short}_H0"));
    // Only referenced values: three stdio env values + one header, nothing else.
    assert_eq!(routing.env.len(), 4);
    assert!(!routing.env.iter().any(|(name, _)| name == "PATH"));
    assert_no_secret_outside_env(&routing);

    // A changed MCP value changes the signature.
    let before = routing.signature;
    let edited = mcp_input("web", McpTransport::Http { url: "https://mcp.example/".into(), headers: vec![value("X-Key", "abd")] });
    McpService::update(&*fx.store, &web.id, web.revision, edited).unwrap();
    assert_ne!(router.signature(&request(BackendKind::Omp, true)).unwrap(), before);
}

#[test]
fn missing_mcp_variable_is_invalid() {
    let fx = fixture();
    let transport = McpTransport::Stdio {
        command: "x".into(),
        args: vec![],
        env: vec![value("TOKEN", "${THINKRAIL_TEST_SURELY_UNSET_VARIABLE}")],
    };
    McpService::create(&*fx.store, mcp_input("broken", transport)).unwrap();
    let message = kind(router(&fx, Arc::new(FakeGateway::default())).prepare(&request(BackendKind::Pi, true)), ErrorKind::Invalid);
    assert!(message.contains("MCP 服务器 broken 引用的环境变量 THINKRAIL_TEST_SURELY_UNSET_VARIABLE 未设置"), "{message}");
}

#[test]
fn catalog_hides_hidden_models_and_lists_each_model_once() {
    let fx = fixture();
    let mut relay = input("Relay", Access::Direct);
    relay.models.push(model("both", &[Protocol::AnthropicMessages, Protocol::OpenaiChat]));
    relay.models.push(ProviderModel { enabled: false, ..model("off", &[Protocol::OpenaiChat]) });
    relay.models.push(ProviderModel { model_type: ModelType::Image, ..model("painter", &[Protocol::OpenaiChat]) });
    let provider = create(&fx, relay, &[]);
    let mut disabled = input("Disabled", Access::Direct);
    disabled.enabled = false;
    create(&fx, disabled, &[]);
    fx.store.set_hidden(&ModelRef { provider: provider.id.clone(), model_id: "gpt-test".into() }, true).unwrap();

    let catalog = router(&fx, Arc::new(FakeGateway::default())).catalog(BackendKind::Dsh).unwrap();
    let short = short(&provider);
    let listed: Vec<_> = catalog.iter().map(|c| (c.model.id.as_str(), c.route.clone())).collect();
    assert_eq!(listed, [("claude-test", format!("thinkrail-{short}-am")), ("both", format!("thinkrail-{short}-am"))], "disabled and non-chat models stay out");
    assert!(catalog.iter().all(|c| c.provider == provider.id && c.provider_name == "Relay"));
}

// ---------- loopback fake upstream ----------

#[derive(Debug, Clone)]
struct Seen {
    path: String,
    headers: hyper::HeaderMap,
}

#[derive(Debug, Clone)]
enum Reply {
    Json(u16, &'static str),
    /// 200 with one body frame per chunk, 50 ms apart.
    Chunks(Vec<&'static str>),
    /// 200 `{}` after the delay.
    Delay(Duration),
}

type TestBody = BoxBody<Bytes, io::Error>;

#[derive(Debug)]
struct Upstream {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Upstream {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().clone()
    }
}

async fn answer(reply: Reply) -> Response<TestBody> {
    let full = |status: u16, body: &'static str| {
        let mut response = Response::new(Full::new(Bytes::from_static(body.as_bytes())).map_err(|never| match never {}).boxed());
        *response.status_mut() = hyper::StatusCode::from_u16(status).unwrap();
        response.headers_mut().insert("content-type", "application/json".parse().unwrap());
        response
    };
    match reply {
        Reply::Json(status, body) => full(status, body),
        Reply::Delay(delay) => {
            tokio::time::sleep(delay).await;
            full(200, "{}")
        }
        Reply::Chunks(chunks) => {
            let stream = futures_util::stream::unfold(chunks.into_iter(), |mut rest| async move {
                let chunk = rest.next()?;
                tokio::time::sleep(Duration::from_millis(50)).await;
                Some((Ok::<_, io::Error>(Frame::data(Bytes::from_static(chunk.as_bytes()))), rest))
            });
            Response::new(StreamBody::new(stream).boxed())
        }
    }
}

fn upstream(reply: Reply) -> Upstream {
    let rt = runtime::get().unwrap();
    let listener = rt.tokio.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    rt.tokio.spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let record = record.clone();
            let reply = reply.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let record = record.clone();
                    let reply = reply.clone();
                    async move {
                        let path = request.uri().path_and_query().map(|p| p.to_string()).unwrap_or_default();
                        record.lock().push(Seen { path, headers: request.headers().clone() });
                        Ok::<_, std::convert::Infallible>(answer(reply).await)
                    }
                });
                let _ = auto::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(stream), service).await;
            });
        }
    });
    Upstream { addr, seen }
}

/// An address nothing listens on.
fn closed_port() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

/// (status, body chunks) for a POST through the shared client.
fn post(url: &str, headers: &[(&str, &str)]) -> (u16, Vec<Bytes>) {
    let rt = runtime::get().unwrap();
    rt.tokio.block_on(async {
        let mut request = rt.client.post(url).body("{\"model\":\"m\"}");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let mut chunks = Vec::new();
        while let Some(chunk) = response.chunk().await.unwrap() {
            chunks.push(chunk);
        }
        (status, chunks)
    })
}

fn joined(chunks: &[Bytes]) -> String {
    chunks.iter().map(|c| String::from_utf8_lossy(c)).collect()
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).and_then(|v| v.to_str().ok())
}

// ---------- T4: gateway ----------

fn gateway_provider(fx: &Fixture, openai: &str, anthropic: &str, keys: &[&str]) -> Provider {
    let mut relay = input("Relay", Access::Gateway);
    relay.endpoints = endpoints(openai, anthropic);
    create(fx, relay, keys)
}

fn chat() -> ChatId {
    ChatId::from("chat-gw")
}

#[test]
fn gateway_rejects_missing_wrong_and_released_tokens() {
    let fx = fixture();
    let up = upstream(Reply::Json(200, "{}"));
    let provider = gateway_provider(&fx, &up.url(""), &up.url(""), &KEYS[..1]);
    let gateway = Gateway::new(fx.store.clone());
    let url = format!("{}/chat/completions", gateway.base_url(&format!("thinkrail-{}-oc", short(&provider)), Protocol::OpenaiChat).unwrap());
    let token = gateway.register(&chat());

    let (status, body) = post(&url, &[]);
    assert_eq!(status, 401);
    assert!(joined(&body).contains("invalid ThinkRail gateway token"));
    assert_eq!(post(&url, &[("authorization", "Bearer nope")]).0, 401);
    assert_eq!(post(&url, &[("x-api-key", token.expose())]).0, 200);
    assert_eq!(post(&url, &[("authorization", &format!("Bearer {}", token.expose()))]).0, 200);

    // A new registration replaces the old token; release revokes it.
    let renewed = gateway.register(&chat());
    assert_eq!(post(&url, &[("x-api-key", token.expose())]).0, 401);
    gateway.release(&chat());
    assert_eq!(post(&url, &[("x-api-key", renewed.expose())]).0, 401);
    assert_eq!(up.seen().len(), 2);
}

#[test]
fn gateway_404s_outside_the_whitelist_and_for_unknown_routes() {
    let fx = fixture();
    let up = upstream(Reply::Json(200, "{}"));
    let provider = gateway_provider(&fx, &up.url(""), &up.url(""), &KEYS[..1]);
    let gateway = Gateway::new(fx.store.clone());
    let short = short(&provider);
    let oc = gateway.base_url(&format!("thinkrail-{short}-oc"), Protocol::OpenaiChat).unwrap();
    let am = gateway.base_url(&format!("thinkrail-{short}-am"), Protocol::AnthropicMessages).unwrap();
    let or = gateway.base_url(&format!("thinkrail-{short}-or"), Protocol::OpenaiResponses).unwrap();
    let token = gateway.register(&chat());
    let auth = [("x-api-key", token.expose())];

    for url in [
        format!("{oc}/messages"),
        format!("{oc}/embeddings"),
        format!("{am}/v1/chat/completions"),
        format!("{am}/v2/messages"),
        // No Responses endpoint is configured for the provider.
        format!("{or}/responses"),
        oc.replace(&short, "00000000") + "/chat/completions",
        oc.replace("-oc", "-xx") + "/chat/completions",
        oc.replace("/p/", "/q/") + "/chat/completions",
    ] {
        assert_eq!(post(&url, &auth).0, 404, "{url}");
    }
    assert!(up.seen().is_empty());
}

#[test]
fn gateway_forwards_paths_keys_and_headers() {
    let fx = fixture();
    let up = upstream(Reply::Json(200, "{}"));
    // OpenAI address without a version, Anthropic address with one: both normalize.
    let provider = gateway_provider(&fx, &up.url("/relay"), &up.url("/claude/v1"), &KEYS[..1]);
    let gateway = Gateway::new(fx.store.clone());
    let sid = short(&provider);
    let oc = gateway.base_url(&format!("thinkrail-{sid}-oc"), Protocol::OpenaiChat).unwrap();
    let am = gateway.base_url(&format!("thinkrail-{sid}-am"), Protocol::AnthropicMessages).unwrap();
    assert!(oc.ends_with("/v1") && !am.ends_with("/v1"));
    let token = gateway.register(&chat());
    let bearer = format!("Bearer {}", token.expose());

    assert_eq!(post(&format!("{oc}/chat/completions?stream=true"), &[("authorization", &bearer)]).0, 200);
    let auth = [("x-api-key", token.expose()), ("anthropic-version", "2023-06-01"), ("anthropic-beta", "tools")];
    assert_eq!(post(&format!("{am}/v1/messages"), &auth).0, 200);

    // Second gateway base: address with `/v1`, Anthropic without one.
    let fx2 = fixture();
    let p2 = gateway_provider(&fx2, &up.url("/v1"), &up.url(""), &[]);
    let g2 = Gateway::new(fx2.store.clone());
    let token2 = g2.register(&chat());
    let oc2 = g2.base_url(&format!("thinkrail-{}-oc", short(&p2)), Protocol::OpenaiChat).unwrap();
    let am2 = g2.base_url(&format!("thinkrail-{}-am", short(&p2)), Protocol::AnthropicMessages).unwrap();
    assert_eq!(post(&format!("{oc2}/models"), &[("x-api-key", token2.expose())]).0, 200);
    assert_eq!(post(&format!("{am2}/v1/messages/count_tokens"), &[("x-api-key", token2.expose())]).0, 200);

    let seen = up.seen();
    let paths: Vec<_> = seen.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(paths, ["/relay/v1/chat/completions?stream=true", "/claude/v1/messages", "/v1/models", "/v1/messages/count_tokens"]);

    // OpenAI: real key as bearer, no x-api-key; provider header added.
    assert_eq!(header(&seen[0], "authorization"), Some("Bearer sk-test-a"));
    assert_eq!(header(&seen[0], "x-api-key"), None);
    assert_eq!(header(&seen[0], "x-trace"), Some(HEADER_VALUE));
    // Anthropic: both auth headers carry the real key; engine headers preserved.
    assert_eq!(header(&seen[1], "x-api-key"), Some("sk-test-a"));
    assert_eq!(header(&seen[1], "authorization"), Some("Bearer sk-test-a"));
    assert_eq!(header(&seen[1], "anthropic-version"), Some("2023-06-01"));
    assert_eq!(header(&seen[1], "anthropic-beta"), Some("tools"));
    let leaked = |s: &Seen| s.headers.values().any(|v| v.to_str().is_ok_and(|v| v.contains(token.expose())));
    assert!(!seen.iter().any(leaked));
    // No keys: no auth header at all, and the token never leaves the host.
    assert_eq!(header(&seen[2], "x-api-key"), None);
    assert_eq!(header(&seen[2], "authorization"), None);
}


#[test]
fn gateway_rotates_keys_per_request() {
    let fx = fixture();
    let up = upstream(Reply::Json(200, "{}"));
    let provider = gateway_provider(&fx, &up.url(""), &up.url(""), &KEYS[..2]);
    let gateway = Gateway::new(fx.store.clone());
    let url = format!("{}/chat/completions", gateway.base_url(&format!("thinkrail-{}-oc", short(&provider)), Protocol::OpenaiChat).unwrap());
    let token = gateway.register(&chat());
    for _ in 0..3 {
        assert_eq!(post(&url, &[("x-api-key", token.expose())]).0, 200);
    }
    let keys: Vec<_> = up.seen().iter().map(|s| header(s, "authorization").unwrap_or_default().to_owned()).collect();
    assert_eq!(keys, ["Bearer sk-test-a", "Bearer sk-test-b", "Bearer sk-test-a"]);
}

#[test]
fn gateway_streams_chunks_and_reports_unreachable_upstreams() {
    let fx = fixture();
    let up = upstream(Reply::Chunks(vec!["data: one\n\n", "data: two\n\n", "data: three\n\n"]));
    let provider = gateway_provider(&fx, &up.url(""), &closed_port(), &KEYS[..1]);
    let gateway = Gateway::new(fx.store.clone());
    let short = short(&provider);
    let token = gateway.register(&chat());
    let oc = gateway.base_url(&format!("thinkrail-{short}-oc"), Protocol::OpenaiChat).unwrap();
    let (status, chunks) = post(&format!("{oc}/chat/completions"), &[("x-api-key", token.expose())]);
    assert_eq!(status, 200);
    assert!(chunks.len() > 1, "expected streamed chunks, got {}", chunks.len());
    assert_eq!(joined(&chunks), "data: one\n\ndata: two\n\ndata: three\n\n");

    let am = gateway.base_url(&format!("thinkrail-{short}-am"), Protocol::AnthropicMessages).unwrap();
    let (status, body) = post(&format!("{am}/v1/messages"), &[("x-api-key", token.expose())]);
    assert_eq!(status, 502);
    let body = joined(&body);
    assert!(body.contains("上游不可达"), "{body}");
    assert!(!body.contains("sk-test") && !body.contains(token.expose()));
}

// ---------- T5: probe ----------

fn target(protocol: Protocol, base_url: String, key: Option<&str>) -> ProbeTarget {
    ProbeTarget { endpoint: Endpoint { protocol, base_url }, headers: vec![], key: key.map(Secret::new) }
}

fn probe(fx: &Fixture) -> Probe {
    Probe::new(fx.store.clone())
}

#[test]
fn probe_parses_openai_and_anthropic_model_lists() {
    let fx = fixture();
    let openai = upstream(Reply::Json(200, r#"{"object":"list","data":[{"id":"gpt-4o","context_length":128000},{"id":"o3"}]}"#));
    let models = probe(&fx).list_models(&target(Protocol::OpenaiChat, openai.url("/v1"), Some(KEYS[0]))).unwrap();
    assert_eq!(models.iter().map(|m| (m.id.as_str(), m.context_window)).collect::<Vec<_>>(), [("gpt-4o", Some(128_000)), ("o3", None)]);
    let seen = openai.seen();
    assert_eq!(seen[0].path, "/v1/models");
    assert_eq!(header(&seen[0], "authorization"), Some("Bearer sk-test-a"));
    assert_eq!(header(&seen[0], "anthropic-version"), None);

    let anthropic = upstream(Reply::Json(200, r#"{"data":[{"id":"claude-x","display_name":"Claude X","type":"model"}],"has_more":false}"#));
    let models = probe(&fx).list_models(&target(Protocol::AnthropicMessages, anthropic.url("/v1"), Some(KEYS[1]))).unwrap();
    assert_eq!((models[0].id.as_str(), models[0].name.as_deref()), ("claude-x", Some("Claude X")));
    let seen = anthropic.seen();
    assert_eq!(seen[0].path, "/v1/models");
    assert_eq!(header(&seen[0], "x-api-key"), Some("sk-test-b"));
    assert_eq!(header(&seen[0], "authorization"), Some("Bearer sk-test-b"));
    assert_eq!(header(&seen[0], "anthropic-version"), Some("2023-06-01"));
}

#[test]
fn probe_classifies_failures() {
    let fx = fixture();
    let rejected = upstream(Reply::Json(401, r#"{"error":{"message":"bad key sk-test-a"}}"#));
    let message = kind(probe(&fx).list_models(&target(Protocol::OpenaiChat, rejected.url(""), Some(KEYS[0]))), ErrorKind::External);
    assert!(message.contains("服务器拒绝了该 API 密钥。") && !message.contains("sk-test"), "{message}");

    let failing = upstream(Reply::Json(500, r#"{"error":{"message":"boom for sk-test-a"}}"#));
    let message = kind(probe(&fx).check(&target(Protocol::OpenaiChat, failing.url(""), Some(KEYS[0])), "m"), ErrorKind::External);
    assert!(message.contains("服务器返回 HTTP 500。") && message.contains("boom") && !message.contains("sk-test"), "{message}");

    let echoed = upstream(Reply::Json(500, r#"{"error":{"message":"echo sensitive-deployment-header"}}"#));
    let mut with_header = target(Protocol::OpenaiChat, echoed.url(""), Some(KEYS[0]));
    with_header.headers.push(Header { name: "x-private".into(), value: "sensitive-deployment-header".into() });
    let message = kind(probe(&fx).check(&with_header, "m"), ErrorKind::External);
    assert!(!message.contains("sensitive-deployment-header"));

    let html = upstream(Reply::Json(200, "<html>nope</html>"));
    let message = kind(probe(&fx).list_models(&target(Protocol::OpenaiChat, html.url(""), None)), ErrorKind::Protocol);
    assert!(message.contains("这不是兼容 OpenAI 或 Anthropic 的模型列表响应。"), "{message}");

    let empty = upstream(Reply::Json(200, r#"{"data":[]}"#));
    let message = kind(probe(&fx).list_models(&target(Protocol::OpenaiChat, empty.url(""), None)), ErrorKind::External);
    assert!(message.contains("服务器未返回任何模型。"), "{message}");

    let slow = upstream(Reply::Delay(Duration::from_secs(2)));
    let quick = Probe::with_timeout(fx.store.clone(), Duration::from_millis(200));
    let message = kind(quick.list_models(&target(Protocol::OpenaiChat, slow.url(""), None)), ErrorKind::Timeout);
    assert!(message.contains("服务器未及时响应。"), "{message}");

    let message = kind(probe(&fx).list_models(&target(Protocol::OpenaiChat, closed_port(), None)), ErrorKind::External);
    assert!(message.contains("无法连接到服务器。"), "{message}");
    kind(probe(&fx).list_models(&target(Protocol::OpenaiChat, "not a url".into(), None)), ErrorKind::Invalid);
}

#[test]
fn probe_check_sends_minimal_requests() {
    let fx = fixture();
    let up = upstream(Reply::Json(200, "{}"));
    let probe = probe(&fx);
    for protocol in [Protocol::OpenaiChat, Protocol::OpenaiResponses, Protocol::AnthropicMessages] {
        probe.check(&target(protocol, up.url(""), Some(KEYS[0])), "m").unwrap();
    }
    let paths: Vec<_> = up.seen().iter().map(|s| s.path.clone()).collect();
    assert_eq!(paths, ["/v1/chat/completions", "/v1/responses", "/v1/messages"]);
}

#[test]
fn check_keys_lists_models_once_per_enabled_key_without_rotating() {
    let fx = fixture();
    let up = upstream(Reply::Json(200, r#"{"data":[{"id":"claude-a"},{"id":"claude-b"}]}"#));
    let mut relay = input("Relay", Access::Direct);
    relay.endpoints = endpoints(&up.url(""), &up.url(""));
    let provider = create(&fx, relay, &KEYS);
    let provider = set_key_enabled(&fx, &provider, 1, false);

    let checks = probe(&fx).check_keys(&provider.id, Protocol::AnthropicMessages).unwrap();
    let summary: Vec<_> = checks.iter().map(|c| (c.key.clone(), c.models)).collect();
    assert_eq!(summary, [(provider.keys[0].id.clone(), Some(2)), (provider.keys[2].id.clone(), Some(2))], "disabled keys are skipped");
    let seen = up.seen();
    assert_eq!(seen.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["/v1/models", "/v1/models"]);
    assert_eq!(seen.iter().map(|s| header(s, "x-api-key").unwrap_or_default()).collect::<Vec<_>>(), [KEYS[0], KEYS[2]]);
    assert_eq!(header(&seen[0], "x-trace"), Some(HEADER_VALUE));
    // Rotation untouched: the next launch still starts with the first key.
    assert_eq!(fx.store.next_key(&provider.id).unwrap().unwrap().value.expose(), KEYS[0]);
}

#[test]
fn check_keys_reports_a_rejected_key_without_its_value() {
    let fx = fixture();
    let up = upstream(Reply::Json(401, r#"{"error":{"message":"bad key"}}"#));
    let mut relay = input("Relay", Access::Direct);
    relay.endpoints = endpoints(&up.url(""), &up.url(""));
    let provider = create(&fx, relay, &KEYS[..1]);
    let checks = probe(&fx).check_keys(&provider.id, Protocol::OpenaiChat).unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].models, None);
    let error = checks[0].error.as_deref().unwrap();
    assert!(!error.contains(KEYS[0]), "{error}");
    let missing = probe(&fx).check_keys(&provider.id, Protocol::OpenaiResponses);
    assert_eq!(missing.unwrap_err().kind(), Some(ErrorKind::Invalid), "a line the provider lacks is refused");
}

#[test]
fn gateway_refuses_requests_after_the_last_key_is_disabled() {
    let fx = fixture();
    let up = upstream(Reply::Json(200, "{}"));
    let provider = gateway_provider(&fx, &up.url(""), &up.url(""), &KEYS[..1]);
    let gateway = Gateway::new(fx.store.clone());
    let url = format!("{}/chat/completions", gateway.base_url(&format!("thinkrail-{}-oc", short(&provider)), Protocol::OpenaiChat).unwrap());
    let token = gateway.register(&chat());
    set_key_enabled(&fx, &provider, 0, false);
    assert_eq!(post(&url, &[("x-api-key", token.expose())]).0, 502);
    assert!(up.seen().is_empty());
}
