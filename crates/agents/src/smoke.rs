//! Real engine handshakes (`#[ignore]`): official CLIs, isolated config dirs, no credentials and no
//! model requests. Point them at installed engines:
//!
//! - `THINKRAIL_SMOKE_PI` = `<node.exe>|<pi dist/bundle/cli.js>`
//! - `THINKRAIL_SMOKE_OMP` = `<omp.exe>` or `<bun.exe>|<omp src/cli.ts>`
//! - `THINKRAIL_SMOKE_DSH` = `<node.exe ≥ 24.9>|<dsh lib/bin.js>`

use std::{fs, path::PathBuf, sync::Arc};

use tempfile::TempDir;
use thinkrail_core::{
    AgentRuntime, BackendKind, DefaultModel, EngineRouting, EnvBinding, ErrorKind, Launch,
    ProjectedMcpServer, ProjectedMcpTransport, ProjectedModel, ProjectedProvider, Protocol, Secret,
    SessionPhase, StartSession, ThinkingLevel,
};

use crate::{DshRuntime, OmpRuntime, PiRuntime, projection::tests::Events};

mod commands;
mod model;
mod skills;

fn launch(var: &str) -> Launch {
    let value = std::env::var(var).unwrap_or_else(|_| panic!("set {var} to run this smoke test"));
    let mut parts = value.split('|').map(PathBuf::from);
    let executable = parts.next().unwrap_or_default();
    let args = parts.map(|p| p.to_string_lossy().into_owned()).collect();
    Launch::External { executable, args }
}

/// A new session of `backend` launched from `var`, in the fixture's isolated config and cwd.
fn request(backend: BackendKind, var: &str, f: &Fixture) -> StartSession {
    StartSession {
        chat: "smoke".into(),
        workspace: "w".into(),
        backend,
        launch: launch(var),
        config_root: Some(f.config.clone()),
        cwd: f.cwd.clone(),
        resume: None,
        host_tools: None,
        routing: EngineRouting::default(),
        support_dir: std::env::temp_dir().join("thinkrail-smoke-engines"),
        role: thinkrail_core::SessionRole::Worker,
    }
}

struct Fixture {
    _root: TempDir,
    config: PathBuf,
    cwd: PathBuf,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let cwd = root.path().join("工作区 smoke");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    Fixture {
        _root: root,
        config,
        cwd,
    }
}

/// Start → real locator → idle → dispose → closed; then resume the same native session.
fn round_trip(runtime: &dyn AgentRuntime, request: StartSession, resume: bool) {
    let events = Arc::new(Events::default());
    let session = runtime
        .start(request.clone(), events.clone())
        .unwrap_or_else(|e| panic!("start: {e}"));
    let snapshot = session.snapshot();
    let key = snapshot
        .native_session_key
        .clone()
        .expect("a real native locator");
    assert!(!key.as_str().is_empty());
    assert_eq!(snapshot.phase, SessionPhase::Idle);
    session.dispose().unwrap();
    assert_eq!(session.snapshot().phase, SessionPhase::Closed);
    assert!(
        session.snapshot().last_error.is_none(),
        "graceful dispose is not an error"
    );
    assert_eq!(
        session.prompt("x").unwrap_err().kind(),
        Some(ErrorKind::SessionClosed)
    );
    if resume {
        let request = StartSession {
            resume: Some(key.clone()),
            ..request
        };
        let resumed = runtime
            .start(request, events)
            .unwrap_or_else(|e| panic!("resume: {e}"));
        assert_eq!(resumed.snapshot().native_session_key, Some(key));
        resumed.dispose().unwrap();
    }
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_PI"]
fn pi_rpc_handshake() {
    let f = fixture();
    round_trip(
        &PiRuntime::new(),
        request(BackendKind::Pi, "THINKRAIL_SMOKE_PI", &f),
        false,
    );
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_OMP"]
fn omp_without_a_model_reports_a_readable_start_error() {
    let f = fixture();
    let request = request(BackendKind::Omp, "THINKRAIL_SMOKE_OMP", &f);
    let error = OmpRuntime::new()
        .start(request, Arc::new(Events::default()))
        .err()
        .expect("start must fail");
    assert_eq!(error.kind(), Some(ErrorKind::BackendUnavailable));
    assert!(error.to_string().contains("models.yml"), "{error}");
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_OMP"]
fn omp_rpc_ui_v2_handshake() {
    let f = fixture();
    // A keyless local placeholder model: OMP refuses to start without a default model.
    fs::write(
        f.config.join("models.yml"),
        "providers:\n  smoke:\n    baseUrl: http://127.0.0.1:9/v1\n    api: openai-completions\n    auth: none\n    models:\n      - id: smoke-model\n        name: Smoke\n",
    )
    .unwrap();
    round_trip(
        &OmpRuntime::new(),
        request(BackendKind::Omp, "THINKRAIL_SMOKE_OMP", &f),
        false,
    );
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_DSH"]
fn dsh_acp_handshake_and_resume() {
    let f = fixture();
    round_trip(
        &DshRuntime::new(),
        request(BackendKind::Dsh, "THINKRAIL_SMOKE_DSH", &f),
        true,
    );
}

/// A stdio MCP server that records each run in `<dir>/echo-<pid>.log` (the value of
/// `ECHO_TOKEN` it received, then every JSON-RPC method it was asked for) and offers one `echo` tool.
const ECHO_SERVER: &str = r#"const fs = require("fs"), path = require("path");
const log = path.join(__dirname, `echo-${process.pid}.log`);
fs.appendFileSync(log, `token=${process.env.ECHO_TOKEN}\n`);
let buffer = "";
process.stdin.on("data", (chunk) => {
	buffer += chunk;
	let end;
	while ((end = buffer.indexOf("\n")) >= 0) {
		const line = buffer.slice(0, end).trim();
		buffer = buffer.slice(end + 1);
		if (!line) continue;
		const msg = JSON.parse(line);
		fs.appendFileSync(log, `${msg.method}\n`);
		if (msg.id === undefined) continue;
		const reply = (result) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: msg.id, result }) + "\n");
		if (msg.method === "initialize") reply({ protocolVersion: msg.params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "echo", version: "1" } });
		else if (msg.method === "tools/list") reply({ tools: [{ name: "echo", description: "Echo a marker", inputSchema: { type: "object", properties: {} } }] });
		else if (msg.method === "tools/call") reply({ content: [{ type: "text", text: "marker" }] });
		else process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: msg.id, error: { code: -32601, message: "no" } }) + "\n");
	}
});
"#;

/// Placeholder value of the echo server's `ECHO_TOKEN`; it must reach the server and no file.
const ECHO_TOKEN: &str = "sk-test-echo-$HOME-!x";

/// A placeholder provider on a closed loopback port (no model request is ever sent) with a
/// reasoning model and a plain one, `proj-a` at `low` as the default, and the echo MCP server
/// (run by the `node` on `PATH`).
fn placeholder_routing(dir: &std::path::Path) -> EngineRouting {
    fs::write(dir.join("echo.js"), ECHO_SERVER).unwrap();
    let model = |id: &str, reasoning: bool| ProjectedModel {
        id: id.into(),
        name: id.into(),
        vision: true,
        reasoning,
        efforts: if reasoning {
            vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::High]
        } else {
            Vec::new()
        },
        context_window: 128_000,
        max_tokens: 16_384,
    };
    EngineRouting {
        providers: vec![ProjectedProvider {
            route: "thinkrail-00000000-oc".into(),
            display_name: "Placeholder".into(),
            protocol: Protocol::OpenaiChat,
            base_url: "http://127.0.0.1:9/v1".into(),
            key_var: "THINKRAIL_PROVIDER_00000000_KEY".into(),
            headers: vec![EnvBinding {
                name: "x-extra".into(),
                var: "THINKRAIL_HDR_00000000_0".into(),
            }],
            models: vec![model("proj-a", true), model("proj-b", false)],
        }],
        default_model: Some(DefaultModel {
            route: "thinkrail-00000000-oc".into(),
            model_id: "proj-a".into(),
            effort: Some(ThinkingLevel::Low),
        }),
        mcp: vec![ProjectedMcpServer {
            name: "thinkrail-echo".into(),
            transport: ProjectedMcpTransport::Stdio {
                command: "node".into(),
                args: vec![dir.join("echo.js").display().to_string()],
                env: vec![EnvBinding {
                    name: "ECHO_TOKEN".into(),
                    var: "THINKRAIL_MCP_00000000_E0".into(),
                }],
            },
        }],
        env: vec![
            (
                "THINKRAIL_PROVIDER_00000000_KEY".into(),
                Secret::new("sk-test-a"),
            ),
            ("THINKRAIL_HDR_00000000_0".into(), Secret::new("sk-test-b")),
            ("THINKRAIL_MCP_00000000_E0".into(), Secret::new(ECHO_TOKEN)),
        ],
        signature: "smoke".into(),
    }
}

/// Waits up to 20 s for an echo server run that received the token and was asked for its tools.
fn wait_for_echo(dir: &std::path::Path) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let logs: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("echo-"))
            })
            .map(|p| fs::read_to_string(p).unwrap_or_default())
            .collect();
        if let Some(log) = logs.iter().find(|log| log.contains("tools/list")) {
            return log.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the echo MCP server was never listed: {logs:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// Fails if any file under `dir` (recursively) contains a placeholder secret.
fn assert_no_secret_in(dir: &std::path::Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_no_secret_in(&path);
        } else if !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("echo-"))
        {
            let text = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
            for secret in ["sk-test-a", "sk-test-b", ECHO_TOKEN] {
                assert!(
                    !text.contains(secret),
                    "{} contains a placeholder secret",
                    path.display()
                );
            }
        }
    }
}

/// Starts a new session with only the projection (the config dir has no model of its own): the
/// default model and level come from it, the other projected model is selectable, and the echo
/// MCP server is started with the token and asked for its tools.
fn projection_round(runtime: &dyn AgentRuntime, request: StartSession, f: &Fixture) {
    let support = f.config.parent().unwrap().join("support");
    let routing = placeholder_routing(f.config.parent().unwrap());
    let request = StartSession {
        routing,
        support_dir: support.clone(),
        ..request
    };
    let session = runtime
        .start(request, Arc::new(Events::default()))
        .unwrap_or_else(|e| panic!("start: {e}"));
    let model = session
        .snapshot()
        .model
        .expect("the projected default model");
    assert_eq!(
        (model.provider.as_deref(), model.id.as_str(), model.effort),
        (
            Some("thinkrail-00000000-oc"),
            "proj-a",
            Some(ThinkingLevel::Low)
        )
    );
    session
        .set_model("thinkrail-00000000-oc", "proj-b", None)
        .unwrap_or_else(|e| panic!("set_model proj-b: {e}"));
    assert_eq!(
        session.snapshot().model.map(|m| m.id),
        Some("proj-b".to_owned())
    );
    let log = wait_for_echo(f.config.parent().unwrap());
    assert!(
        log.starts_with(&format!("token={ECHO_TOKEN}\n")),
        "the MCP server got its value verbatim: {log}"
    );
    session.dispose().unwrap();
    assert_no_secret_in(&support);
    assert_no_secret_in(&f.config);
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_PI"]
fn pi_loads_the_projected_provider_default_model_and_mcp_server() {
    let f = fixture();
    projection_round(
        &PiRuntime::new(),
        request(BackendKind::Pi, "THINKRAIL_SMOKE_PI", &f),
        &f,
    );
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_OMP"]
fn omp_loads_the_projected_provider_default_model_and_mcp_server() {
    let f = fixture();
    projection_round(
        &OmpRuntime::new(),
        request(BackendKind::Omp, "THINKRAIL_SMOKE_OMP", &f),
        &f,
    );
}

#[test]
#[ignore = "needs THINKRAIL_SMOKE_DSH"]
fn dsh_loads_the_projected_provider_default_model_and_mcp_server() {
    let f = fixture();
    let support = f.config.parent().unwrap().join("support");
    let mut routing = placeholder_routing(f.config.parent().unwrap());
    for provider in &mut routing.providers {
        provider.headers.clear();
    }
    fs::create_dir_all(f.config.join("profiles/acp")).unwrap();
    fs::write(f.config.join("profiles/acp/cordis.patch.yml"), concat!(
        "- id: llm-pi-ai\n  config:\n    providers:\n      user-owned:\n",
        "        api: openai-completions\n        baseURL: http://127.0.0.1:9/v1\n",
        "        models:\n          - id: user-model\n            name: !!js '\"User\"'\n",
        "            contextWindow: 128000\n            maxTokens: 4096\n            reasoningEfforts: false\n",
    )).unwrap();
    let request = StartSession {
        routing,
        support_dir: support.clone(),
        ..request(BackendKind::Dsh, "THINKRAIL_SMOKE_DSH", &f)
    };
    let session = DshRuntime::new()
        .start(request, Arc::new(Events::default()))
        .unwrap_or_else(|e| panic!("start: {e}"));
    let current = session.snapshot().model.expect("projected model");
    assert_eq!(
        (
            current.provider.as_deref(),
            current.id.as_str(),
            current.effort
        ),
        (
            Some("thinkrail-00000000-oc"),
            "proj-a",
            Some(ThinkingLevel::Low)
        )
    );
    session
        .set_model("thinkrail-00000000-oc", "proj-b", None)
        .unwrap();
    assert_eq!(
        session.snapshot().model.map(|m| m.id),
        Some("proj-b".to_owned())
    );
    session.set_model("user-owned", "user-model", None).unwrap();
    assert_eq!(
        session.snapshot().model.map(|m| m.id),
        Some("user-model".to_owned())
    );
    let log = wait_for_echo(f.config.parent().unwrap());
    assert!(log.starts_with(&format!("token={ECHO_TOKEN}\n")));
    session.dispose().unwrap();
    assert_no_secret_in(&support);
}
