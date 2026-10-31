//! The `mcp-host` subcommand (04 §6.2): a stdio MCP server whose tools are the host's, reached over
//! the authenticated pipe. stdout carries only protocol frames.

use std::io::{BufRead, Write};

use serde_json::{Value, json};
use thinkrail_core::{HostToolResult, HostToolSpec};

use crate::bridge::{self, BridgeClient};

const SUPPORTED: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL: i64 = -32603;
const PARSE_ERROR: i64 = -32700;

/// Where tool listing and calls are served (the pipe in production, a fake in tests).
pub(crate) trait Backend {
    fn list(&mut self) -> Result<Vec<HostToolSpec>, String>;
    fn call(&mut self, name: &str, arguments: Value) -> Result<HostToolResult, String>;
}

impl Backend for BridgeClient {
    fn list(&mut self) -> Result<Vec<HostToolSpec>, String> {
        let reply = self.request(json!({"method": "list"})).map_err(|e| e.to_string())?;
        result_of(reply)
    }

    fn call(&mut self, name: &str, arguments: Value) -> Result<HostToolResult, String> {
        let reply = self.request(json!({"method": "call", "name": name, "arguments": arguments})).map_err(|e| e.to_string())?;
        result_of(reply)
    }
}

fn result_of<T: serde::de::DeserializeOwned>(reply: Value) -> Result<T, String> {
    if let Some(error) = reply.get("error") {
        return Err(error["message"].as_str().unwrap_or("host tool failed").to_owned());
    }
    serde_json::from_value(reply["result"].clone()).map_err(|e| e.to_string())
}

fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One MCP frame → its reply (`None` for notifications).
pub(crate) fn handle(line: &str, backend: &mut dyn Backend) -> Option<Value> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Some(failure(&Value::Null, PARSE_ERROR, "invalid JSON"));
    };
    let id = message.get("id").cloned()?;
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    Some(match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = SUPPORTED.iter().find(|v| **v == asked).copied().unwrap_or(SUPPORTED[0]);
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
                return Some(failure(&id, INVALID_PARAMS, "missing tool name"));
            };
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match backend.call(name, arguments) {
                Ok(result) => {
                    let mut body = json!({"content": [{"type": "text", "text": result.text}]});
                    if result.is_error {
                        body["isError"] = json!(true);
                    }
                    if let Some(structured) = result.structured {
                        body["structuredContent"] = structured;
                    }
                    reply(&id, body)
                }
                Err(message) => failure(&id, INTERNAL, &message),
            }
        }
        _ => failure(&id, METHOD_NOT_FOUND, &format!("method not found: {method}")),
    })
}

/// Serves MCP frames from `input` to `output` until EOF.
pub(crate) fn serve(input: &mut impl BufRead, output: &mut impl Write, backend: &mut dyn Backend) -> std::io::Result<()> {
    while let Some(line) = bridge::read_line(input)? {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle(&line, backend) {
            bridge::write_line(output, &response)?;
        }
    }
    Ok(())
}

/// Body of `thinkrail-native.exe mcp-host`: connect with `THINKRAIL_HOST_PIPE` /
/// `THINKRAIL_HOST_TOKEN`, then serve MCP on stdio.
pub fn run_mcp_stdio() -> Result<(), String> {
    let pipe = std::env::var("THINKRAIL_HOST_PIPE").map_err(|_| "缺少 THINKRAIL_HOST_PIPE".to_owned())?;
    let token = std::env::var("THINKRAIL_HOST_TOKEN").map_err(|_| "缺少 THINKRAIL_HOST_TOKEN".to_owned())?;
    let mut client = BridgeClient::connect(&pipe, &token).map_err(|e| format!("无法连接宿主工具管道：{e}"))?;
    // Fail fast on a refused token instead of on the first tool call.
    client.list().map_err(|e| format!("宿主拒绝了工具连接：{e}"))?;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(&mut stdin.lock(), &mut stdout.lock(), &mut client).map_err(|e| format!("MCP 连接中断：{e}"))
}
