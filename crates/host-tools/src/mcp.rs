//! The `mcp-host` subcommand (04 §6.2): a stdio MCP server whose tools are the host's, reached over
//! the authenticated pipe. stdout carries only protocol frames.
//!
//! `tools/call` runs on its own thread so the loop keeps reading stdin: a call may block for a
//! whole plan-step review, and the engine's `notifications/cancelled` for it must still get
//! through (agent-settings 08 §5.5). A cancelled request gets no reply.

use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, Write},
    sync::{Arc, Mutex, PoisonError},
    thread,
};

use serde_json::{Value, json};
use thinkrail_core::{HostToolResult, HostToolSpec};
use thinkrail_observability::TraceContext;

use crate::bridge::{self, BridgeClient};

const SUPPORTED: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL: i64 = -32603;
const PARSE_ERROR: i64 = -32700;

/// Where tool listing and calls are served (the pipe in production, a fake in tests). `call` is
/// made from worker threads, several at a time.
pub(crate) trait Backend: Send + Sync {
    fn list(&self) -> Result<Vec<HostToolSpec>, String>;
    fn call(&self, name: &str, arguments: Value) -> Result<HostToolResult, String>;
    /// The engine withdrew a running call.
    fn cancel(&self);
}

/// The pipe: listing and cancelling share one connection; each call gets its own, because the
/// host answers one connection's requests in order.
pub(crate) struct Pipe {
    pipe: String,
    token: String,
    main: Mutex<BridgeClient>,
}

impl Pipe {
    fn request(client: &mut BridgeClient, request: Value) -> Result<Value, String> {
        client.request(request).map_err(|e| e.to_string())
    }
}

impl Backend for Pipe {
    fn list(&self) -> Result<Vec<HostToolSpec>, String> {
        let mut main = self.main.lock().unwrap_or_else(PoisonError::into_inner);
        result_of(Self::request(&mut main, json!({"method": "list"}))?)
    }

    fn call(&self, name: &str, arguments: Value) -> Result<HostToolResult, String> {
        let mut client = BridgeClient::connect(&self.pipe, &self.token)
            .map_err(|e| format!("无法连接宿主工具管道：{e}"))?;
        result_of(Self::request(
            &mut client,
            json!({"method": "call", "name": name, "arguments": arguments}),
        )?)
    }

    fn cancel(&self) {
        let mut main = self.main.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(error) = Self::request(&mut main, json!({"method": "cancel"})) {
            tracing::debug!(%error, "host tool cancel not delivered");
        }
    }
}

fn result_of<T: serde::de::DeserializeOwned>(reply: Value) -> Result<T, String> {
    if let Some(error) = reply.get("error") {
        return Err(error["message"]
            .as_str()
            .unwrap_or("host tool failed")
            .to_owned());
    }
    serde_json::from_value(reply["result"].clone()).map_err(|e| e.to_string())
}

fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn call_reply(id: &Value, outcome: Result<HostToolResult, String>) -> Value {
    match outcome {
        Ok(result) => {
            let mut body = json!({"content": [{"type": "text", "text": result.text}]});
            if result.is_error {
                body["isError"] = json!(true);
            }
            if let Some(structured) = result.structured {
                body["structuredContent"] = structured;
            }
            reply(id, body)
        }
        Err(message) => failure(id, INTERNAL, &message),
    }
}

/// What one frame asks for.
enum Frame {
    Reply(Value),
    /// `tools/call`: run off the loop.
    Call {
        id: Value,
        name: String,
        arguments: Value,
    },
    Cancelled(Value),
    Ignore,
}

fn parse(line: &str, backend: &dyn Backend) -> Frame {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Frame::Reply(failure(&Value::Null, PARSE_ERROR, "invalid JSON"));
    };
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    let Some(id) = message.get("id").cloned() else {
        return match method {
            "notifications/cancelled" => Frame::Cancelled(params["requestId"].clone()),
            _ => Frame::Ignore,
        };
    };
    Frame::Reply(match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = SUPPORTED
                .iter()
                .find(|v| **v == asked)
                .copied()
                .unwrap_or(SUPPORTED[0]);
            reply(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "thinkrail", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
        }
        "ping" => reply(&id, json!({})),
        "tools/list" => match backend.list() {
            Ok(specs) => {
                let tools: Vec<Value> = specs
                    .into_iter()
                    .map(|s| json!({"name": s.name, "description": s.description, "inputSchema": s.input_schema}))
                    .collect();
                reply(&id, json!({"tools": tools}))
            }
            Err(message) => failure(&id, INTERNAL, &message),
        },
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return Frame::Reply(failure(&id, INVALID_PARAMS, "missing tool name"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            return Frame::Call {
                id,
                name: name.to_owned(),
                arguments,
            };
        }
        _ => failure(
            &id,
            METHOD_NOT_FOUND,
            &format!("method not found: {method}"),
        ),
    })
}

/// Serves MCP frames from `input` to `output` until EOF, then waits for running calls.
pub(crate) fn serve<W: Write + Send + 'static>(
    input: &mut impl BufRead,
    output: W,
    backend: Arc<dyn Backend>,
) -> std::io::Result<()> {
    let (trace, session_span) = match TraceContext::root("mcp.session") {
        Ok((trace, span)) => (Some(trace), span),
        Err(_) => {
            tracing::warn!("mcp.trace.unavailable");
            (None, tracing::Span::current())
        }
    };
    let _session = session_span.enter();
    let output = Arc::new(Mutex::new(output));
    let running: Arc<Mutex<HashMap<String, tracing::Span>>> = Arc::default();
    let cancelled: Arc<Mutex<HashSet<String>>> = Arc::default();
    let mut workers = Vec::new();
    let write = |output: &Mutex<W>, value: &Value| -> std::io::Result<()> {
        let mut out = output.lock().unwrap_or_else(PoisonError::into_inner);
        bridge::write_line(&mut *out, value)
    };
    while let Some(line) = bridge::read_line(input)? {
        if line.trim().is_empty() {
            continue;
        }
        match parse(&line, backend.as_ref()) {
            Frame::Reply(value) => write(&output, &value)?,
            Frame::Ignore => {}
            Frame::Cancelled(id) => {
                let key = id.to_string();
                let call_span = running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&key)
                    .cloned();
                if let Some(span) = call_span {
                    let _entered = span.enter();
                    tracing::debug!("mcp.call.cancel_requested");
                    cancelled
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(key);
                    backend.cancel();
                }
            }
            Frame::Call {
                id,
                name,
                arguments,
            } => {
                let key = id.to_string();
                let span = tracing::info_span!(parent: &session_span, "mcp.call",
                    trace_id = trace.as_ref().map(TraceContext::trace_id),
                    operation = "mcp.call", component = "host_tools");
                running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(key.clone(), span.clone());
                let trace = trace.clone();
                let dispatch = tracing::dispatcher::get_default(Clone::clone);
                let (backend, output, running, cancelled) = (
                    backend.clone(),
                    output.clone(),
                    running.clone(),
                    cancelled.clone(),
                );
                let worker =
                    thread::Builder::new()
                        .name("mcp-host-call".into())
                        .spawn(move || {
                            let _dispatch = tracing::dispatcher::set_default(&dispatch);
                            let _entered = span.enter();
                            let child = trace.as_ref().map(|trace| trace.child("mcp.worker"));
                            let _worker = child.as_ref().map(tracing::Span::enter);
                            let response = call_reply(&id, backend.call(&name, arguments));
                            running
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .remove(&key);
                            let withdrawn = cancelled
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .remove(&key);
                            tracing::debug!(cancelled = withdrawn, "mcp.call.returned");
                            if !withdrawn {
                                let mut out = output.lock().unwrap_or_else(PoisonError::into_inner);
                                if let Err(error) = bridge::write_line(&mut *out, &response) {
                                    tracing::debug!(%error, "MCP reply not written");
                                }
                            }
                        })?;
                workers.retain(|worker: &thread::JoinHandle<()>| !worker.is_finished());
                workers.push(worker);
            }
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    Ok(())
}

/// Body of `thinkrail-native.exe mcp-host`: connect with `THINKRAIL_HOST_PIPE` /
/// `THINKRAIL_HOST_TOKEN`, then serve MCP on stdio.
pub fn run_mcp_stdio() -> Result<(), String> {
    let pipe =
        std::env::var("THINKRAIL_HOST_PIPE").map_err(|_| "缺少 THINKRAIL_HOST_PIPE".to_owned())?;
    let token = std::env::var("THINKRAIL_HOST_TOKEN")
        .map_err(|_| "缺少 THINKRAIL_HOST_TOKEN".to_owned())?;
    let main =
        BridgeClient::connect(&pipe, &token).map_err(|e| format!("无法连接宿主工具管道：{e}"))?;
    let backend = Pipe {
        pipe,
        token,
        main: Mutex::new(main),
    };
    // Fail fast on a refused token instead of on the first tool call.
    backend
        .list()
        .map_err(|e| format!("宿主拒绝了工具连接：{e}"))?;
    let stdin = std::io::stdin();
    serve(&mut stdin.lock(), std::io::stdout(), Arc::new(backend))
        .map_err(|e| format!("MCP 连接中断：{e}"))
}
