//! Host side of the MCP bridge (04 §6.2): a local named pipe that `mcp-host` processes connect to.
//! The first line of a connection is its token; the token alone decides the chat scope, so an
//! engine switching sessions can never send a tool call into another chat.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
use parking_lot::Mutex;
use serde_json::{Value, json};
use thinkrail_core::{
    Error, ErrorCode, ErrorKind, HostToolHandler, HostToolOptions, PlanScope, Result,
};
use thinkrail_observability::TraceContext;
use tracing::{debug, info, warn};

/// Largest request or response line.
pub(crate) const MAX_LINE: usize = 16 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 64;

/// Reads one `\n`-terminated line of at most `MAX_LINE` bytes; `None` at EOF.
pub(crate) fn read_line(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    let read = reader
        .by_ref()
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > MAX_LINE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "line exceeds the bridge limit",
        ));
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "line is not UTF-8"))
}

pub(crate) fn write_line(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()
}

fn bridge_error(what: &str, error: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::External, format!("{what}：{error}"))
}

/// token → the chat it reaches and what that engine launch was offered.
type Tokens = Arc<Mutex<HashMap<String, BridgeSession>>>;

#[derive(Clone)]
struct BridgeSession {
    scope: PlanScope,
    options: HostToolOptions,
    trace: Option<TraceContext>,
    span: tracing::Span,
    dispatch: tracing::Dispatch,
}

impl BridgeSession {
    fn span(&self, operation: &'static str) -> tracing::Span {
        let _entered = self.span.enter();
        match &self.trace {
            Some(trace) => {
                let child = trace.child(operation);
                if child.is_disabled() {
                    self.span.clone()
                } else {
                    child
                }
            }
            None => self.span.clone(),
        }
    }
}

/// The running pipe server. Dropping it does not stop the accept thread; it lives for the process.
pub struct ToolBridge {
    pipe: String,
    tokens: Tokens,
}

impl std::fmt::Debug for ToolBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolBridge")
            .field("pipe", &self.pipe)
            .finish_non_exhaustive()
    }
}

impl ToolBridge {
    pub fn start(handler: Arc<dyn HostToolHandler>) -> Result<Self> {
        let pipe = format!("thinkrail-native-{}", uuid::Uuid::new_v4().simple());
        let name = pipe
            .clone()
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| bridge_error("宿主工具管道名无效", e))?;
        let listener = ListenerOptions::new()
            .name(name)
            .create_sync()
            .map_err(|e| bridge_error("无法创建宿主工具管道", e))?;
        let tokens: Tokens = Arc::new(Mutex::new(HashMap::new()));
        let accept_tokens = tokens.clone();
        let live = Arc::new(AtomicUsize::new(0));
        thread::Builder::new()
            .name("host-tools-accept".into())
            .spawn(move || {
                for connection in listener.incoming() {
                    let stream = match connection {
                        Ok(stream) => stream,
                        Err(error) => {
                            warn!(%error, "host tool connection failed");
                            continue;
                        }
                    };
                    if live.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                        live.fetch_sub(1, Ordering::AcqRel);
                        warn!("host tool connection refused: too many connections");
                        continue;
                    }
                    let (tokens, handler, live) =
                        (accept_tokens.clone(), handler.clone(), live.clone());
                    let spawned =
                        thread::Builder::new()
                            .name("host-tools-conn".into())
                            .spawn(move || {
                                serve(stream, &tokens, handler.as_ref());
                                live.fetch_sub(1, Ordering::AcqRel);
                            });
                    if let Err(error) = spawned {
                        warn!(%error, "cannot start host tool connection thread");
                    }
                }
            })
            .map_err(|e| bridge_error("无法启动宿主工具线程", e))?;
        info!("host tool bridge started");
        Ok(Self { pipe, tokens })
    }

    /// The pipe name `mcp-host` connects to (`THINKRAIL_HOST_PIPE`).
    pub fn pipe_name(&self) -> &str {
        &self.pipe
    }

    /// A fresh secret for one engine launch of a chat (`THINKRAIL_HOST_TOKEN`).
    pub fn register(&self, scope: PlanScope, options: HostToolOptions) -> String {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let (trace, root) = match TraceContext::root("host_bridge.session") {
            Ok((trace, span)) => (Some(trace), span),
            Err(_) => {
                warn!("host_bridge.trace.unavailable");
                (None, tracing::Span::current())
            }
        };
        let span = tracing::info_span!(parent: &root, "host_bridge.session",
            trace_id = trace.as_ref().map(TraceContext::trace_id), component = "host_tools",
            chat_id = %scope.chat, workspace_id = %scope.workspace);
        self.tokens.lock().insert(
            token.clone(),
            BridgeSession {
                scope,
                options,
                trace,
                span,
                dispatch: tracing::dispatcher::get_default(Clone::clone),
            },
        );
        token
    }

    pub fn revoke(&self, token: &str) {
        self.tokens.lock().remove(token);
    }
}

fn serve(stream: Stream, tokens: &Tokens, handler: &dyn HostToolHandler) {
    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;
    let entry = match read_line(&mut reader) {
        Ok(Some(token)) => tokens.lock().get(token.trim()).cloned(),
        _ => None,
    };
    let Some(session) = entry else {
        warn!("host tool connection refused: unknown token");
        return;
    };
    let _dispatch = tracing::dispatcher::set_default(&session.dispatch);
    let _entered = session.span.enter();
    let (scope, options) = (&session.scope, session.options);
    debug!(workspace = %scope.workspace, chat = %scope.chat, "host tool client connected");
    loop {
        let line = match read_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                warn!(%error, "host tool connection closed");
                break;
            }
        };
        // A revoked token ends the connection at the next request.
        if !tokens
            .lock()
            .values()
            .any(|session| &session.scope == scope)
        {
            break;
        }
        let span = session.span("host_bridge.request");
        let _request = span.enter();
        let reply = handle(&line, scope, options, handler);
        if let Err(error) = write_line(&mut writer, &reply) {
            debug!(%error, "host tool client went away");
            break;
        }
    }
}

/// One bridge request → reply.
pub(crate) fn handle(
    line: &str,
    scope: &PlanScope,
    options: HostToolOptions,
    handler: &dyn HostToolHandler,
) -> Value {
    let request: Value = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(_) => {
            return json!({"id": null, "error": {"code": "PROTOCOL", "message": "request is not JSON"}});
        }
    };
    let id = request["id"].clone();
    match request["method"].as_str() {
        Some("list") => json!({"id": id, "result": handler.specs(options)}),
        Some("call") => {
            let Some(name) = request["name"].as_str() else {
                return json!({"id": id, "error": {"code": "PROTOCOL", "message": "missing tool name"}});
            };
            match handler.call(scope, options, name, request["arguments"].clone()) {
                Ok(result) => json!({"id": id, "result": result}),
                // The handler logs the failure with its chat and duration.
                Err(error) => {
                    json!({"id": id, "error": {"code": error.code(), "message": error.to_string()}})
                }
            }
        }
        // The engine withdrew a call (MCP `notifications/cancelled`).
        Some("cancel") => {
            handler.cancel(scope);
            json!({"id": id, "result": null})
        }
        _ => json!({"id": id, "error": {"code": "PROTOCOL", "message": "unknown method"}}),
    }
}

/// Client side, used by `mcp-host`: connects and authenticates with `token`.
pub(crate) struct BridgeClient {
    reader: BufReader<Stream>,
    next: u64,
}

impl BridgeClient {
    pub(crate) fn connect(pipe: &str, token: &str) -> std::io::Result<Self> {
        let name = pipe.to_owned().to_ns_name::<GenericNamespaced>()?;
        let stream = Stream::connect(name)?;
        let mut client = Self {
            reader: BufReader::new(stream),
            next: 0,
        };
        client
            .reader
            .get_mut()
            .write_all(format!("{token}\n").as_bytes())?;
        Ok(client)
    }

    pub(crate) fn request(&mut self, mut request: Value) -> std::io::Result<Value> {
        self.next += 1;
        request["id"] = json!(self.next);
        write_line(self.reader.get_mut(), &request)?;
        let line = read_line(&mut self.reader)?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "host closed the tool bridge",
            )
        })?;
        serde_json::from_str(&line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}
